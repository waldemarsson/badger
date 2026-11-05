// src-tauri/src/db.rs
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Pool, Sqlite,
};
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
    ExternalId  TEXT,
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

CREATE TABLE IF NOT EXISTS app_settings (
    Key       TEXT    PRIMARY KEY,
    Value     TEXT    NOT NULL
);
";

/// Opens (or creates) the SQLite database at `db_path` with WAL mode and FK enforcement,
/// then runs `SCHEMA_SQL` to create all tables idempotently.
pub async fn init_db(db_path: &Path) -> Result<Pool<Sqlite>, sqlx::Error> {
    // Ensure ~/.timetracker/ directory exists
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
