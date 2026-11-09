// src-tauri/src/db.rs
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Pool, Sqlite, SqliteConnection,
};
use std::cmp::Ordering;
use std::path::Path;

/// Full schema DDL.  Uses PRAGMA lines so in-memory test pools also get FK enforcement.
/// Execute via `sqlx::raw_sql()` — `sqlx::query()` only runs the first statement in a
/// multi-statement string.
pub const SCHEMA_SQL: &str = "
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

CREATE TABLE IF NOT EXISTS todos (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    Title     TEXT    NOT NULL,
    Notes     TEXT,
    Status    TEXT    NOT NULL DEFAULT 'not_started'
              CHECK (Status IN ('not_started', 'in_progress', 'done')),
    Position  INTEGER NOT NULL DEFAULT 0,
    DueDate   TEXT,
    CreatedAt TEXT    NOT NULL,
    IssueId   INTEGER REFERENCES issues(id) ON DELETE SET NULL
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
    Position  INTEGER NOT NULL DEFAULT 0
);

-- Reminders: the current occurrence (RemindAt, naive local time), the recurrence anchor
-- (TimeOfDay HH:MM) and rule, an optional link to one todo or one note, and the state.
CREATE TABLE IF NOT EXISTS reminders (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    Text      TEXT    NOT NULL,
    RemindAt  TEXT    NOT NULL,
    TimeOfDay TEXT    NOT NULL,
    Repeat    TEXT    NOT NULL DEFAULT 'none'
              CHECK (Repeat IN ('none', 'daily', 'weekdays', 'weekly', 'monthly')),
    Weekdays  TEXT    NOT NULL DEFAULT '',
    MonthDay  INTEGER CHECK (MonthDay IS NULL OR MonthDay BETWEEN 1 AND 31),
    State     TEXT    NOT NULL DEFAULT 'pending'
              CHECK (State IN ('pending', 'fired', 'done')),
    TodoId    INTEGER REFERENCES todos(id) ON DELETE CASCADE,
    NoteId    INTEGER REFERENCES notes(id) ON DELETE CASCADE,
    CreatedAt TEXT    NOT NULL,
    UpdatedAt TEXT    NOT NULL,
    CHECK (TodoId IS NULL OR NoteId IS NULL)
);
CREATE INDEX IF NOT EXISTS idx_reminders_state ON reminders(State, RemindAt);
CREATE INDEX IF NOT EXISTS idx_reminders_todo ON reminders(TodoId);
CREATE INDEX IF NOT EXISTS idx_reminders_note ON reminders(NoteId);

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
";

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
    migrate_todos_status(&pool).await?;
    migrate_issues_drop_external_id(&pool).await?;
    migrate_notes_position(&pool).await?;
    backfill_notes_fts(&pool).await?;
    backfill_note_refs(&pool).await?;

    Ok(pool)
}

/// Upgrades a `todos` table created with the early `Done` flag (no `Status`/`Position`).
/// Idempotent: does nothing once `Status` exists.
pub async fn migrate_todos_status(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let columns: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('todos')")
        .fetch_all(pool)
        .await?;
    if columns.iter().any(|(name,)| name == "Status") {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::raw_sql(
        "ALTER TABLE todos ADD COLUMN Status TEXT NOT NULL DEFAULT 'not_started' \
             CHECK (Status IN ('not_started', 'in_progress', 'done'));
         ALTER TABLE todos ADD COLUMN Position INTEGER NOT NULL DEFAULT 0;
         UPDATE todos SET Status = CASE WHEN Done = 1 THEN 'done' ELSE 'not_started' END,
                          Position = id;
         ALTER TABLE todos DROP COLUMN Done;",
    )
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

/// Rebuilds the link index when its stored version differs from the code's: the first
/// start after it was added, or after the extraction rules changed. Idempotent.
pub async fn backfill_note_refs(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let stored: Option<String> = sqlx::query_scalar("SELECT Value FROM app_settings WHERE Key = ?")
        .bind(crate::commands::notes::NOTE_REFS_VERSION_KEY)
        .fetch_optional(pool)
        .await?;
    if stored.as_deref() == Some(crate::commands::notes::NOTE_REFS_VERSION) {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    crate::commands::notes::rebuild_refs(&mut tx).await?;
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
        assert!(names.contains(&"todos"), "todos table missing");
        assert!(
            names.contains(&"app_settings"),
            "app_settings table missing"
        );
        assert!(names.contains(&"folders"), "folders table missing");
        assert!(names.contains(&"notes"), "notes table missing");
        assert!(names.contains(&"note_refs"), "note_refs table missing");
        assert!(
            names.contains(&"note_embeddings"),
            "note_embeddings table missing"
        );
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

    #[tokio::test]
    async fn test_migrate_todos_from_done_flag() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE todos (id INTEGER PRIMARY KEY AUTOINCREMENT, Title TEXT NOT NULL,
                 Notes TEXT, Done INTEGER NOT NULL DEFAULT 0, DueDate TEXT,
                 CreatedAt TEXT NOT NULL, IssueId INTEGER);
             INSERT INTO todos (Title, Done, CreatedAt) VALUES ('open', 0, 'x'), ('closed', 1, 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();

        super::migrate_todos_status(&pool).await.unwrap();
        super::migrate_todos_status(&pool).await.unwrap(); // idempotent

        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT Title, Status, Position FROM todos ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("open".into(), "not_started".into(), 1),
                ("closed".into(), "done".into(), 2)
            ]
        );
        let done_col: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('todos') WHERE name = 'Done'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(done_col.0, 0, "Done column must be dropped");
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
            sqlx::raw_sql(&old_schema).execute(&pool).await.unwrap();
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
