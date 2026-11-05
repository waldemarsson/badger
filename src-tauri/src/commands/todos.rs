use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

use super::issues::inner_resolve_issue;

/// Todo with optional issue context (LEFT JOINs — all link fields are null when unlinked).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TodoRow {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub done: bool,
    pub due_date: Option<String>, // YYYY-MM-DD
    pub created_at: String,
    pub issue_id: Option<i64>,
    pub issue_title: Option<String>,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub customer_id: Option<i64>,
    pub customer_name: Option<String>,
}

const SELECT_TODOS: &str = "SELECT t.id,
        t.Title     AS title,
        t.Notes     AS notes,
        t.Done      AS done,
        t.DueDate   AS due_date,
        t.CreatedAt AS created_at,
        i.id        AS issue_id,
        i.Title     AS issue_title,
        p.id        AS project_id,
        p.Name      AS project_name,
        c.id        AS customer_id,
        c.Name      AS customer_name
 FROM todos t
 LEFT JOIN issues    i ON i.id = t.IssueId
 LEFT JOIN projects  p ON p.id = i.ProjectId
 LEFT JOIN customers c ON c.id = p.CustomerId";

// ── Validation helpers ────────────────────────────────────────────────────────

fn clean_title(title: &str) -> Result<String, AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError::Custom("Title is required".to_string()));
    }
    Ok(t.to_string())
}

fn clean_notes(notes: Option<&str>) -> Option<String> {
    notes
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

fn clean_due_date(due_date: Option<&str>) -> Result<Option<String>, AppError> {
    match due_date.map(str::trim).filter(|d| !d.is_empty()) {
        None => Ok(None),
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map(|_| Some(d.to_string()))
            .map_err(|_| AppError::Custom("Invalid due date".to_string())),
    }
}

async fn ensure_issue_exists(issue_id: Option<i64>, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    if let Some(id) = issue_id {
        let found: Option<(i64,)> = sqlx::query_as("SELECT id FROM issues WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        if found.is_none() {
            return Err(AppError::Custom("Issue not found".to_string()));
        }
    }
    Ok(())
}

fn not_found() -> AppError {
    AppError::Custom("Todo not found".to_string())
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// Open todos first, then by due date (no due date last), then oldest first.
pub async fn inner_list_todos(
    include_done: bool,
    pool: &Pool<Sqlite>,
) -> Result<Vec<TodoRow>, AppError> {
    let sql = format!(
        "{SELECT_TODOS} WHERE (? OR t.Done = 0) \
         ORDER BY t.Done, t.DueDate IS NULL, t.DueDate, t.CreatedAt, t.id"
    );
    let rows = sqlx::query_as::<_, TodoRow>(&sql)
        .bind(include_done)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn inner_get_todo(id: i64, pool: &Pool<Sqlite>) -> Result<TodoRow, AppError> {
    let sql = format!("{SELECT_TODOS} WHERE t.id = ?");
    sqlx::query_as::<_, TodoRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(not_found)
}

pub async fn inner_create_todo(
    title: &str,
    notes: Option<&str>,
    due_date: Option<&str>,
    issue_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<TodoRow, AppError> {
    let title = clean_title(title)?;
    let due_date = clean_due_date(due_date)?;
    ensure_issue_exists(issue_id, pool).await?;
    let created_at = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string();

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO todos (Title, Notes, Done, DueDate, CreatedAt, IssueId) \
         VALUES (?, ?, 0, ?, ?, ?) RETURNING id",
    )
    .bind(&title)
    .bind(clean_notes(notes))
    .bind(&due_date)
    .bind(&created_at)
    .bind(issue_id)
    .fetch_one(pool)
    .await?;

    inner_get_todo(row.0, pool).await
}

/// Replaces title, notes, due date, and issue link. `issue_id = None` unlinks.
pub async fn inner_update_todo(
    id: i64,
    title: &str,
    notes: Option<&str>,
    due_date: Option<&str>,
    issue_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<TodoRow, AppError> {
    let title = clean_title(title)?;
    let due_date = clean_due_date(due_date)?;
    ensure_issue_exists(issue_id, pool).await?;

    let result =
        sqlx::query("UPDATE todos SET Title = ?, Notes = ?, DueDate = ?, IssueId = ? WHERE id = ?")
            .bind(&title)
            .bind(clean_notes(notes))
            .bind(&due_date)
            .bind(issue_id)
            .bind(id)
            .execute(pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    inner_get_todo(id, pool).await
}

pub async fn inner_set_todo_done(
    id: i64,
    done: bool,
    pool: &Pool<Sqlite>,
) -> Result<TodoRow, AppError> {
    let result = sqlx::query("UPDATE todos SET Done = ? WHERE id = ?")
        .bind(done)
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    inner_get_todo(id, pool).await
}

pub async fn inner_delete_todo(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM todos WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(())
}

/// Creates (or reuses a same-title / same-ExternalId) issue in `project_id` and links the todo.
pub async fn inner_create_issue_for_todo(
    todo_id: i64,
    project_id: i64,
    issue_title: &str,
    pool: &Pool<Sqlite>,
) -> Result<TodoRow, AppError> {
    inner_get_todo(todo_id, pool).await?; // fail before creating an issue
    let issue_id = inner_resolve_issue(project_id, issue_title, pool).await?;
    sqlx::query("UPDATE todos SET IssueId = ? WHERE id = ?")
        .bind(issue_id)
        .bind(todo_id)
        .execute(pool)
        .await?;
    inner_get_todo(todo_id, pool).await
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_todos(
    include_done: bool,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<TodoRow>, AppError> {
    inner_list_todos(include_done, pool.inner()).await
}

#[tauri::command]
pub async fn create_todo(
    title: String,
    notes: Option<String>,
    due_date: Option<String>,
    issue_id: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TodoRow, AppError> {
    inner_create_todo(
        &title,
        notes.as_deref(),
        due_date.as_deref(),
        issue_id,
        pool.inner(),
    )
    .await
}

#[tauri::command]
pub async fn update_todo(
    id: i64,
    title: String,
    notes: Option<String>,
    due_date: Option<String>,
    issue_id: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TodoRow, AppError> {
    inner_update_todo(
        id,
        &title,
        notes.as_deref(),
        due_date.as_deref(),
        issue_id,
        pool.inner(),
    )
    .await
}

#[tauri::command]
pub async fn set_todo_done(
    id: i64,
    done: bool,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TodoRow, AppError> {
    inner_set_todo_done(id, done, pool.inner()).await
}

#[tauri::command]
pub async fn delete_todo(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_todo(id, pool.inner()).await
}

#[tauri::command]
pub async fn create_issue_for_todo(
    todo_id: i64,
    project_id: i64,
    issue_title: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TodoRow, AppError> {
    inner_create_issue_for_todo(todo_id, project_id, &issue_title, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::customers::inner_create_customer;
    use crate::commands::projects::inner_create_project;

    /// Returns (customer_id, project_id).
    async fn setup_project(pool: &Pool<Sqlite>) -> (i64, i64) {
        let c = inner_create_customer("Acme", pool).await.unwrap();
        let p = inner_create_project(c.id, "Web", pool).await.unwrap();
        (c.id, p.id)
    }

    fn err_msg<T: std::fmt::Debug>(r: Result<T, AppError>) -> String {
        r.unwrap_err().to_string()
    }

    #[tokio::test]
    async fn create_standalone_todo_trims_and_nulls_empty_fields() {
        let pool = crate::test_db().await;
        let t = inner_create_todo("  Buy milk ü  ", Some("   "), Some(""), None, &pool)
            .await
            .unwrap();
        assert_eq!(t.title, "Buy milk ü");
        assert_eq!(t.notes, None);
        assert_eq!(t.due_date, None);
        assert!(!t.done);
        assert_eq!(t.issue_id, None);
        assert_eq!(t.project_name, None);
    }

    #[tokio::test]
    async fn create_rejects_blank_title_and_bad_date_and_unknown_issue() {
        let pool = crate::test_db().await;
        assert!(
            err_msg(inner_create_todo("  ", None, None, None, &pool).await)
                .contains("Title is required")
        );
        assert!(
            err_msg(inner_create_todo("x", None, Some("2026-13-01"), None, &pool).await)
                .contains("Invalid due date")
        );
        assert!(
            err_msg(inner_create_todo("x", None, None, Some(999), &pool).await)
                .contains("Issue not found")
        );
    }

    #[tokio::test]
    async fn update_edits_fields_links_and_unlinks() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup_project(&pool).await;
        let issue_id = inner_resolve_issue(project_id, "Login bug", &pool)
            .await
            .unwrap();
        let t = inner_create_todo("a", None, None, None, &pool)
            .await
            .unwrap();

        let t = inner_update_todo(
            t.id,
            "b",
            Some("note"),
            Some("2026-12-24"),
            Some(issue_id),
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(t.title, "b");
        assert_eq!(t.notes.as_deref(), Some("note"));
        assert_eq!(t.due_date.as_deref(), Some("2026-12-24"));
        assert_eq!(t.issue_title.as_deref(), Some("Login bug"));
        assert_eq!(t.project_name.as_deref(), Some("Web"));
        assert_eq!(t.customer_name.as_deref(), Some("Acme"));

        let t = inner_update_todo(t.id, "b", None, None, None, &pool)
            .await
            .unwrap();
        assert_eq!(t.issue_id, None);
        assert_eq!(t.due_date, None);
    }

    #[tokio::test]
    async fn missing_todo_reports_not_found() {
        let pool = crate::test_db().await;
        assert!(
            err_msg(inner_update_todo(42, "x", None, None, None, &pool).await)
                .contains("Todo not found")
        );
        assert!(err_msg(inner_set_todo_done(42, true, &pool).await).contains("Todo not found"));
        assert!(err_msg(inner_delete_todo(42, &pool).await).contains("Todo not found"));
        assert!(
            err_msg(inner_create_issue_for_todo(42, 1, "x", &pool).await)
                .contains("Todo not found")
        );
    }

    #[tokio::test]
    async fn done_toggle_and_delete() {
        let pool = crate::test_db().await;
        let t = inner_create_todo("a", None, None, None, &pool)
            .await
            .unwrap();
        assert!(inner_set_todo_done(t.id, true, &pool).await.unwrap().done);
        assert!(!inner_set_todo_done(t.id, false, &pool).await.unwrap().done);
        inner_delete_todo(t.id, &pool).await.unwrap();
        assert!(inner_list_todos(true, &pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_issue_for_todo_creates_then_reuses() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup_project(&pool).await;
        let a = inner_create_todo("Fix login", None, None, None, &pool)
            .await
            .unwrap();
        let b = inner_create_todo("Other", None, None, None, &pool)
            .await
            .unwrap();

        let a = inner_create_issue_for_todo(a.id, project_id, "Fix login", &pool)
            .await
            .unwrap();
        let b = inner_create_issue_for_todo(b.id, project_id, "Fix login", &pool)
            .await
            .unwrap();
        assert!(a.issue_id.is_some());
        assert_eq!(a.issue_id, b.issue_id, "same title must reuse the issue");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);

        assert!(
            err_msg(inner_create_issue_for_todo(a.id, project_id, " ", &pool).await)
                .contains("Issue is required")
        );
    }

    #[tokio::test]
    async fn deleting_customer_keeps_todo_and_clears_link() {
        let pool = crate::test_db().await;
        let (customer_id, project_id) = setup_project(&pool).await;
        let t = inner_create_todo("a", None, None, None, &pool)
            .await
            .unwrap();
        inner_create_issue_for_todo(t.id, project_id, "Issue", &pool)
            .await
            .unwrap();

        sqlx::query("DELETE FROM customers WHERE id = ?")
            .bind(customer_id)
            .execute(&pool)
            .await
            .unwrap();

        let t = inner_get_todo(t.id, &pool).await.unwrap();
        assert_eq!(t.issue_id, None);
        assert_eq!(t.title, "a");
    }

    #[tokio::test]
    async fn list_orders_and_filters() {
        let pool = crate::test_db().await;
        let no_due = inner_create_todo("no due", None, None, None, &pool)
            .await
            .unwrap();
        let late = inner_create_todo("late", None, Some("2026-12-31"), None, &pool)
            .await
            .unwrap();
        let early = inner_create_todo("early", None, Some("2026-01-01"), None, &pool)
            .await
            .unwrap();
        let done = inner_create_todo("done", None, Some("2025-01-01"), None, &pool)
            .await
            .unwrap();
        inner_set_todo_done(done.id, true, &pool).await.unwrap();

        let open: Vec<i64> = inner_list_todos(false, &pool)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(open, vec![early.id, late.id, no_due.id]);

        let all: Vec<i64> = inner_list_todos(true, &pool)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(all, vec![early.id, late.id, no_due.id, done.id]);
    }
}
