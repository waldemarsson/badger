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
    #[serde(default = "default_todo_status")]
    pub status: String,
    #[serde(default)]
    pub position: i64,
    pub due_date: Option<String>,
    pub created_at: String,
    pub issue_id: Option<i64>,
    /// Legacy flag from the first todos format (before `status`); read-only.
    #[serde(default, skip_serializing)]
    #[sqlx(default)]
    pub done: bool,
}

/// A trip record as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupTravel {
    pub id: i64,
    pub date: String,
    pub from_place: String,
    pub to_place: String,
    pub km: Option<f64>,
    pub amount: Option<f64>,
}

/// A note folder as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct BackupFolder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub position: i64,
}

/// A note as stored in a backup file.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct BackupNote {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A reminder as stored in a backup file (`weekdays` as stored, e.g. `"1,4"`).
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct BackupReminder {
    pub id: i64,
    pub text: String,
    pub remind_at: String,
    pub time_of_day: String,
    pub repeat: String,
    pub weekdays: String,
    pub month_day: Option<i64>,
    pub state: String,
    pub todo_id: Option<i64>,
    pub note_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Shown when a restored reminder links a todo or note that is not in the backup.
const INVALID_REMINDER_REF: &str = "Backup is invalid: a reminder refers to a missing todo or note";

/// Shown when a restored folder or note points at a folder that is not in the backup.
const INVALID_FOLDER_REF: &str = "Backup is invalid: a folder or note refers to a missing folder";

/// Shown when restored folders are their own ancestors.
const FOLDER_CYCLE: &str = "Backup is invalid: folders form a cycle";

/// Shown when two restored notes have titles that differ only in case.
const DUPLICATE_NOTE_TITLE: &str = "Backup is invalid: duplicate note title";

fn default_todo_status() -> String {
    "not_started".to_string()
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
    /// Absent in backups made before travel existed.
    #[serde(default)]
    pub travel: Vec<BackupTravel>,
    /// Absent in backups made before notes existed.
    #[serde(default)]
    pub folders: Vec<BackupFolder>,
    /// Absent in backups made before notes existed.
    #[serde(default)]
    pub notes: Vec<BackupNote>,
    /// Absent in backups made before reminders existed.
    #[serde(default)]
    pub reminders: Vec<BackupReminder>,
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
        "SELECT id, ProjectId as project_id, Title as title FROM issues ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let time_entries = sqlx::query_as::<_, BackupTimeEntry>(
        "SELECT id, IssueId as issue_id, Date as date, Hours as hours, Notes as notes FROM time_entries ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let todos = sqlx::query_as::<_, BackupTodo>(
        "SELECT id, Title as title, Notes as notes, Status as status, Position as position, DueDate as due_date, CreatedAt as created_at, IssueId as issue_id FROM todos ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let travel = sqlx::query_as::<_, BackupTravel>(
        "SELECT id, Date as date, FromPlace as from_place, ToPlace as to_place, Km as km, Amount as amount FROM travel ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let folders = sqlx::query_as::<_, BackupFolder>(
        "SELECT id, ParentId as parent_id, Name as name, Position as position FROM folders ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let notes = sqlx::query_as::<_, BackupNote>(
        "SELECT id, FolderId as folder_id, Title as title, Body as body, Pinned as pinned, CreatedAt as created_at, UpdatedAt as updated_at FROM notes ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let reminders = sqlx::query_as::<_, BackupReminder>(
        "SELECT id, Text as text, RemindAt as remind_at, TimeOfDay as time_of_day, Repeat as repeat, Weekdays as weekdays, MonthDay as month_day, State as state, TodoId as todo_id, NoteId as note_id, CreatedAt as created_at, UpdatedAt as updated_at FROM reminders ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let backup = BackupData {
        customers,
        projects,
        issues,
        time_entries,
        todos,
        travel,
        folders,
        notes,
        reminders,
    };
    serde_json::to_string(&backup).map_err(|e| AppError::Custom(e.to_string()))
}

/// Replaces all data with the contents of a JSON backup.
///
/// Deletes everything first (customers cascade via FK; reminders, todos, travel, notes
/// and folders are cleared explicitly), then re-inserts the backup in one transaction.
pub async fn inner_restore_data(json: &str, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let mut data: BackupData = serde_json::from_str(json)
        .map_err(|e| AppError::Custom(format!("Invalid backup JSON: {}", e)))?;
    check_note_titles(&data.notes)?;
    check_folder_tree(&data.folders)?;
    suffix_duplicate_folder_names(&mut data.folders);
    check_reminder_links(&data)?;

    let mut tx = pool.begin().await?;

    // Single delete cascades to projects → issues → time_entries via ON DELETE CASCADE.
    sqlx::query("DELETE FROM customers")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM reminders")
        .execute(&mut *tx)
        .await?;
    // Todos are not reached by the cascade (their issue link is SET NULL).
    sqlx::query("DELETE FROM todos").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM travel").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM notes").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM folders").execute(&mut *tx).await?;

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
        sqlx::query("INSERT INTO issues (id, ProjectId, Title) VALUES (?, ?, ?)")
            .bind(i.id)
            .bind(i.project_id)
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
    insert_travel(&data.travel, &mut tx).await?;
    insert_folders_and_notes(&data.folders, &data.notes, &mut tx).await?;
    insert_reminders(&data.reminders, &mut tx).await?;
    // The search and link indexes are derived and not in the backup: rebuild them in the
    // same transaction.
    crate::commands::notes::rebuild_index(&mut tx).await?;

    tx.commit().await.map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
            AppError::Custom(INVALID_FOLDER_REF.to_string())
        }
        _ => AppError::Database(e),
    })?;
    Ok(())
}

/// Rejects reminders linking a todo or note that is not in the backup (foreign keys are
/// deferred during restore, so the commit error would blame folders instead).
fn check_reminder_links(data: &BackupData) -> Result<(), AppError> {
    use std::collections::HashSet;
    let todos: HashSet<i64> = data.todos.iter().map(|t| t.id).collect();
    let notes: HashSet<i64> = data.notes.iter().map(|n| n.id).collect();
    let dangling = data.reminders.iter().any(|r| {
        matches!(r.todo_id, Some(id) if !todos.contains(&id))
            || matches!(r.note_id, Some(id) if !notes.contains(&id))
    });
    if !dangling {
        Ok(())
    } else {
        Err(AppError::Custom(INVALID_REMINDER_REF.to_string()))
    }
}

/// Rejects case-insensitive duplicate note titles, folding like the notes commands do
/// (`to_lowercase`, so non-ASCII too); the NOCASE unique index only folds ASCII.
fn check_note_titles(notes: &[BackupNote]) -> Result<(), AppError> {
    let mut seen = std::collections::HashSet::new();
    if notes.iter().all(|n| seen.insert(n.title.to_lowercase())) {
        Ok(())
    } else {
        Err(AppError::Custom(DUPLICATE_NOTE_TITLE.to_string()))
    }
}

/// Rejects folders that are their own ancestor (a self-parent included). A parent that is
/// not in the backup ends the walk; the deferred FK check rejects it at commit.
fn check_folder_tree(folders: &[BackupFolder]) -> Result<(), AppError> {
    let parents: std::collections::HashMap<i64, Option<i64>> =
        folders.iter().map(|f| (f.id, f.parent_id)).collect();
    for f in folders {
        // A walk longer than the folder count must have revisited a folder.
        let mut current = f.parent_id;
        let mut steps = 0;
        while let Some(id) = current {
            steps += 1;
            if id == f.id || steps > parents.len() {
                return Err(AppError::Custom(FOLDER_CYCLE.to_string()));
            }
            current = parents.get(&id).copied().flatten();
        }
    }
    Ok(())
}

/// Makes sibling folder names unique case-insensitively, like the folder commands
/// require: in id order the first folder keeps a name and later duplicates get the
/// first ` 2`, ` 3`, … suffix free among all of their siblings' names.
fn suffix_duplicate_folder_names(folders: &mut [BackupFolder]) {
    use std::collections::{HashMap, HashSet};
    let mut taken: HashMap<Option<i64>, HashSet<String>> = HashMap::new();
    for f in folders.iter() {
        taken
            .entry(f.parent_id)
            .or_default()
            .insert(f.name.to_lowercase());
    }
    let mut order: Vec<usize> = (0..folders.len()).collect();
    order.sort_by_key(|&i| folders[i].id);
    let mut kept: HashMap<Option<i64>, HashSet<String>> = HashMap::new();
    for i in order {
        let f = &mut folders[i];
        let kept = kept.entry(f.parent_id).or_default();
        if kept.insert(f.name.to_lowercase()) {
            continue;
        }
        let taken = taken.entry(f.parent_id).or_default();
        f.name = crate::commands::folders::first_free_name(&f.name, taken);
        taken.insert(f.name.to_lowercase());
        kept.insert(f.name.to_lowercase());
    }
}

/// Inserts folders, then notes. Foreign keys are deferred to commit from here on, so a
/// child folder may come before its parent; earlier tables keep immediate checks, which
/// means a commit-time FK failure can only come from folders or notes.
async fn insert_folders_and_notes(
    folders: &[BackupFolder],
    notes: &[BackupNote],
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<(), AppError> {
    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut **tx)
        .await?;
    for f in folders {
        sqlx::query("INSERT INTO folders (id, ParentId, Name, Position) VALUES (?, ?, ?, ?)")
            .bind(f.id)
            .bind(f.parent_id)
            .bind(&f.name)
            .bind(f.position)
            .execute(&mut **tx)
            .await?;
    }
    for n in notes {
        sqlx::query(
            "INSERT INTO notes (id, FolderId, Title, Body, Pinned, CreatedAt, UpdatedAt) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(n.id)
        .bind(n.folder_id)
        .bind(&n.title)
        .bind(&n.body)
        .bind(n.pinned)
        .bind(&n.created_at)
        .bind(&n.updated_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Inserts todos. Runs after issues so `IssueId` references resolve.
async fn insert_todos(
    todos: &[BackupTodo],
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<(), AppError> {
    for t in todos {
        sqlx::query(
            "INSERT INTO todos (id, Title, Notes, Status, Position, DueDate, CreatedAt, IssueId) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(t.id)
        .bind(&t.title)
        .bind(&t.notes)
        .bind(if t.done { "done" } else { t.status.as_str() })
        .bind(t.position)
        .bind(&t.due_date)
        .bind(&t.created_at)
        .bind(t.issue_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Inserts reminders. Runs after todos and notes so their links resolve.
async fn insert_reminders(
    reminders: &[BackupReminder],
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<(), AppError> {
    for r in reminders {
        sqlx::query(
            "INSERT INTO reminders (id, Text, RemindAt, TimeOfDay, Repeat, Weekdays, MonthDay, State, TodoId, NoteId, CreatedAt, UpdatedAt) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(r.id)
        .bind(&r.text)
        .bind(&r.remind_at)
        .bind(&r.time_of_day)
        .bind(&r.repeat)
        .bind(&r.weekdays)
        .bind(r.month_day)
        .bind(&r.state)
        .bind(r.todo_id)
        .bind(r.note_id)
        .bind(&r.created_at)
        .bind(&r.updated_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Inserts trips. Travel has no foreign keys, so order does not matter.
async fn insert_travel(
    travel: &[BackupTravel],
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<(), AppError> {
    for t in travel {
        sqlx::query(
            "INSERT INTO travel (id, Date, FromPlace, ToPlace, Km, Amount) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(t.id)
        .bind(&t.date)
        .bind(&t.from_place)
        .bind(&t.to_place)
        .bind(t.km)
        .bind(t.amount)
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

/// Replaces all data with the contents of a JSON backup.
/// The semantic index is derived and not in backups: every restored note is re-embedded.
#[tauri::command]
pub async fn restore_data(
    json: String,
    app: tauri::AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, crate::semantic::SemanticState>,
) -> Result<(), AppError> {
    inner_restore_data(&json, &pool).await?;
    crate::commands::semantic::after_restore(semantic.inner(), pool.inner()).await;
    crate::reminders_scheduler::reminders_changed(&app, pool.inner()).await;
    Ok(())
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
pub async fn get_startup_backup_retention(pool: State<'_, Pool<Sqlite>>) -> Result<i64, AppError> {
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
        sqlx::query("INSERT INTO issues (id, ProjectId, Title) VALUES (1, 1, 'Test Issue')")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO time_entries (id, IssueId, Date, Hours, Notes) VALUES (1, 1, '2024-01-01', 2.5, NULL)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO todos (id, Title, Notes, Status, Position, DueDate, CreatedAt, IssueId) VALUES (1, 'Linked', 'n', 'in_progress', 3, '2024-02-01', '2024-01-01T00:00:00.000', 1), (2, 'Standalone', NULL, 'done', 0, NULL, '2024-01-02T00:00:00.000', NULL)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO travel (id, Date, FromPlace, ToPlace, Km, Amount) VALUES (1, '2024-01-05', 'Malmö', 'Karlskrona', 500.0, 1300.0), (2, '2024-01-10', 'Lund', 'Ystad', NULL, NULL)")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_backup_restore_replace_roundtrip() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;

        let json = inner_backup_data(&pool).await.expect("backup failed");

        inner_restore_data(&json, &pool)
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
        assert_eq!(result.todos[0].status, "in_progress");
        assert_eq!(result.todos[0].position, 3);
        assert_eq!(result.todos[1].status, "done");
        assert!(
            !json.contains("\"done\":"),
            "legacy flag must not be exported"
        );
    }

    #[tokio::test]
    async fn test_backup_restore_roundtrips_travel() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;
        let json = inner_backup_data(&pool).await.expect("backup failed");

        sqlx::query("DELETE FROM travel")
            .execute(&pool)
            .await
            .unwrap();
        inner_restore_data(&json, &pool)
            .await
            .expect("restore failed");

        let result: BackupData =
            serde_json::from_str(&inner_backup_data(&pool).await.unwrap()).unwrap();
        assert_eq!(result.travel.len(), 2);
        let t = &result.travel[0];
        assert_eq!(
            (t.date.as_str(), t.from_place.as_str(), t.to_place.as_str()),
            ("2024-01-05", "Malmö", "Karlskrona")
        );
        assert_eq!((t.km, t.amount), (Some(500.0), Some(1300.0)));
        assert_eq!((result.travel[1].km, result.travel[1].amount), (None, None));
    }

    async fn seed_notes(pool: &Pool<Sqlite>) {
        // Child folder 1 has a lower id than its parent 5; note 2 lives in the child.
        sqlx::raw_sql(
            "INSERT INTO folders (id, ParentId, Name, Position) VALUES (5, NULL, 'Work', 0);
             INSERT INTO folders (id, ParentId, Name, Position) VALUES (1, 5, 'Archive 🗄', 3);
             INSERT INTO notes (id, FolderId, Title, Body, Pinned, CreatedAt, UpdatedAt) VALUES
               (1, NULL, 'Åsa', '# Hi\r\n\n| a | b |\n|---|---|\ntrailing  \n', 1,
                '2024-01-01T00:00:00.000', '2024-01-02T00:00:00.000'),
               (2, 1, 'Plan 🎉', '', 0, '2024-01-03T00:00:00.000', '2024-01-03T00:00:00.000');",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_backup_restore_roundtrips_folders_and_notes() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;
        seed_notes(&pool).await;
        let json = inner_backup_data(&pool).await.expect("backup failed");
        let original: BackupData = serde_json::from_str(&json).unwrap();
        assert_eq!(original.folders.len(), 2);
        assert_eq!(original.notes.len(), 2);
        assert_eq!(
            original.notes[0].body,
            "# Hi\r\n\n| a | b |\n|---|---|\ntrailing  \n"
        );

        // Diverge, then restore into a fresh DB and over the modified one.
        sqlx::raw_sql(
            "DELETE FROM folders WHERE id = 5;
             UPDATE notes SET Body = 'changed' WHERE id = 1;
             INSERT INTO notes (Title, CreatedAt, UpdatedAt) VALUES ('Extra', 'x', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let fresh = crate::test_db().await;
        for target in [&pool, &fresh] {
            inner_restore_data(&json, target)
                .await
                .expect("restore failed");
            let result: BackupData =
                serde_json::from_str(&inner_backup_data(target).await.unwrap()).unwrap();
            assert_eq!(result.folders, original.folders);
            assert_eq!(result.notes, original.notes);
            let exported = serde_json::to_string(&(&result.folders, &result.notes)).unwrap();
            let expected = serde_json::to_string(&(&original.folders, &original.notes)).unwrap();
            assert_eq!(exported, expected, "byte-for-byte");
        }
    }

    #[tokio::test]
    async fn test_restore_rebuilds_search_index() {
        let pool = crate::test_db().await;
        seed_notes(&pool).await; // raw SQL: not indexed yet
        let json = inner_backup_data(&pool).await.unwrap();
        let ids = |q: &'static str, pool: Pool<Sqlite>| async move {
            crate::commands::notes::inner_search_notes(q, None, &pool)
                .await
                .unwrap()
                .into_iter()
                .map(|h| h.note.id)
                .collect::<Vec<i64>>()
        };
        let extra = crate::commands::notes::inner_create_note(None, &pool)
            .await
            .unwrap();
        crate::commands::notes::inner_rename_note(extra.id, "Stray", &pool)
            .await
            .unwrap();
        assert_eq!(ids("stray", pool.clone()).await, vec![extra.id]);

        inner_restore_data(&json, &pool).await.unwrap();
        assert!(
            ids("stray", pool.clone()).await.is_empty(),
            "old notes gone"
        );
        assert_eq!(ids("plan", pool.clone()).await, vec![2]);
        assert_eq!(ids("åsa", pool.clone()).await, vec![1]);
    }

    #[tokio::test]
    async fn test_restore_reembeds_all_notes_and_backup_excludes_vectors() {
        use crate::semantic::index::tests::ready_state;
        let pool = crate::test_db().await;
        seed_notes(&pool).await;
        let (state, fake) = ready_state(&pool).await;
        state.enqueue_all(&pool).await.unwrap();
        state.drain(&pool).await;
        let before: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT NoteId) FROM note_embeddings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(before > 0);

        let json = inner_backup_data(&pool).await.unwrap();
        assert!(!json.contains("embedding"), "vectors are not backed up");
        assert!(!json.contains("ChunkText") && !json.contains("chunk_text"));

        let embedded_before = fake.embedded().len();
        inner_restore_data(&json, &pool).await.unwrap();
        crate::commands::semantic::after_restore(&state, &pool).await;
        assert_eq!(state.indexed_notes(), 0, "cache cleared");
        state.drain(&pool).await;
        let (notes, embedded): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM notes), (SELECT COUNT(DISTINCT NoteId) FROM note_embeddings)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(embedded, notes, "every restored note re-embedded");
        assert_eq!(fake.embedded().len() - embedded_before, notes as usize);
        assert_eq!(state.indexed_notes() as i64, notes);
    }

    #[tokio::test]
    async fn test_restore_rebuilds_link_index() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO notes (id, Title, Body, CreatedAt, UpdatedAt) VALUES
               (1, 'Linker', 'see [[Target]] and #Acme/Web', 'x', 'x'),
               (2, 'Target', '', 'x', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap(); // raw SQL: no refs yet
        let json = inner_backup_data(&pool).await.unwrap();
        assert!(!json.contains("note_refs"), "derived, not backed up");
        sqlx::query("INSERT INTO note_refs (NoteId, Kind, Target) VALUES (2, 'tag', 'stale')")
            .execute(&pool)
            .await
            .unwrap();

        inner_restore_data(&json, &pool).await.unwrap();
        let refs: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT NoteId, Kind, Target FROM note_refs ORDER BY NoteId, Kind")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            refs,
            vec![
                (1, "link".into(), "target".into()),
                (1, "tag".into(), "acme/web".into())
            ]
        );
    }

    #[tokio::test]
    async fn test_restore_dangling_folder_reference_is_rejected() {
        let pool = crate::test_db().await;
        seed_notes(&pool).await;
        let dangling_parent = r#"{"customers":[],"projects":[],"issues":[],"time_entries":[],
            "folders":[{"id":1,"parent_id":99,"name":"Orphan","position":0}],"notes":[]}"#;
        let dangling_folder = r#"{"customers":[],"projects":[],"issues":[],"time_entries":[],
            "folders":[],"notes":[{"id":1,"folder_id":7,"title":"N","body":"",
            "pinned":false,"created_at":"x","updated_at":"x"}]}"#;

        for json in [dangling_parent, dangling_folder] {
            let err = inner_restore_data(json, &pool).await.unwrap_err();
            assert_eq!(err.to_string(), INVALID_FOLDER_REF);
        }

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2, "failed restore must roll back");
        let fk: (i64,) = sqlx::query_as("PRAGMA defer_foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fk.0, 0, "deferral must not leak past the transaction");
    }

    fn folders_backup(folders: &str) -> String {
        format!(
            r#"{{"customers":[],"projects":[],"issues":[],"time_entries":[],"folders":[{folders}],"notes":[]}}"#
        )
    }

    #[tokio::test]
    async fn test_restore_folder_cycle_is_rejected() {
        let pool = crate::test_db().await;
        seed_notes(&pool).await;
        let self_parent = folders_backup(r#"{"id":1,"parent_id":1,"name":"Loop","position":0}"#);
        let two_cycle = folders_backup(
            r#"{"id":1,"parent_id":2,"name":"A","position":0},
               {"id":2,"parent_id":1,"name":"B","position":0}"#,
        );
        let into_cycle = folders_backup(
            r#"{"id":1,"parent_id":null,"name":"Root","position":0},
               {"id":2,"parent_id":3,"name":"Hangs off","position":0},
               {"id":3,"parent_id":4,"name":"C","position":0},
               {"id":4,"parent_id":3,"name":"D","position":0}"#,
        );

        for json in [self_parent, two_cycle, into_cycle] {
            let err = inner_restore_data(&json, &pool).await.unwrap_err();
            assert_eq!(err.to_string(), FOLDER_CYCLE, "{json}");
        }

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM folders")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2, "rejected restore leaves the data alone");
    }

    #[test]
    fn check_folder_tree_accepts_trees_and_leaves_missing_parents_to_the_fk() {
        let folder = |id, parent_id| BackupFolder {
            id,
            parent_id,
            name: format!("F{id}"),
            position: 0,
        };
        // Child before parent, a deep chain and a missing parent are not cycles.
        let ok = vec![
            folder(1, Some(5)),
            folder(5, None),
            folder(2, Some(1)),
            folder(3, Some(2)),
            folder(4, Some(99)),
        ];
        assert!(check_folder_tree(&ok).is_ok());
        assert!(check_folder_tree(&[]).is_ok());
    }

    #[tokio::test]
    async fn test_restore_suffixes_duplicate_sibling_folder_names_in_id_order() {
        let pool = crate::test_db().await;
        let json = folders_backup(
            r#"{"id":3,"parent_id":null,"name":"Work","position":0},
               {"id":1,"parent_id":null,"name":"work","position":0},
               {"id":5,"parent_id":null,"name":"Work 2","position":0},
               {"id":7,"parent_id":3,"name":"Åsa","position":0},
               {"id":6,"parent_id":3,"name":"ÅSA","position":0},
               {"id":8,"parent_id":1,"name":"Åsa","position":0},
               {"id":9,"parent_id":null,"name":"Plain/Name","position":2}"#,
        );

        inner_restore_data(&json, &pool).await.unwrap();

        let result: BackupData =
            serde_json::from_str(&inner_backup_data(&pool).await.unwrap()).unwrap();
        let names: Vec<(i64, Option<i64>, &str, i64)> = result
            .folders
            .iter()
            .map(|f| (f.id, f.parent_id, f.name.as_str(), f.position))
            .collect();
        assert_eq!(
            names,
            vec![
                (1, None, "work", 0),
                (3, None, "Work 3", 0),
                (5, None, "Work 2", 0),
                (6, Some(3), "ÅSA", 0),
                (7, Some(3), "Åsa 2", 0),
                (8, Some(1), "Åsa", 0),
                (9, None, "Plain/Name", 2),
            ]
        );
    }

    #[tokio::test]
    async fn test_restore_case_duplicate_note_titles_is_rejected() {
        let pool = crate::test_db().await;
        seed_notes(&pool).await;
        let backup = |a: &str, b: &str| {
            format!(
                r#"{{"customers":[],"projects":[],"issues":[],"time_entries":[],"folders":[],"notes":[
                {{"id":1,"folder_id":null,"title":"{a}","body":"","pinned":false,"created_at":"x","updated_at":"x"}},
                {{"id":2,"folder_id":null,"title":"{b}","body":"","pinned":false,"created_at":"x","updated_at":"x"}}]}}"#
            )
        };

        // ASCII case (the unique index) and non-ASCII case (the app's Rust-side rule).
        for (a, b) in [("Plan", "plan"), ("Åsa", "åsa")] {
            let err = inner_restore_data(&backup(a, b), &pool).await.unwrap_err();
            assert_eq!(err.to_string(), DUPLICATE_NOTE_TITLE, "{a} / {b}");
        }

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2, "failed restore must roll back");
    }

    #[tokio::test]
    async fn test_restore_without_notes_fields_clears_notes() {
        let pool = crate::test_db().await;
        seed_notes(&pool).await;
        let legacy = r#"{"customers":[],"projects":[],"issues":[],"time_entries":[],"todos":[],"travel":[]}"#;

        inner_restore_data(legacy, &pool)
            .await
            .expect("legacy backup must restore");

        for table in ["notes", "folders"] {
            let count: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count.0, 0, "replace clears {table} not in the backup");
        }
    }

    #[tokio::test]
    async fn test_restore_legacy_todo_done_flag() {
        let pool = crate::test_db().await;
        let legacy = r#"{"customers":[],"projects":[],"issues":[],"time_entries":[],
            "todos":[{"id":1,"title":"Old","notes":null,"done":true,"due_date":null,"created_at":"x","issue_id":null},
                     {"id":2,"title":"Open","notes":null,"done":false,"due_date":null,"created_at":"x","issue_id":null}]}"#;

        inner_restore_data(legacy, &pool)
            .await
            .expect("legacy todos must restore");

        let rows: Vec<(String,)> = sqlx::query_as("SELECT Status FROM todos ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows, vec![("done".into(),), ("not_started".into(),)]);
    }

    #[tokio::test]
    async fn test_restore_backup_with_legacy_external_id() {
        let pool = crate::test_db().await;
        let legacy = r#"{"customers":[{"id":1,"name":"C","use_as_default":false}],
            "projects":[{"id":1,"customer_id":1,"name":"P","use_as_default":false}],
            "issues":[{"id":1,"project_id":1,"external_id":"42","title":"Old issue"}],
            "time_entries":[]}"#;

        inner_restore_data(legacy, &pool)
            .await
            .expect("backup with external_id must restore");

        let result: BackupData =
            serde_json::from_str(&inner_backup_data(&pool).await.unwrap()).unwrap();
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].title, "Old issue");
    }

    async fn seed_reminders(pool: &Pool<Sqlite>) {
        seed_test_data(pool).await;
        seed_notes(pool).await;
        sqlx::query(
            "INSERT INTO reminders (id, Text, RemindAt, TimeOfDay, Repeat, Weekdays, MonthDay, State, TodoId, NoteId, CreatedAt, UpdatedAt) VALUES \
             (1, 'Call Anna', '2026-11-09T09:00:00.000', '09:00', 'weekly', '1,4', NULL, 'pending', 1, NULL, '2026-11-01T00:00:00.000', '2026-11-01T00:00:00.000'), \
             (2, 'Read note', '2026-11-08T10:00:00.000', '10:00', 'monthly', '', 31, 'fired', NULL, 2, '2026-11-01T00:00:00.000', '2026-11-08T10:00:00.000'), \
             (3, 'Done one', '2026-11-07T08:00:00.000', '08:00', 'none', '', NULL, 'done', NULL, NULL, '2026-11-01T00:00:00.000', '2026-11-07T08:01:00.000')",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_backup_restore_roundtrips_reminders_with_links() {
        let pool = crate::test_db().await;
        seed_reminders(&pool).await;
        let json = inner_backup_data(&pool).await.expect("backup failed");
        let original: BackupData = serde_json::from_str(&json).unwrap();
        assert_eq!(original.reminders.len(), 3);

        sqlx::query("DELETE FROM reminders")
            .execute(&pool)
            .await
            .unwrap();
        inner_restore_data(&json, &pool)
            .await
            .expect("restore failed");
        let result: BackupData =
            serde_json::from_str(&inner_backup_data(&pool).await.unwrap()).unwrap();
        assert_eq!(result.reminders, original.reminders);
        assert_eq!(result.reminders[0].todo_id, Some(1));
        assert_eq!(result.reminders[1].note_id, Some(2));
        assert_eq!(result.reminders[0].weekdays, "1,4");
        assert_eq!(result.reminders[1].month_day, Some(31));
    }

    #[tokio::test]
    async fn test_restore_without_reminders_field_clears_reminders() {
        let pool = crate::test_db().await;
        seed_reminders(&pool).await;
        let legacy = r#"{"customers":[],"projects":[],"issues":[],"time_entries":[],"todos":[{"id":5,"title":"T","notes":null,"status":"not_started","position":0,"due_date":null,"created_at":"2024-01-01T00:00:00.000","issue_id":null}]}"#;
        inner_restore_data(legacy, &pool)
            .await
            .expect("backup without reminders must restore");
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reminders")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "replace clears reminders not in the backup");
    }

    #[tokio::test]
    async fn test_restore_reminder_with_missing_link_is_rejected() {
        let pool = crate::test_db().await;
        seed_reminders(&pool).await;
        let json = r#"{"customers":[],"projects":[],"issues":[],"time_entries":[],"reminders":[{"id":1,"text":"x","remind_at":"2026-11-09T09:00:00.000","time_of_day":"09:00","repeat":"none","weekdays":"","month_day":null,"state":"pending","todo_id":7,"note_id":null,"created_at":"c","updated_at":"u"}]}"#;
        let err = inner_restore_data(json, &pool).await.unwrap_err();
        assert_eq!(err.to_string(), INVALID_REMINDER_REF);
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reminders")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 3, "a rejected restore keeps the existing data");
    }

    #[tokio::test]
    async fn test_restore_backup_without_todos_field() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;
        let legacy = r#"{"customers":[{"id":1,"name":"C","use_as_default":false}],"projects":[],"issues":[],"time_entries":[]}"#;

        inner_restore_data(legacy, &pool)
            .await
            .expect("legacy backup must restore");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "replace clears todos not in the backup");
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM travel")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "replace clears travel not in the backup");
    }

    #[tokio::test]
    async fn test_restore_overwrites_existing_data() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;
        let json = inner_backup_data(&pool).await.expect("backup failed");

        sqlx::query("INSERT INTO customers (id, Name, UseAsDefault) VALUES (2, 'Extra Co', 0)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE customers SET Name = 'Renamed' WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO time_entries (id, IssueId, Date, Hours, Notes) VALUES (2, 1, '2024-01-02', 1.0, NULL)")
            .execute(&pool)
            .await
            .unwrap();

        inner_restore_data(&json, &pool)
            .await
            .expect("restore failed");

        let result: BackupData =
            serde_json::from_str(&inner_backup_data(&pool).await.unwrap()).unwrap();
        let customers: Vec<_> = result.customers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(customers, vec!["Test Co"]);
        let entries: Vec<_> = result.time_entries.iter().map(|e| e.id).collect();
        assert_eq!(entries, vec![1]);
        assert_eq!(result.todos.len(), 2);
        assert_eq!(result.todos[0].issue_id, Some(1));
    }

    #[tokio::test]
    async fn test_restore_invalid_json_keeps_data() {
        let pool = crate::test_db().await;
        seed_test_data(&pool).await;

        let err = inner_restore_data("not json", &pool).await.unwrap_err();
        assert!(err.to_string().starts_with("Invalid backup JSON"));

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM customers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
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
        let db_path = test_dir.join("badger.db");

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
