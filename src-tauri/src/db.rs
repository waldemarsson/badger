// src-tauri/src/db.rs
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Pool, Sqlite, SqliteConnection,
};
use std::cmp::Ordering;
use std::path::Path;

/// DDL of the Apple Reminders pairing table, shared by `SCHEMA_SQL` and the tasks migration
/// (which drops and recreates it).
///
/// One Badger task ↔ one EventKit item in the "Badger" list, with the synced fields as each
/// side looked after the last sync (JSON), for the three-way merge. PhoneAlarms is the JSON
/// list of absolute alarm times last pushed (past ones stay, so they aren't re-pushed); the
/// native notification skip checks a firing reminder's time against it. MissedPasses counts
/// consecutive passes the open phone item was missing from its list; a phone delete applies
/// to Badger on the second, so a one-pass glitch deletes nothing. No foreign keys on purpose:
/// a missing task means it was deleted in Badger, and restore's DELETEs must not cascade
/// pairings away. Device-local: not part of backups.
macro_rules! apple_sync_items_ddl {
    () => {
        "CREATE TABLE IF NOT EXISTS apple_sync_items (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    BadgerId     INTEGER NOT NULL,
    EkId         TEXT    NOT NULL,
    ExternalId   TEXT,
    BadgerSnap   TEXT    NOT NULL,
    PhoneSnap    TEXT    NOT NULL,
    PhoneAlarms  TEXT    NOT NULL DEFAULT '[]',
    SyncedAt     TEXT    NOT NULL,
    MissedPasses INTEGER NOT NULL DEFAULT 0,
    UNIQUE (BadgerId),
    UNIQUE (EkId)
);
"
    };
}

/// [`apple_sync_items_ddl!`] as a constant.
pub const APPLE_SYNC_ITEMS_DDL: &str = apple_sync_items_ddl!();

/// Full schema DDL.  Uses PRAGMA lines so in-memory test pools also get FK enforcement.
/// Execute via `sqlx::raw_sql()` — `sqlx::query()` only runs the first statement in a
/// multi-statement string.
pub const SCHEMA_SQL: &str = concat!(
    "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS customers (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    Name          TEXT    NOT NULL,
    UseAsDefault  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS projects (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    CustomerId    INTEGER NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    Name          TEXT    NOT NULL,
    UseAsDefault  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS issues (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ProjectId   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    Title       TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS time_entries (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    IssueId   INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    Date      TEXT    NOT NULL,
    Hours     REAL    NOT NULL,
    Notes     TEXT
);

CREATE TABLE IF NOT EXISTS travel (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    Date      TEXT    NOT NULL,
    FromPlace TEXT    NOT NULL,
    ToPlace   TEXT    NOT NULL,
    Km        REAL,
    Amount    REAL
);

CREATE TABLE IF NOT EXISTS app_settings (
    Key       TEXT    PRIMARY KEY,
    Value     TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    ParentId  INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    Name      TEXT    NOT NULL,
    -- Manual order among siblings; shown as Position, id.
    Position  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS notes (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    FolderId  INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    Title     TEXT    NOT NULL,
    Body      TEXT    NOT NULL DEFAULT '',
    Pinned    INTEGER NOT NULL DEFAULT 0,
    CreatedAt TEXT    NOT NULL,
    UpdatedAt TEXT    NOT NULL,
    -- Manual order within the folder; shown as Pinned DESC, Position, id.
    Position  INTEGER NOT NULL DEFAULT 0,
    -- Editor mode: 'markdown' or 'json'. JSON notes are title-only in the derived indexes.
    Kind      TEXT    NOT NULL DEFAULT 'markdown' CHECK (Kind IN ('markdown', 'json'))
);

-- Tasks: open or done, an optional due date (and time), an optional repeat rule (needs a due
-- date; Mac-only: never synced), optional links to one issue and one note (deleting either
-- unlinks the task). Times are naive local wall-clock strings like everywhere else.
CREATE TABLE IF NOT EXISTS tasks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    Title       TEXT    NOT NULL,
    Notes       TEXT,
    Done        INTEGER NOT NULL DEFAULT 0 CHECK (Done IN (0, 1)),
    CompletedAt TEXT,
    -- YYYY-MM-DD, and HH:MM only together with a date.
    DueDate     TEXT,
    DueTime     TEXT,
    Repeat      TEXT    NOT NULL DEFAULT 'none'
                CHECK (Repeat IN ('none', 'daily', 'weekdays', 'weekly', 'monthly')),
    Weekdays    TEXT    NOT NULL DEFAULT '',
    MonthDay    INTEGER CHECK (MonthDay IS NULL OR MonthDay BETWEEN 1 AND 31),
    IssueId     INTEGER REFERENCES issues(id) ON DELETE SET NULL,
    NoteId      INTEGER REFERENCES notes(id) ON DELETE SET NULL,
    -- Keep off phone: never synced to Apple Reminders.
    MacOnly     INTEGER NOT NULL DEFAULT 0 CHECK (MacOnly IN (0, 1)),
    CreatedAt   TEXT    NOT NULL,
    UpdatedAt   TEXT    NOT NULL,
    CHECK (DueTime IS NULL OR DueDate IS NOT NULL),
    CHECK (Repeat = 'none' OR DueDate IS NOT NULL)
);

-- A task's reminders: relative to its due (OffsetMinutes before it; a date-only due counts
-- from 09:00) or at a custom time (At). State is pending → sent (notified) → dismissed, and
-- applies to the fire time in FiredFor: when edits move that time, the row re-arms (future)
-- or is dismissed silently (past). SnoozedUntil moves only this reminder. Fired/snoozed state
-- is Badger-only (not synced).
CREATE TABLE IF NOT EXISTS task_reminders (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    TaskId        INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    OffsetMinutes INTEGER CHECK (OffsetMinutes IS NULL OR OffsetMinutes >= 0),
    At            TEXT,
    State         TEXT    NOT NULL DEFAULT 'pending'
                  CHECK (State IN ('pending', 'sent', 'dismissed')),
    SnoozedUntil  TEXT,
    FiredFor      TEXT,
    CreatedAt     TEXT    NOT NULL,
    CHECK ((OffsetMinutes IS NULL) != (At IS NULL))
);
-- A new table, so the index never meets an older shape.
CREATE INDEX IF NOT EXISTS idx_task_reminders_task ON task_reminders(TaskId);

-- The scratchpad: a single row (id 1), written on first save. Not a note: it stays out of
-- notes search, the link index and the semantic index.
CREATE TABLE IF NOT EXISTS scratchpad (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    Body      TEXT    NOT NULL DEFAULT '',
    UpdatedAt TEXT    NOT NULL,
    -- Editor mode: 'markdown' or 'json'.
    Kind      TEXT    NOT NULL DEFAULT 'markdown' CHECK (Kind IN ('markdown', 'json'))
);

-- Handovers: one end-of-day note per local date (YYYY-MM-DD, the day it was written),
-- shown on Today the next working day. Not a note: it stays out of notes search, the link
-- index and the semantic index. An emptied handover is deleted, not stored blank.
CREATE TABLE IF NOT EXISTS handovers (
    Date        TEXT    PRIMARY KEY,
    Body        TEXT    NOT NULL,
    DismissedAt TEXT,
    CreatedAt   TEXT    NOT NULL,
    UpdatedAt   TEXT    NOT NULL
);

-- Playground scripts: saved JavaScript snippets. Not notes: they stay out of notes search,
-- the link index and the semantic index. Titles are not unique.
CREATE TABLE IF NOT EXISTS scripts (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    Title     TEXT    NOT NULL,
    Body      TEXT    NOT NULL DEFAULT '',
    CreatedAt TEXT    NOT NULL,
    UpdatedAt TEXT    NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_notes_title ON notes(Title COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_notes_folder ON notes(FolderId);
CREATE INDEX IF NOT EXISTS idx_folders_parent ON folders(ParentId);

-- Derived search index (rowid = notes.id, Body = plain text). Rust keeps it in sync;
-- not part of backups: restore rebuilds it.
CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(Title, Body, tokenize = 'trigram');

-- Derived link index: each note's #tags and [[wikilink]] targets, lowercased. Rust keeps
-- it in sync with bodies; not part of backups: restore rebuilds it.
CREATE TABLE IF NOT EXISTS note_refs (
    NoteId    INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    Kind      TEXT    NOT NULL CHECK (Kind IN ('tag', 'link')),
    Target    TEXT    NOT NULL,
    PRIMARY KEY (NoteId, Kind, Target)
);
CREATE INDEX IF NOT EXISTS idx_note_refs_target ON note_refs(Kind, Target);

-- Derived semantic index: one row per embedded chunk of a note (Vector = L2-normalised
-- little-endian f32). Rust keeps it in sync; not part of backups: restore re-embeds.
CREATE TABLE IF NOT EXISTS note_embeddings (
    NoteId      INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    ChunkIndex  INTEGER NOT NULL,
    ContentHash TEXT    NOT NULL,
    ModelId     TEXT    NOT NULL,
    ChunkText   TEXT    NOT NULL,
    Vector      BLOB    NOT NULL,
    PRIMARY KEY (NoteId, ChunkIndex)
);

",
    apple_sync_items_ddl!()
);

/// Opens (or creates) the SQLite database at `db_path` with WAL mode and FK enforcement,
/// then runs `SCHEMA_SQL` to create all tables idempotently.
pub async fn init_db(db_path: &Path) -> Result<Pool<Sqlite>, sqlx::Error> {
    // Ensure ~/.badger/ directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?; // sqlx::Error::Io via From impl
    }

    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    // raw_sql handles multiple semicolon-separated statements (sqlx 0.8 feature)
    sqlx::raw_sql(SCHEMA_SQL).execute(&pool).await?;
    migrate_to_tasks(&pool).await?;
    migrate_issues_drop_external_id(&pool).await?;
    migrate_notes_position(&pool).await?;
    migrate_notes_kind(&pool).await?;
    migrate_scratchpad_kind(&pool).await?;
    backfill_notes_fts(&pool).await?;
    backfill_note_refs(&pool).await?;

    Ok(pool)
}

/// `app_settings` marker: the one-time switch from todos and reminders to tasks has run.
pub const TASKS_MIGRATED_KEY: &str = "tasks_migrated";

/// Apple Reminders sync settings the tasks migration clears: the two old list ids and the
/// last pass's state. (`apple_sync_enabled` and the last sync time stay.)
const CLEARED_SYNC_KEYS: [&str; 6] = [
    crate::app_settings::LEGACY_APPLE_SYNC_REMINDERS_LIST_KEY,
    crate::app_settings::LEGACY_APPLE_SYNC_TODOS_LIST_KEY,
    crate::app_settings::APPLE_SYNC_PAUSED_KEY,
    crate::app_settings::APPLE_SYNC_LAST_ERROR_KEY,
    crate::app_settings::APPLE_SYNC_SKIPPED_KEY,
    crate::app_settings::APPLE_SYNC_RESTORE_PENDING_KEY,
];

/// The one-time fresh start for tasks (no data migration): drops `reminders` (the FK child)
/// and `todos`, recreates `apple_sync_items` empty in its single-kind shape, clears the old
/// sync settings, and notes that the old phone lists can be deleted when sync had any.
///
/// Guarded by the [`TASKS_MIGRATED_KEY`] marker rather than by the old tables existing, so a
/// downgrade (which recreates them) and a re-upgrade never wipe anything again. Runs right
/// after `SCHEMA_SQL`, which has already created `tasks` and `task_reminders`. Fresh
/// databases run it too and just get the marker.
pub async fn migrate_to_tasks(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let done: Option<String> = sqlx::query_scalar("SELECT Value FROM app_settings WHERE Key = ?")
        .bind(TASKS_MIGRATED_KEY)
        .fetch_optional(pool)
        .await?;
    if done.is_some() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let (old_lists,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM app_settings WHERE Key IN (?, ?) AND TRIM(Value) != ''",
    )
    .bind(crate::app_settings::LEGACY_APPLE_SYNC_REMINDERS_LIST_KEY)
    .bind(crate::app_settings::LEGACY_APPLE_SYNC_TODOS_LIST_KEY)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS reminders;
         DROP TABLE IF EXISTS todos;
         DROP TABLE IF EXISTS apple_sync_items;",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::raw_sql(APPLE_SYNC_ITEMS_DDL)
        .execute(&mut *tx)
        .await?;
    for key in CLEARED_SYNC_KEYS {
        sqlx::query("DELETE FROM app_settings WHERE Key = ?")
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    if old_lists > 0 {
        sqlx::query(
            "INSERT INTO app_settings (Key, Value) VALUES (?, 'true')
             ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
        )
        .bind(crate::app_settings::APPLE_SYNC_OLD_LISTS_NOTE_KEY)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("INSERT INTO app_settings (Key, Value) VALUES (?, '1')")
        .bind(TASKS_MIGRATED_KEY)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Drops the unused `issues.ExternalId` column from databases created before it was removed.
/// Idempotent: does nothing once the column is gone.
pub async fn migrate_issues_drop_external_id(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let columns: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('issues')")
        .fetch_all(pool)
        .await?;
    if !columns.iter().any(|(name,)| name == "ExternalId") {
        return Ok(());
    }

    sqlx::query("ALTER TABLE issues DROP COLUMN ExternalId")
        .execute(pool)
        .await?;
    Ok(())
}

/// Adds `notes.Position` to databases created before manual sorting and backfills notes
/// and folders with the order they used to show in. Idempotent: does nothing once the
/// column exists.
pub async fn migrate_notes_position(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let columns: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('notes')")
        .fetch_all(pool)
        .await?;
    if columns.iter().any(|(name,)| name == "Position") {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::query("ALTER TABLE notes ADD COLUMN Position INTEGER NOT NULL DEFAULT 0")
        .execute(&mut *tx)
        .await?;
    backfill_positions(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Column definition shared by the `Kind` migrations (same as in `SCHEMA_SQL`).
const KIND_COLUMN: &str =
    "Kind TEXT NOT NULL DEFAULT 'markdown' CHECK (Kind IN ('markdown', 'json'))";

/// Adds `Kind` to `table` unless it has it; existing rows read as `markdown`.
async fn add_kind_column(pool: &Pool<Sqlite>, table: &'static str) -> Result<(), sqlx::Error> {
    let columns: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT name FROM pragma_table_info('{table}')"
    )))
    .fetch_all(pool)
    .await?;
    if columns.iter().any(|(name,)| name == "Kind") {
        return Ok(());
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE {table} ADD COLUMN {KIND_COLUMN}"
    )))
    .execute(pool)
    .await?;
    Ok(())
}

/// Adds `notes.Kind` (markdown or JSON editor) to databases created before JSON notes.
/// Idempotent: does nothing once the column exists.
pub async fn migrate_notes_kind(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    add_kind_column(pool, "notes").await
}

/// Adds `scratchpad.Kind` to databases created before JSON notes. Idempotent.
pub async fn migrate_scratchpad_kind(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    add_kind_column(pool, "scratchpad").await
}

/// Numbers notes and folders densely (0..n per folder / parent) in the order the tree
/// used before manual sorting. Used by the migration and by restoring older backups.
pub async fn backfill_positions(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    backfill_note_positions(&mut *conn).await?;
    backfill_folder_positions(conn).await
}

/// Notes per folder: pinned first, then most recently updated, then id.
pub async fn backfill_note_positions(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE notes SET Position = (
             SELECT r.Pos FROM (
                 SELECT id, ROW_NUMBER() OVER (
                     PARTITION BY FolderId ORDER BY Pinned DESC, UpdatedAt DESC, id
                 ) - 1 AS Pos
                 FROM notes
             ) r WHERE r.id = notes.id
         )",
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Folders per parent: A–Z by [`compare_folder_names`], then id.
pub async fn backfill_folder_positions(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut folders: Vec<(i64, Option<i64>, String)> =
        sqlx::query_as("SELECT id, ParentId, Name FROM folders")
            .fetch_all(&mut *conn)
            .await?;
    folders.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| compare_folder_names(&a.2, &b.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    let mut parent: Option<Option<i64>> = None;
    let mut position = 0_i64;
    for (id, parent_id, _) in folders {
        if parent != Some(parent_id) {
            parent = Some(parent_id);
            position = 0;
        }
        sqlx::query("UPDATE folders SET Position = ? WHERE id = ?")
            .bind(position)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        position += 1;
    }
    Ok(())
}

/// A token of a name for [`compare_folder_names`]: a run of digits or a single character.
enum NameToken<'a> {
    Number(&'a str),
    Char(char),
}

fn name_tokens(name: &str) -> Vec<NameToken<'_>> {
    let mut tokens = Vec::new();
    let mut rest = name;
    while let Some(c) = rest.chars().next() {
        if c.is_ascii_digit() {
            let end = rest
                .find(|ch: char| !ch.is_ascii_digit())
                .unwrap_or(rest.len());
            tokens.push(NameToken::Number(&rest[..end]));
            rest = &rest[end..];
        } else {
            tokens.push(NameToken::Char(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    tokens
}

/// Sort key of a non-digit character: (class, weight). Spaces and punctuation (class 0)
/// sort before digits (class 1) and letters (class 2). Letters ignore case and most
/// accents; å, ä, ö (and æ, ø) follow z as in Swedish.
fn char_key(c: char) -> (u8, u32) {
    let lower = c.to_lowercase().next().unwrap_or(c);
    let folded = match lower {
        'à' | 'á' | 'â' | 'ã' | 'ā' => 'a',
        'ç' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' => 'o',
        'ù' | 'ú' | 'û' => 'u',
        'ü' | 'ý' | 'ÿ' => 'y',
        'š' => 's',
        'ž' => 'z',
        'æ' => 'ä',
        'ø' => 'ö',
        other => other,
    };
    match folded {
        'a'..='z' => (2, folded as u32 - 'a' as u32),
        'å' => (2, 26),
        'ä' => (2, 27),
        'ö' => (2, 28),
        other if other.is_alphabetic() => (2, 1000 + other as u32),
        other => (0, other as u32),
    }
}

/// Compares two runs of ASCII digits by value ("2" < "10"; "007" == "7").
fn compare_numbers(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Folder-name order approximating the old A–Z tree in a Swedish locale: case-insensitive,
/// numbers compared by value ("Work 2" before "Work 10"), å < ä < ö after z.
pub fn compare_folder_names(a: &str, b: &str) -> Ordering {
    let (left, right) = (name_tokens(a), name_tokens(b));
    for (x, y) in left.iter().zip(&right) {
        let order = match (x, y) {
            (NameToken::Number(m), NameToken::Number(n)) => compare_numbers(m, n),
            (NameToken::Number(_), NameToken::Char(c)) => (1, 0).cmp(&char_key(*c)),
            (NameToken::Char(c), NameToken::Number(_)) => char_key(*c).cmp(&(1, 0)),
            (NameToken::Char(c), NameToken::Char(d)) => char_key(*c).cmp(&char_key(*d)),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

/// Rebuilds the search index when its row count differs from `notes`: the first start
/// after the index was added, or notes written by an older build. Idempotent.
pub async fn backfill_notes_fts(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let (indexed, notes): (i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM notes_fts), (SELECT COUNT(*) FROM notes)")
            .fetch_one(pool)
            .await?;
    if indexed == notes {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    crate::commands::notes::rebuild_index(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Rebuilds the link and search indexes when the stored link-index version differs from
/// the code's: the first start after it was added, after the extraction rules changed, or
/// after an older build (which indexes JSON bodies as markdown) reset it. Idempotent.
pub async fn backfill_note_refs(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let stored: Option<String> = sqlx::query_scalar("SELECT Value FROM app_settings WHERE Key = ?")
        .bind(crate::commands::notes::NOTE_REFS_VERSION_KEY)
        .fetch_optional(pool)
        .await?;
    if stored.as_deref() == Some(crate::commands::notes::NOTE_REFS_VERSION) {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    crate::commands::notes::rebuild_index(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_schema_creates_all_tables() {
        let pool = crate::test_db().await;

        // sqlite_master lists all created objects; filter to tables
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&pool)
                .await
                .expect("sqlite_master query failed");

        let names: Vec<&str> = rows.iter().map(|(n,)| n.as_str()).collect();
        assert!(names.contains(&"customers"), "customers table missing");
        assert!(names.contains(&"projects"), "projects table missing");
        assert!(names.contains(&"issues"), "issues table missing");
        assert!(
            names.contains(&"time_entries"),
            "time_entries table missing"
        );
        assert!(names.contains(&"tasks"), "tasks table missing");
        assert!(
            names.contains(&"task_reminders"),
            "task_reminders table missing"
        );
        assert!(!names.contains(&"todos"), "todos table is gone");
        assert!(!names.contains(&"reminders"), "reminders table is gone");
        assert!(
            names.contains(&"app_settings"),
            "app_settings table missing"
        );
        assert!(names.contains(&"folders"), "folders table missing");
        assert!(names.contains(&"notes"), "notes table missing");
        assert!(names.contains(&"note_refs"), "note_refs table missing");
        assert!(names.contains(&"scratchpad"), "scratchpad table missing");
        assert!(
            names.contains(&"note_embeddings"),
            "note_embeddings table missing"
        );
        assert!(
            names.contains(&"apple_sync_items"),
            "apple_sync_items table missing"
        );
    }

    #[tokio::test]
    async fn test_apple_sync_items_is_unique_per_task_and_per_eventkit_id() {
        let pool = crate::test_db().await;
        let insert = |badger_id: i64, ek_id: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO apple_sync_items (BadgerId, EkId, BadgerSnap, PhoneSnap, SyncedAt) \
                     VALUES (?, ?, '{}', '{}', 'x')",
                )
                .bind(badger_id)
                .bind(ek_id)
                .execute(&pool)
                .await
            }
        };
        insert(1, "ek-1").await.unwrap();
        assert!(insert(1, "ek-2").await.is_err(), "one pairing per task");
        assert!(insert(2, "ek-1").await.is_err(), "EkId is unique");
        // No foreign keys: a pairing may point at a task that doesn't exist.
        insert(999, "ek-5").await.unwrap();
        let alarms: String =
            sqlx::query_scalar("SELECT PhoneAlarms FROM apple_sync_items WHERE BadgerId = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(alarms, "[]");
    }

    #[tokio::test]
    async fn test_task_checks_and_links() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'C');
             INSERT INTO projects (id, CustomerId, Name) VALUES (1, 1, 'P');
             INSERT INTO issues (id, ProjectId, Title) VALUES (1, 1, 'I');
             INSERT INTO notes (id, Title, CreatedAt, UpdatedAt) VALUES (1, 'N', 'x', 'x');
             INSERT INTO tasks (id, Title, IssueId, NoteId, CreatedAt, UpdatedAt)
                 VALUES (1, 'T', 1, 1, 'x', 'x');
             INSERT INTO task_reminders (TaskId, OffsetMinutes, CreatedAt) VALUES (1, 0, 'x');
             INSERT INTO task_reminders (TaskId, At, CreatedAt) VALUES (1, '2026-11-20T09:00:00.000', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let bad = [
            "INSERT INTO tasks (Title, DueTime, CreatedAt, UpdatedAt) VALUES ('t', '09:00', 'x', 'x')",
            "INSERT INTO tasks (Title, Repeat, CreatedAt, UpdatedAt) VALUES ('t', 'daily', 'x', 'x')",
            "INSERT INTO tasks (Title, Done, CreatedAt, UpdatedAt) VALUES ('t', 2, 'x', 'x')",
            "INSERT INTO task_reminders (TaskId, CreatedAt) VALUES (1, 'x')",
            "INSERT INTO task_reminders (TaskId, OffsetMinutes, At, CreatedAt) VALUES (1, 0, 'y', 'x')",
            "INSERT INTO task_reminders (TaskId, OffsetMinutes, State, CreatedAt) VALUES (1, 0, 'fired', 'x')",
            "INSERT INTO task_reminders (TaskId, OffsetMinutes, CreatedAt) VALUES (1, -5, 'x')",
            "INSERT INTO task_reminders (TaskId, OffsetMinutes, CreatedAt) VALUES (42, 0, 'x')",
        ];
        for sql in bad {
            assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
        }

        // Deleting the issue or the note unlinks the task; deleting the task deletes its
        // reminders.
        sqlx::raw_sql("DELETE FROM customers; DELETE FROM notes;")
            .execute(&pool)
            .await
            .unwrap();
        let links: (Option<i64>, Option<i64>) =
            sqlx::query_as("SELECT IssueId, NoteId FROM tasks WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(links, (None, None));
        sqlx::query("DELETE FROM tasks")
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_reminders")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    async fn object_names(pool: &sqlx::SqlitePool, kind: &str) -> Vec<String> {
        sqlx::query_as::<_, (String,)>(
            "SELECT name FROM sqlite_master WHERE type = ? AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .bind(kind)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(n,)| n)
        .collect()
    }

    /// Unique DB file under the OS temp dir; removed (with WAL/SHM siblings) on drop.
    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let name = format!("badger-{tag}-{}-{nanos}.db", std::process::id());
            TempDb(std::env::temp_dir().join(name))
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(p);
            }
        }
    }

    async fn assert_notes_schema(pool: &sqlx::SqlitePool) {
        let tables = object_names(pool, "table").await;
        assert!(tables.contains(&"folders".to_string()), "folders missing");
        assert!(tables.contains(&"notes".to_string()), "notes missing");
        let indexes = object_names(pool, "index").await;
        for idx in ["idx_folders_parent", "idx_notes_folder", "idx_notes_title"] {
            assert!(indexes.contains(&idx.to_string()), "{idx} missing");
        }
    }

    #[tokio::test]
    async fn test_init_db_creates_notes_schema_on_fresh_file_and_is_idempotent() {
        let db = TempDb::new("fresh");
        let pool = super::init_db(&db.0).await.unwrap();
        assert_notes_schema(&pool).await;
        let before = (
            object_names(&pool, "table").await,
            object_names(&pool, "index").await,
        );
        pool.close().await;

        let pool = super::init_db(&db.0).await.unwrap(); // second run is a no-op
        let after = (
            object_names(&pool, "table").await,
            object_names(&pool, "index").await,
        );
        assert_eq!(before, after);
        pool.close().await;
    }

    #[tokio::test]
    async fn test_init_db_adds_notes_schema_to_pre_notes_file() {
        let db = TempDb::new("pre-notes");
        {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(&db.0)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
            // Pre-notes shape: everything in SCHEMA_SQL before `folders`.
            let old_schema = super::SCHEMA_SQL
                .split("CREATE TABLE IF NOT EXISTS folders")
                .next()
                .unwrap();
            sqlx::raw_sql(old_schema).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO customers (Name) VALUES ('Kept')")
                .execute(&pool)
                .await
                .unwrap();
            assert!(!object_names(&pool, "table")
                .await
                .contains(&"notes".to_string()));
            pool.close().await;
        }

        let pool = super::init_db(&db.0).await.unwrap();
        assert_notes_schema(&pool).await;
        let kept: (String,) = sqlx::query_as("SELECT Name FROM customers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kept.0, "Kept");
        pool.close().await;
        let pool = super::init_db(&db.0).await.unwrap();
        assert_notes_schema(&pool).await;
        pool.close().await;
    }

    #[tokio::test]
    async fn test_init_db_backfills_search_index_for_existing_notes() {
        let db = TempDb::new("pre-fts");
        {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(&db.0)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
            // Pre-search shape: everything in SCHEMA_SQL before the FTS table.
            let old_schema = super::SCHEMA_SQL
                .split("-- Derived search index")
                .next()
                .unwrap();
            sqlx::raw_sql(old_schema).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO notes (Title, Body, CreatedAt, UpdatedAt) \
                 VALUES ('Old note', 'remember the **quarterly** numbers', 'x', 'x')",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }

        for _ in 0..2 {
            // second run: populated index is left alone
            let pool = super::init_db(&db.0).await.unwrap();
            let hits = crate::commands::notes::inner_search_notes("quarterly", None, &pool)
                .await
                .unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].note.title, "Old note");
            let stats = crate::commands::notes::inner_search_index_stats(&pool)
                .await
                .unwrap();
            assert_eq!(stats.notes, 1);
            pool.close().await;
        }

        // A partly stale index (a note written by a build without the index) is repaired.
        let pool = super::init_db(&db.0).await.unwrap();
        sqlx::query(
            "INSERT INTO notes (Title, Body, CreatedAt, UpdatedAt) VALUES ('Unindexed', 'x', 'x', 'x')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let pool = super::init_db(&db.0).await.unwrap();
        let hits = crate::commands::notes::inner_search_notes("unindexed", None, &pool)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        pool.close().await;
    }

    async fn ref_rows(pool: &sqlx::SqlitePool) -> Vec<(String, String)> {
        sqlx::query_as("SELECT Kind, Target FROM note_refs ORDER BY Kind, Target")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn refs_version(pool: &sqlx::SqlitePool) -> Option<String> {
        sqlx::query_scalar("SELECT Value FROM app_settings WHERE Key = 'note_refs_version'")
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_init_db_builds_link_index_only_when_its_version_changes() {
        let db = TempDb::new("pre-refs");
        {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(&db.0)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
            // Pre-links shape: everything in SCHEMA_SQL before `note_refs`; the search
            // index is complete, so only the version can trigger the build.
            let old_schema = super::SCHEMA_SQL
                .split("-- Derived link index")
                .next()
                .unwrap();
            sqlx::raw_sql(old_schema).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO notes (Title, Body, CreatedAt, UpdatedAt) \
                 VALUES ('Old note', 'see #Acme and [[Plan]] `#code`', 'x', 'x')",
            )
            .execute(&pool)
            .await
            .unwrap();
            let mut conn = pool.acquire().await.unwrap();
            sqlx::query("INSERT INTO notes_fts (rowid, Title, Body) VALUES (1, 'Old note', 'x')")
                .execute(&mut *conn)
                .await
                .unwrap();
            drop(conn);
            pool.close().await;
        }

        let pool = super::init_db(&db.0).await.unwrap();
        let expected = vec![
            ("link".to_string(), "plan".to_string()),
            ("tag".to_string(), "acme".to_string()),
        ];
        assert_eq!(ref_rows(&pool).await, expected);
        assert_eq!(
            refs_version(&pool).await.as_deref(),
            Some(crate::commands::notes::NOTE_REFS_VERSION)
        );
        // Same version: a drifted index is left alone (Settings › Rebuild repairs it).
        sqlx::query("DELETE FROM note_refs")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let pool = super::init_db(&db.0).await.unwrap();
        assert!(ref_rows(&pool).await.is_empty());

        // An old version rebuilds it.
        sqlx::query("UPDATE app_settings SET Value = '0' WHERE Key = 'note_refs_version'")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let pool = super::init_db(&db.0).await.unwrap();
        assert_eq!(ref_rows(&pool).await, expected);
        pool.close().await;
    }

    #[tokio::test]
    async fn test_init_db_clears_json_bodies_from_indexes_after_a_downgrade() {
        let db = TempDb::new("downgrade");
        let pool = super::init_db(&db.0).await.unwrap();
        assert_eq!(
            refs_version(&pool).await.as_deref(),
            Some(crate::commands::notes::NOTE_REFS_VERSION)
        );
        // An older build ran: it indexed a JSON note's body as markdown and reset the
        // version to its own.
        sqlx::raw_sql(
            "INSERT INTO notes (id, Title, Body, CreatedAt, UpdatedAt, Kind)
                 VALUES (1, 'Data', '{\"k\": \"zebracorn #tag [[Plan]]\"}', 'x', 'x', 'json');
             INSERT INTO notes_fts (rowid, Title, Body) VALUES (1, 'Data', 'zebracorn');
             INSERT INTO note_refs (NoteId, Kind, Target) VALUES (1, 'tag', 'tag');
             UPDATE app_settings SET Value = '1' WHERE Key = 'note_refs_version';",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let pool = super::init_db(&db.0).await.unwrap();
        assert!(ref_rows(&pool).await.is_empty());
        let body: String = sqlx::query_scalar("SELECT Body FROM notes_fts WHERE rowid = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body, "");
        let hits = crate::commands::notes::inner_search_notes("data", None, &pool)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "title still searchable");
        assert_eq!(
            refs_version(&pool).await.as_deref(),
            Some(crate::commands::notes::NOTE_REFS_VERSION)
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn test_deleting_note_cascades_to_its_embeddings() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO notes (id, Title, CreatedAt, UpdatedAt) VALUES (1, 'A', 'x', 'x'), (2, 'B', 'x', 'x');
             INSERT INTO note_embeddings (NoteId, ChunkIndex, ContentHash, ModelId, ChunkText, Vector)
                 VALUES (1, 0, 'h', 'm', '', x''), (1, 1, 'h', 'm', '', x''), (2, 0, 'h', 'm', '', x'');",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM notes WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        let left: Vec<(i64,)> = sqlx::query_as("SELECT NoteId FROM note_embeddings")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, vec![(2,)]);
    }

    #[tokio::test]
    async fn test_deleting_folder_nulls_children_and_keeps_notes() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO folders (id, ParentId, Name) VALUES (1, NULL, 'Parent'), (2, 1, 'Child');
             INSERT INTO notes (id, FolderId, Title, CreatedAt, UpdatedAt)
                 VALUES (1, 1, 'In parent', 'x', 'x'), (2, 2, 'In child', 'x', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM folders WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();

        let child: (Option<i64>,) = sqlx::query_as("SELECT ParentId FROM folders WHERE id = 2")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(child.0, None, "child folder must be detached, not deleted");
        let notes: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, FolderId FROM notes ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(notes, vec![(1, None), (2, Some(2))]);
    }

    #[test]
    fn folder_names_compare_case_insensitively_naturally_and_swedish() {
        let mut names = vec![
            "Ödla", "b", "Work 10", "Zebra", "Ärende", "A", "Åsa", "work 2", "éclair", "a",
            "Work 2b", "_misc", "2024",
        ];
        names.sort_by(|a, b| super::compare_folder_names(a, b));
        assert_eq!(
            names,
            vec![
                "_misc", "2024", "A", "a", "b", "éclair", "work 2", "Work 2b", "Work 10", "Zebra",
                "Åsa", "Ärende", "Ödla"
            ]
        );
        assert_eq!(
            super::compare_folder_names("Item 007", "item 7"),
            std::cmp::Ordering::Equal
        );
    }

    #[tokio::test]
    async fn test_migrate_notes_position_backfills_old_order() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // Old shape: folders already had Position (always 0), notes had none.
        sqlx::raw_sql(
            "CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, ParentId INTEGER,
                 Name TEXT NOT NULL, Position INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, FolderId INTEGER,
                 Title TEXT NOT NULL, Body TEXT NOT NULL DEFAULT '',
                 Pinned INTEGER NOT NULL DEFAULT 0, CreatedAt TEXT NOT NULL,
                 UpdatedAt TEXT NOT NULL);
             INSERT INTO folders (id, ParentId, Name) VALUES
                 (1, NULL, 'Work 10'), (2, NULL, 'ödla'), (3, NULL, 'work 2'),
                 (4, NULL, 'Archive'), (5, 4, 'b'), (6, 4, 'A');
             INSERT INTO notes (id, FolderId, Title, Pinned, CreatedAt, UpdatedAt) VALUES
                 (1, NULL, 'old', 0, 'x', '2026-01-01'),
                 (2, NULL, 'new', 0, 'x', '2026-03-01'),
                 (3, NULL, 'pinned old', 1, 'x', '2026-01-01'),
                 (4, NULL, 'tie', 0, 'x', '2026-03-01'),
                 (5, 4, 'only', 0, 'x', '2026-02-01'),
                 (6, 4, 'pinned', 1, 'x', '2025-01-01');",
        )
        .execute(&pool)
        .await
        .unwrap();

        super::migrate_notes_position(&pool).await.unwrap();
        let notes = || async {
            sqlx::query_as::<_, (i64, Option<i64>, i64)>(
                "SELECT id, FolderId, Position FROM notes ORDER BY FolderId, Position",
            )
            .fetch_all(&pool)
            .await
            .unwrap()
        };
        let folders = || async {
            sqlx::query_as::<_, (i64, Option<i64>, i64)>(
                "SELECT id, ParentId, Position FROM folders ORDER BY ParentId, Position",
            )
            .fetch_all(&pool)
            .await
            .unwrap()
        };
        let expected_notes = vec![
            (3, None, 0),
            (2, None, 1),
            (4, None, 2),
            (1, None, 3),
            (6, Some(4), 0),
            (5, Some(4), 1),
        ];
        let expected_folders = vec![
            (4, None, 0),
            (3, None, 1),
            (1, None, 2),
            (2, None, 3),
            (6, Some(4), 0),
            (5, Some(4), 1),
        ];
        assert_eq!(notes().await, expected_notes);
        assert_eq!(folders().await, expected_folders);

        // Second run is a no-op: manual changes survive.
        sqlx::query("UPDATE notes SET Position = 9 WHERE id = 3")
            .execute(&pool)
            .await
            .unwrap();
        super::migrate_notes_position(&pool).await.unwrap();
        let position: (i64,) = sqlx::query_as("SELECT Position FROM notes WHERE id = 3")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(position.0, 9);
        assert_eq!(folders().await, expected_folders);
    }

    #[tokio::test]
    async fn test_init_db_migrates_notes_without_position() {
        let db = TempDb::new("pre-position");
        {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(&db.0)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
            let old_schema = super::SCHEMA_SQL.replace(
                ",\n    -- Manual order within the folder; shown as Pinned DESC, Position, id.\n    Position  INTEGER NOT NULL DEFAULT 0",
                "",
            );
            assert_ne!(
                old_schema,
                super::SCHEMA_SQL,
                "old shape must drop Position"
            );
            sqlx::raw_sql(sqlx::AssertSqlSafe(old_schema))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO notes (Title, CreatedAt, UpdatedAt) VALUES ('a', 'x', '1'), ('b', 'x', '2')",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }
        let pool = super::init_db(&db.0).await.unwrap();
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT Title, Position FROM notes ORDER BY Position")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows, vec![("b".into(), 0), ("a".into(), 1)]);
        pool.close().await;
    }

    #[tokio::test]
    async fn test_migrate_kind_adds_markdown_default_to_notes_and_scratchpad() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // Old shape: no Kind on either table.
        sqlx::raw_sql(
            "CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, FolderId INTEGER,
                 Title TEXT NOT NULL, Body TEXT NOT NULL DEFAULT '',
                 Pinned INTEGER NOT NULL DEFAULT 0, CreatedAt TEXT NOT NULL,
                 UpdatedAt TEXT NOT NULL, Position INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE scratchpad (id INTEGER PRIMARY KEY CHECK (id = 1),
                 Body TEXT NOT NULL DEFAULT '', UpdatedAt TEXT NOT NULL);
             INSERT INTO notes (Title, Body, CreatedAt, UpdatedAt) VALUES ('a', '{}', 'x', 'x');
             INSERT INTO scratchpad (id, Body, UpdatedAt) VALUES (1, 'pad', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();

        for _ in 0..2 {
            // second run is a no-op
            super::migrate_notes_kind(&pool).await.unwrap();
            super::migrate_scratchpad_kind(&pool).await.unwrap();
        }
        let note: (String, String) = sqlx::query_as("SELECT Body, Kind FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(note, ("{}".into(), "markdown".into()));
        let pad: (String, String) = sqlx::query_as("SELECT Body, Kind FROM scratchpad")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(pad, ("pad".into(), "markdown".into()));
        // New rows default to markdown; unknown kinds are rejected.
        sqlx::query("INSERT INTO notes (Title, CreatedAt, UpdatedAt) VALUES ('b', 'x', 'x')")
            .execute(&pool)
            .await
            .unwrap();
        let kinds: Vec<(String,)> = sqlx::query_as("SELECT Kind FROM notes ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(kinds, vec![("markdown".into(),), ("markdown".into(),)]);
        assert!(sqlx::query("UPDATE notes SET Kind = 'yaml'")
            .execute(&pool)
            .await
            .is_err());
        sqlx::query("UPDATE scratchpad SET Kind = 'json'")
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_init_db_migrates_notes_and_scratchpad_without_kind() {
        let db = TempDb::new("pre-kind");
        {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(&db.0)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
            let kind_line = ",\n    -- Editor mode: 'markdown' or 'json'. JSON notes are title-only in the derived indexes.\n    Kind      TEXT    NOT NULL DEFAULT 'markdown' CHECK (Kind IN ('markdown', 'json'))";
            let pad_line = ",\n    -- Editor mode: 'markdown' or 'json'.\n    Kind      TEXT    NOT NULL DEFAULT 'markdown' CHECK (Kind IN ('markdown', 'json'))";
            let old_schema = super::SCHEMA_SQL
                .replace(kind_line, "")
                .replace(pad_line, "");
            assert_eq!(
                old_schema
                    .matches("Kind      TEXT    NOT NULL DEFAULT 'markdown'")
                    .count(),
                0,
                "old shape must drop both Kind columns"
            );
            sqlx::raw_sql(sqlx::AssertSqlSafe(old_schema))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::raw_sql(
                "INSERT INTO notes (Title, Body, CreatedAt, UpdatedAt) VALUES ('Old', 'see #tag', 'x', 'x');
                 INSERT INTO scratchpad (id, Body, UpdatedAt) VALUES (1, 'jot', 'x');",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }
        for _ in 0..2 {
            let pool = super::init_db(&db.0).await.unwrap();
            let note = crate::commands::notes::inner_get_note(1, &pool)
                .await
                .unwrap();
            assert_eq!(note.kind, "markdown");
            let pad = crate::commands::scratchpad::inner_get_scratchpad(&pool)
                .await
                .unwrap();
            assert_eq!((pad.body.as_str(), pad.kind.as_str()), ("jot", "markdown"));
            pool.close().await;
        }
    }

    #[tokio::test]
    async fn test_migrate_issues_drops_external_id() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE projects (id INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE issues (id INTEGER PRIMARY KEY AUTOINCREMENT,
                 ProjectId INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                 ExternalId TEXT, Title TEXT NOT NULL);
             CREATE TABLE time_entries (id INTEGER PRIMARY KEY AUTOINCREMENT,
                 IssueId INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE);
             INSERT INTO projects (id) VALUES (1);
             INSERT INTO issues (id, ProjectId, ExternalId, Title) VALUES (1, 1, '42', 'Kept'), (2, 1, NULL, 'Also kept');
             INSERT INTO time_entries (IssueId) VALUES (1), (2);",
        )
        .execute(&pool)
        .await
        .unwrap();

        super::migrate_issues_drop_external_id(&pool).await.unwrap();
        super::migrate_issues_drop_external_id(&pool).await.unwrap(); // idempotent

        let columns: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('issues') ORDER BY cid")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            columns,
            vec![("id".into(),), ("ProjectId".into(),), ("Title".into(),)]
        );
        let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, Title FROM issues ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows, vec![(1, "Kept".into()), (2, "Also kept".into())]);
        let entries: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM time_entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(entries.0, 2, "time entries must survive the migration");
    }

    /// The schema a 1.6 database has: todos, reminders and the two-kind pairing table.
    const OLD_TASK_TABLES: &str = "
        CREATE TABLE todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT, Title TEXT NOT NULL, Notes TEXT,
            Status TEXT NOT NULL DEFAULT 'not_started'
                CHECK (Status IN ('not_started', 'in_progress', 'done')),
            Position INTEGER NOT NULL DEFAULT 0, DueDate TEXT, CreatedAt TEXT NOT NULL,
            IssueId INTEGER REFERENCES issues(id) ON DELETE SET NULL,
            MacOnly INTEGER NOT NULL DEFAULT 0 CHECK (MacOnly IN (0, 1)));
        CREATE TABLE reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT, Text TEXT NOT NULL, RemindAt TEXT NOT NULL,
            TimeOfDay TEXT NOT NULL, Repeat TEXT NOT NULL DEFAULT 'none',
            Weekdays TEXT NOT NULL DEFAULT '', MonthDay INTEGER,
            State TEXT NOT NULL DEFAULT 'pending',
            TodoId INTEGER REFERENCES todos(id) ON DELETE CASCADE,
            NoteId INTEGER REFERENCES notes(id) ON DELETE CASCADE,
            CreatedAt TEXT NOT NULL, UpdatedAt TEXT NOT NULL,
            MacOnly INTEGER NOT NULL DEFAULT 0, CHECK (TodoId IS NULL OR NoteId IS NULL));
        CREATE INDEX idx_reminders_state ON reminders(State, RemindAt);
        CREATE INDEX idx_reminders_todo ON reminders(TodoId);
        CREATE TABLE apple_sync_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            Kind TEXT NOT NULL CHECK (Kind IN ('reminder', 'todo')),
            BadgerId INTEGER NOT NULL, EkId TEXT NOT NULL, ExternalId TEXT,
            BadgerSnap TEXT NOT NULL, PhoneSnap TEXT NOT NULL, PhoneDue TEXT,
            SyncedAt TEXT NOT NULL, MissedPasses INTEGER NOT NULL DEFAULT 0,
            UNIQUE (Kind, BadgerId), UNIQUE (EkId));
    ";

    /// Builds a 1.6-shaped database file: today's schema without the task tables, the old
    /// tables with a linked todo, reminder and same-id pairings of both kinds, and the old
    /// sync settings.
    async fn build_pre_tasks_db(path: &std::path::Path, with_old_lists: bool) {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        let new_tables = (
            super::SCHEMA_SQL.find("-- Tasks: open or done").unwrap(),
            super::SCHEMA_SQL
                .find("-- The scratchpad: a single row")
                .unwrap(),
        );
        let old_schema = format!(
            "{}{}",
            &super::SCHEMA_SQL[..new_tables.0],
            &super::SCHEMA_SQL[new_tables.1..]
        )
        .replace(super::APPLE_SYNC_ITEMS_DDL, "");
        sqlx::raw_sql(sqlx::AssertSqlSafe(old_schema))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(OLD_TASK_TABLES).execute(&pool).await.unwrap();
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'Kept');
             INSERT INTO notes (id, Title, CreatedAt, UpdatedAt) VALUES (1, 'Kept note', 'x', 'x');
             INSERT INTO todos (id, Title, CreatedAt) VALUES (1, 'Old todo', 'x');
             INSERT INTO reminders (id, Text, RemindAt, TimeOfDay, TodoId, CreatedAt, UpdatedAt)
                 VALUES (1, 'Old reminder', '2026-11-09T09:00:00.000', '09:00', 1, 'x', 'x');
             INSERT INTO apple_sync_items (Kind, BadgerId, EkId, BadgerSnap, PhoneSnap, SyncedAt)
                 VALUES ('todo', 1, 'ek-t', '{}', '{}', 'x'), ('reminder', 1, 'ek-r', '{}', '{}', 'x');
             INSERT INTO app_settings (Key, Value) VALUES
                 ('apple_sync_enabled', 'true'),
                 ('apple_sync_last_sync_at', '2026-11-20T10:00:00.000'),
                 ('apple_sync_paused', 'empty_fetch'),
                 ('apple_sync_last_error', 'Boom'),
                 ('apple_sync_skipped', '[]'),
                 ('apple_sync_restore_pending', 'true'),
                 ('theme', 'dark');",
        )
        .execute(&pool)
        .await
        .unwrap();
        if with_old_lists {
            sqlx::raw_sql(
                "INSERT INTO app_settings (Key, Value) VALUES
                     ('apple_sync_reminders_list_id', 'cal-r'),
                     ('apple_sync_todos_list_id', 'cal-t');",
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        pool.close().await;
    }

    async fn settings(pool: &sqlx::SqlitePool) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT Key, Value FROM app_settings WHERE Key != 'note_refs_version' ORDER BY Key",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn columns(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
            .bind(table)
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_init_db_replaces_todos_and_reminders_with_tasks_once() {
        let db = TempDb::new("pre-tasks");
        build_pre_tasks_db(&db.0, true).await;

        let pool = super::init_db(&db.0).await.unwrap();
        let tables = object_names(&pool, "table").await;
        assert!(!tables.contains(&"todos".to_string()), "{tables:?}");
        assert!(!tables.contains(&"reminders".to_string()), "{tables:?}");
        assert!(tables.contains(&"tasks".to_string()));
        assert!(tables.contains(&"task_reminders".to_string()));
        let indexes = object_names(&pool, "index").await;
        assert!(
            !indexes.iter().any(|i| i.starts_with("idx_reminders")),
            "{indexes:?}"
        );
        assert!(
            indexes.contains(&"idx_task_reminders_task".to_string()),
            "{indexes:?}"
        );
        let pairings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM apple_sync_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(pairings, 0, "pairings of both kinds are gone");
        let cols = columns(&pool, "apple_sync_items").await;
        assert!(!cols.contains(&"Kind".to_string()), "{cols:?}");
        assert!(cols.contains(&"PhoneAlarms".to_string()), "{cols:?}");
        let expected = vec![
            ("apple_sync_enabled".to_string(), "true".to_string()),
            (
                "apple_sync_last_sync_at".to_string(),
                "2026-11-20T10:00:00.000".to_string(),
            ),
            ("apple_sync_old_lists_note".to_string(), "true".to_string()),
            ("tasks_migrated".to_string(), "1".to_string()),
            ("theme".to_string(), "dark".to_string()),
        ];
        assert_eq!(settings(&pool).await, expected);
        // Everything else is kept.
        let kept: (String, String) =
            sqlx::query_as("SELECT (SELECT Name FROM customers), (SELECT Title FROM notes)")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(kept, ("Kept".into(), "Kept note".into()));

        // Second run is a no-op: new data survives.
        sqlx::raw_sql(
            "INSERT INTO tasks (id, Title, CreatedAt, UpdatedAt) VALUES (1, 'New', 'x', 'x');
             INSERT INTO apple_sync_items (BadgerId, EkId, BadgerSnap, PhoneSnap, SyncedAt)
                 VALUES (1, 'ek-1', '{}', '{}', 'x');
             DELETE FROM app_settings WHERE Key = 'apple_sync_old_lists_note';",
        )
        .execute(&pool)
        .await
        .unwrap();
        let before = (
            object_names(&pool, "table").await,
            object_names(&pool, "index").await,
        );
        pool.close().await;
        let pool = super::init_db(&db.0).await.unwrap();
        let after = (
            object_names(&pool, "table").await,
            object_names(&pool, "index").await,
        );
        assert_eq!(before, after);
        let kept: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM tasks), (SELECT COUNT(*) FROM apple_sync_items)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kept, (1, 1));
        assert!(
            !settings(&pool)
                .await
                .iter()
                .any(|(k, _)| k == "apple_sync_old_lists_note"),
            "a dismissed note stays dismissed"
        );

        // A downgrade recreates the old tables; re-upgrading never drops again.
        sqlx::raw_sql(
            "CREATE TABLE todos (id INTEGER PRIMARY KEY, Title TEXT NOT NULL);
             INSERT INTO todos (id, Title) VALUES (7, 'From the old build');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let pool = super::init_db(&db.0).await.unwrap();
        let (todos, tasks, pairings): (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM todos), (SELECT COUNT(*) FROM tasks),
                    (SELECT COUNT(*) FROM apple_sync_items)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((todos, tasks, pairings), (1, 1, 1));
        pool.close().await;
    }

    #[tokio::test]
    async fn test_init_db_without_old_lists_sets_no_note() {
        let db = TempDb::new("pre-tasks-no-lists");
        build_pre_tasks_db(&db.0, false).await;
        let pool = super::init_db(&db.0).await.unwrap();
        let keys: Vec<String> = settings(&pool).await.into_iter().map(|(k, _)| k).collect();
        assert!(
            !keys.contains(&"apple_sync_old_lists_note".to_string()),
            "{keys:?}"
        );
        assert!(keys.contains(&"tasks_migrated".to_string()));
        pool.close().await;
    }

    #[tokio::test]
    async fn test_fresh_db_gets_the_same_shape_as_a_migrated_one() {
        let fresh = TempDb::new("fresh-tasks");
        let pool = super::init_db(&fresh.0).await.unwrap();
        let fresh_shape = (
            object_names(&pool, "table").await,
            object_names(&pool, "index").await,
            columns(&pool, "apple_sync_items").await,
            columns(&pool, "tasks").await,
            columns(&pool, "task_reminders").await,
        );
        assert_eq!(
            settings(&pool).await,
            vec![("tasks_migrated".to_string(), "1".to_string())]
        );
        pool.close().await;

        let old = TempDb::new("migrated-tasks");
        build_pre_tasks_db(&old.0, true).await;
        let pool = super::init_db(&old.0).await.unwrap();
        let migrated_shape = (
            object_names(&pool, "table").await,
            object_names(&pool, "index").await,
            columns(&pool, "apple_sync_items").await,
            columns(&pool, "tasks").await,
            columns(&pool, "task_reminders").await,
        );
        assert_eq!(fresh_shape, migrated_shape);
        pool.close().await;
    }

    #[tokio::test]
    async fn test_fk_pragma_enabled() {
        let pool = crate::test_db().await;

        // PRAGMA foreign_keys returns 0 or 1
        let row: (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("PRAGMA foreign_keys failed");

        assert_eq!(row.0, 1, "Foreign keys must be ON (1), got 0");
    }

    #[tokio::test]
    async fn test_app_settings_schema_uses_expected_columns() {
        let pool = crate::test_db().await;

        // PRAGMA table_info columns: cid, name, type, notnull, dflt_value, pk
        type TableInfoRow = (i64, String, String, i64, Option<String>, i64);

        let rows: Vec<TableInfoRow> = sqlx::query_as("PRAGMA table_info(app_settings)")
            .fetch_all(&pool)
            .await
            .expect("PRAGMA table_info(app_settings) failed");

        assert_eq!(
            rows.len(),
            2,
            "app_settings should have exactly two columns"
        );

        let (_, key_name, key_type, key_not_null, _, key_pk) = &rows[0];
        assert_eq!(key_name, "Key");
        assert_eq!(key_type, "TEXT");
        assert_eq!(
            *key_not_null, 0,
            "Key should not be declared NOT NULL separately"
        );
        assert_eq!(*key_pk, 1, "Key should be the primary key");

        let (_, value_name, value_type, value_not_null, _, value_pk) = &rows[1];
        assert_eq!(value_name, "Value");
        assert_eq!(value_type, "TEXT");
        assert_eq!(*value_not_null, 1, "Value should be NOT NULL");
        assert_eq!(*value_pk, 0, "Value should not be part of the primary key");
    }
}
