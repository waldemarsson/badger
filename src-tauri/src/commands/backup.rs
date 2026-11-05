/// Backup and restore commands for exporting/importing all app data as JSON.
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use tauri::State;

/// A customer record as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupCustomer {
    pub id: i64,
    pub name: String,
    pub use_as_default: bool,
}

/// A project record as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupProject {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub use_as_default: bool,
}

/// An issue record as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupIssue {
    pub id: i64,
    pub project_id: i64,
    pub external_id: Option<String>,
    pub title: String,
}

/// A time entry record as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupTimeEntry {
    pub id: i64,
    pub issue_id: i64,
    pub date: String,
    pub hours: f64,
    pub notes: Option<String>,
}

/// A todo record as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupTodo {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub done: bool,
    pub due_date: Option<String>,
    pub created_at: String,
    pub issue_id: Option<i64>,
}

/// Full database export structure serialised to JSON.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupData {
    pub customers: Vec<BackupCustomer>,
    pub projects: Vec<BackupProject>,
    pub issues: Vec<BackupIssue>,
    pub time_entries: Vec<BackupTimeEntry>,
    /// Absent in backups made before todos existed.
    #[serde(default)]
    pub todos: Vec<BackupTodo>,
}

/// Reads all data from the database and serialises it to a JSON string.
pub async fn inner_backup_data(pool: &Pool<Sqlite>) -> Result<String, AppError> {
    let customers = sqlx::query_as::<_, BackupCustomer>(
        "SELECT id, Name as name, UseAsDefault as use_as_default FROM customers ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let projects = sqlx::query_as::<_, BackupProject>(
        "SELECT id, CustomerId as customer_id, Name as name, UseAsDefault as use_as_default FROM projects ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let issues = sqlx::query_as::<_, BackupIssue>(
        "SELECT id, ProjectId as project_id, ExternalId as external_id, Title as title FROM issues ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let time_entries = sqlx::query_as::<_, BackupTimeEntry>(
        "SELECT id, IssueId as issue_id, Date as date, Hours as hours, Notes as notes FROM time_entries ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let todos = sqlx::query_as::<_, BackupTodo>(
        "SELECT id, Title as title, Notes as notes, Done as done, DueDate as due_date, CreatedAt as created_at, IssueId as issue_id FROM todos ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let backup = BackupData {
        customers,
        projects,
        issues,
        time_entries,
        todos,
    };
    serde_json::to_string(&backup).map_err(|e| AppError::Custom(e.to_string()))
}

/// Applies a JSON backup string to the database in either `replace` or `merge` mode.
///
/// - `replace`: deletes all existing data (cascades via FK) then re-inserts from backup.
/// - `merge`: upserts records from backup without deleting unrelated existing data.
pub async fn inner_restore_data(
    mode: &str,
    json: &str,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    let data: BackupData = serde_json::from_str(json)
        .map_err(|e| AppError::Custom(format!("Invalid backup JSON: {}", e)))?;

    let mut tx = pool.begin().await?;

    if mode == "replace" {
        // Single delete cascades to projects → issues → time_entries via ON DELETE CASCADE.
        sqlx::query("DELETE FROM customers")
            .execute(&mut *tx)
            .await?;
        // Todos are not reached by the cascade (their issue link is SET NULL).
        sqlx::query("DELETE FROM todos").execute(&mut *tx).await?;

        for c in &data.customers {
            sqlx::query("INSERT INTO customers (id, Name, UseAsDefault) VALUES (?, ?, ?)")
                .bind(c.id)
                .bind(&c.name)
                .bind(c.use_as_default)
                .execute(&mut *tx)
                .await?;
        }
        for p in &data.projects {
            sqlx::query(
                "INSERT INTO projects (id, CustomerId, Name, UseAsDefault) VALUES (?, ?, ?, ?)",
            )
            .bind(p.id)
            .bind(p.customer_id)
            .bind(&p.name)
            .bind(p.use_as_default)
            .execute(&mut *tx)
            .await?;
        }
        for i in &data.issues {
            sqlx::query(
                "INSERT INTO issues (id, ProjectId, ExternalId, Title) VALUES (?, ?, ?, ?)",
            )
            .bind(i.id)
            .bind(i.project_id)
            .bind(&i.external_id)
            .bind(&i.title)
            .execute(&mut *tx)
            .await?;
        }
        for e in &data.time_entries {
            sqlx::query(
                "INSERT INTO time_entries (id, IssueId, Date, Hours, Notes) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(e.id)
            .bind(e.issue_id)
            .bind(&e.date)
            .bind(e.hours)
            .bind(&e.notes)
            .execute(&mut *tx)
            .await?;
        }
        insert_todos(&data.todos, &mut tx).await?;
    } else if mode == "merge" {
        for c in &data.customers {
            sqlx::query(
                "INSERT OR REPLACE INTO customers (id, Name, UseAsDefault) VALUES (?, ?, ?)",
            )
            .bind(c.id)
            .bind(&c.name)
            .bind(c.use_as_default)
            .execute(&mut *tx)
            .await?;
        }
        for p in &data.projects {
            sqlx::query("INSERT OR REPLACE INTO projects (id, CustomerId, Name, UseAsDefault) VALUES (?, ?, ?, ?)")
                .bind(p.id)
                .bind(p.customer_id)
                .bind(&p.name)
                .bind(p.use_as_default)
                .execute(&mut *tx)
                .await?;
        }
        for i in &data.issues {
            sqlx::query("INSERT OR REPLACE INTO issues (id, ProjectId, ExternalId, Title) VALUES (?, ?, ?, ?)")
                .bind(i.id)
                .bind(i.project_id)
                .bind(&i.external_id)
                .bind(&i.title)
                .execute(&mut *tx)
                .await?;
        }
        for e in &data.time_entries {
            sqlx::query("INSERT OR REPLACE INTO time_entries (id, IssueId, Date, Hours, Notes) VALUES (?, ?, ?, ?, ?)")
                .bind(e.id)
                .bind(e.issue_id)
                .bind(&e.date)
                .bind(e.hours)
                .bind(&e.notes)
                .execute(&mut *tx)
                .await?;
        }
        insert_todos(&data.todos, &mut tx).await?;
    } else {
        return Err(AppError::Custom(format!("Unknown restore mode: {}", mode)));
    }

    tx.commit().await?;
    Ok(())
}

/// Upserts todos by id. Runs after issues so `IssueId` references resolve.
async fn insert_todos(
    todos: &[BackupTodo],
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<(), AppError> {
    for t in todos {
        sqlx::query(
            "INSERT OR REPLACE INTO todos (id, Title, Notes, Done, DueDate, CreatedAt, IssueId) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(t.id)
        .bind(&t.title)
        .bind(&t.notes)
        .bind(t.done)
        .bind(&t.due_date)
        .bind(&t.created_at)
        .bind(t.issue_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Serialises all data to JSON and writes it to the given file path.
#[tauri::command]
pub async fn backup_data_to_file(
    path: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<(), AppError> {
    let json = inner_backup_data(&pool).await?;
    std::fs::write(&path, &json)?;
    Ok(())
}

/// Reads a backup JSON file from disk and returns its contents.
#[tauri::command]
pub async fn read_backup_file(path: String) -> Result<String, AppError> {
    let content = std::fs::read_to_string(&path)?;
    Ok(content)
}

/// Applies a JSON backup to the database in the given mode (`replace` or `merge`).
#[tauri::command]
pub async fn restore_data(
    mode: String,
    json: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<(), AppError> {
    inner_restore_data(&mode, &json, &pool).await
}

/// Reads the startup backup retention setting from the database.
pub async fn inner_get_startup_backup_retention(pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    crate::app_settings::get_startup_backup_retention(pool).await
}

/// Writes the startup backup retention setting to the database.
pub async fn inner_set_startup_backup_retention(
    value: i64,
    pool: &Pool<Sqlite>,
) -> Result<i64, AppError> {
    crate::app_settings::set_startup_backup_retention(pool, value).await
}

/// Lists all stored startup backups by reading the backup directory structure.
pub async fn inner_list_startup_backups(
    db_path: &Path,
) -> Result<Vec<crate::startup_backup::StoredBackupSummary>, AppError> {
    crate::startup_backup::list_startup_backups(db_path)
}

/// Reads the startup backup retention setting from the database.
#[tauri::command]
pub async fn get_startup_backup_retention(
    pool: State<'_, Pool<Sqlite>>,
) -> Result<i64, AppError> {
    inner_get_startup_backup_retention(&pool).await
}

/// Writes the startup backup retention setting to the database.
#[tauri::command]
pub async fn set_startup_backup_retention(
    value: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<i64, AppError> {
    inner_set_startup_backup_retention(value, &pool).await
}

/// Lists all stored startup backups.
#[tauri::command]
pub async fn list_startup_backups(
    runtime_paths: State<'_, crate::runtime_paths::RuntimePaths>,
) -> Result<Vec<crate::startup_backup::StoredBackupSummary>, AppError> {
    inner_list_startup_backups(&runtime_paths.db_path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_test_data(pool: &Pool<Sqlite>) {
        sqlx::query("INSERT INTO customers (id, Name, UseAsDefault) VALUES (1, 'Test Co', 0)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (id, CustomerId, Name, UseAsDefault) VALUES (1, 1, 'Test Project', 0)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO issues (id, ProjectId, ExternalId, Title) VALUES (1, 1, NULL, 'Test Issue')")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO time_entries (id, IssueId, Date, Hours, Notes) VALUES (1, 1, '2024-01-01', 2.5, NULL)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO todos (id, Title, Notes, Done, DueDate, CreatedAt, IssueId) VALUES (1, 'Linked', 'n', 0, '2024-02-01', '2024-01-01T00:00:00.000', 1), (2, 'Standalone', NULL, 1, NULL, '2024-01-02T00:00:00.000', NULL)")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_backup_restore_replace_roundtrip() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;

        let json = inner_backup_data(&pool).await.expect("backup failed");

        inner_restore_data("replace", &json, &pool)
            .await
            .expect("restore failed");

        let restored = inner_backup_data(&pool)
            .await
            .expect("second backup failed");
        let original: BackupData = serde_json::from_str(&json).unwrap();
        let result: BackupData = serde_json::from_str(&restored).unwrap();

        assert_eq!(result.customers.len(), original.customers.len());
        assert_eq!(result.projects.len(), original.projects.len());
        assert_eq!(result.issues.len(), original.issues.len());
        assert_eq!(result.time_entries.len(), original.time_entries.len());
        assert_eq!(result.customers[0].name, original.customers[0].name);
        assert_eq!(result.todos.len(), 2);
        assert_eq!(result.todos[0].issue_id, Some(1));
        assert_eq!(result.todos[1].issue_id, None);
        assert!(result.todos[1].done);
    }

    #[tokio::test]
    async fn test_restore_merge_roundtrips_todos() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;
        let json = inner_backup_data(&pool).await.expect("backup failed");

        sqlx::query("DELETE FROM todos")
            .execute(&pool)
            .await
            .unwrap();
        inner_restore_data("merge", &json, &pool)
            .await
            .expect("merge restore failed");

        let result: BackupData =
            serde_json::from_str(&inner_backup_data(&pool).await.unwrap()).unwrap();
        assert_eq!(result.todos.len(), 2);
        assert_eq!(result.todos[0].title, "Linked");
        assert_eq!(result.todos[0].issue_id, Some(1));
    }

    #[tokio::test]
    async fn test_restore_backup_without_todos_field() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;
        let legacy = r#"{"customers":[{"id":1,"name":"C","use_as_default":false}],"projects":[],"issues":[],"time_entries":[]}"#;

        inner_restore_data("replace", legacy, &pool)
            .await
            .expect("legacy backup must restore");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "replace clears todos not in the backup");
    }

    #[tokio::test]
    async fn test_backup_restore_merge_keeps_existing() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;

        let json = inner_backup_data(&pool).await.expect("backup failed");

        // Add extra record not in the backup
        sqlx::query("INSERT INTO customers (Name, UseAsDefault) VALUES ('Extra Co', 0)")
            .execute(&pool)
            .await
            .unwrap();

        inner_restore_data("merge", &json, &pool)
            .await
            .expect("merge restore failed");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM customers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2, "merge should preserve existing records");
    }

    #[tokio::test]
    async fn test_startup_backup_retention_get_set_roundtrip() {
        let pool = crate::test_db().await;

        let default_retention = inner_get_startup_backup_retention(&pool)
            .await
            .expect("reading default retention should succeed");
        assert_eq!(default_retention, 3);

        let saved = inner_set_startup_backup_retention(7, &pool)
            .await
            .expect("setting retention to 7 should succeed");
        assert_eq!(saved, 7);

        let read_back = inner_get_startup_backup_retention(&pool)
            .await
            .expect("reading updated retention should succeed");
        assert_eq!(read_back, 7);

        let saved_zero = inner_set_startup_backup_retention(0, &pool)
            .await
            .expect("setting retention to 0 should succeed");
        assert_eq!(saved_zero, 0);

        let read_zero = inner_get_startup_backup_retention(&pool)
            .await
            .expect("reading retention of 0 should succeed");
        assert_eq!(read_zero, 0);
    }

    #[tokio::test]
    async fn test_list_startup_backups_reads_backup_directories() {
        use std::{
            fs,
            path::Path,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let test_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-artifacts")
            .join(format!("list-startup-backups-{unique}"));
        let db_path = test_dir.join("timetracker.db");

        crate::startup_backup::write_daily_backup(&db_path, "2026-05-08", r#"{"test":"older"}"#)
            .expect("older backup write should succeed");
        crate::startup_backup::write_daily_backup(&db_path, "2026-05-09", r#"{"test":"newer"}"#)
            .expect("newer backup write should succeed");

        let summaries = inner_list_startup_backups(&db_path)
            .await
            .expect("listing startup backups should succeed");

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].date, "2026-05-09");
        assert_eq!(summaries[0].size_bytes, r#"{"test":"newer"}"#.len() as u64);
        assert_eq!(summaries[1].date, "2026-05-08");
        assert_eq!(summaries[1].size_bytes, r#"{"test":"older"}"#.len() as u64);

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }
}
