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

    Ok(pool)
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
        assert!(
            names.contains(&"app_settings"),
            "app_settings table missing"
        );
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

        let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(app_settings)")
                .fetch_all(&pool)
                .await
                .expect("PRAGMA table_info(app_settings) failed");

        assert_eq!(
            rows.len(),
            2,
            "app_settings should have exactly two columns"
        );

        assert_eq!(rows[0].1, "Key");
        assert_eq!(rows[0].2, "TEXT");
        assert_eq!(
            rows[0].3, 0,
            "Key should not be declared NOT NULL separately"
        );
        assert_eq!(rows[0].5, 1, "Key should be the primary key");

        assert_eq!(rows[1].1, "Value");
        assert_eq!(rows[1].2, "TEXT");
        assert_eq!(rows[1].3, 1, "Value should be NOT NULL");
        assert_eq!(rows[1].5, 0, "Value should not be part of the primary key");
    }
}
