use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

use super::issues::inner_resolve_issue;

/// Fully-joined time entry row — includes denormalized names for display.
/// The time_entries table only stores IssueId; names come from the JOIN.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TimeEntryRow {
    pub id: i64,
    pub date: String, // YYYY-MM-DD
    pub hours: f64,
    pub notes: Option<String>,
    pub issue_id: i64,
    pub issue_title: String,
    pub project_id: i64,
    pub project_name: String,
    pub customer_id: i64,
    pub customer_name: String,
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

pub async fn inner_list_time_entries(
    start_date: &str,
    end_date: &str,
    pool: &Pool<Sqlite>,
) -> Result<Vec<TimeEntryRow>, AppError> {
    let rows = sqlx::query_as::<_, TimeEntryRow>(
        "SELECT te.id,
                te.Date      AS date,
                te.Hours     AS hours,
                te.Notes     AS notes,
                i.id         AS issue_id,
                i.Title      AS issue_title,
                p.id         AS project_id,
                p.Name       AS project_name,
                c.id         AS customer_id,
                c.Name       AS customer_name
         FROM time_entries te
         JOIN issues   i ON i.id = te.IssueId
         JOIN projects p ON p.id = i.ProjectId
         JOIN customers c ON c.id = p.CustomerId
         WHERE te.Date >= ? AND te.Date <= ?
         ORDER BY te.Date DESC, te.id DESC",
    )
    .bind(start_date)
    .bind(end_date)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn inner_create_time_entry(
    project_id: i64,
    issue_identifier: &str,
    date: &str,
    hours: f64,
    notes: Option<&str>,
    pool: &Pool<Sqlite>,
) -> Result<TimeEntryRow, AppError> {
    let issue_id = inner_resolve_issue(project_id, issue_identifier, pool).await?;

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO time_entries (IssueId, Date, Hours, Notes) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(issue_id)
    .bind(date)
    .bind(hours)
    .bind(notes)
    .fetch_one(pool)
    .await?;

    // Re-fetch with JOIN to return display row
    let entries = inner_list_time_entries(date, date, pool).await?;
    entries
        .into_iter()
        .find(|e| e.id == row.0)
        .ok_or_else(|| AppError::Custom("Failed to fetch created entry".to_string()))
}

pub async fn inner_update_time_entry(
    id: i64,
    project_id: i64,
    issue_identifier: &str,
    date: &str,
    hours: f64,
    notes: Option<&str>,
    pool: &Pool<Sqlite>,
) -> Result<TimeEntryRow, AppError> {
    let issue_id = inner_resolve_issue(project_id, issue_identifier, pool).await?;

    sqlx::query("UPDATE time_entries SET IssueId = ?, Date = ?, Hours = ?, Notes = ? WHERE id = ?")
        .bind(issue_id)
        .bind(date)
        .bind(hours)
        .bind(notes)
        .bind(id)
        .execute(pool)
        .await?;

    // Re-fetch with JOIN to return updated display row
    let entries = inner_list_time_entries(date, date, pool).await?;
    entries
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::Custom("Failed to fetch updated entry".to_string()))
}

pub async fn inner_delete_time_entry(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    sqlx::query("DELETE FROM time_entries WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_time_entries(
    start_date: String,
    end_date: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<TimeEntryRow>, AppError> {
    inner_list_time_entries(&start_date, &end_date, pool.inner()).await
}

#[tauri::command]
pub async fn create_time_entry(
    project_id: i64,
    issue_identifier: String,
    date: String,
    hours: f64,
    notes: Option<String>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TimeEntryRow, AppError> {
    inner_create_time_entry(
        project_id,
        &issue_identifier,
        &date,
        hours,
        notes.as_deref(),
        pool.inner(),
    )
    .await
}

#[tauri::command]
pub async fn update_time_entry(
    id: i64,
    project_id: i64,
    issue_identifier: String,
    date: String,
    hours: f64,
    notes: Option<String>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TimeEntryRow, AppError> {
    inner_update_time_entry(
        id,
        project_id,
        &issue_identifier,
        &date,
        hours,
        notes.as_deref(),
        pool.inner(),
    )
    .await
}

#[tauri::command]
pub async fn delete_time_entry(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_time_entry(id, pool.inner()).await
}

// ── count_project_time_entries (used by frontend delete guard) ────────────────

pub async fn inner_count_project_time_entries(
    project_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM time_entries te
         JOIN issues i ON i.id = te.IssueId
         WHERE i.ProjectId = ?",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

#[tauri::command]
pub async fn count_project_time_entries(
    project_id: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<i64, AppError> {
    inner_count_project_time_entries(project_id, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::customers::inner_create_customer;
    use crate::commands::issues::inner_list_issues;
    use crate::commands::projects::inner_create_project;

    async fn setup(pool: &Pool<Sqlite>) -> (i64, i64) {
        let c = inner_create_customer("Test Corp", pool).await.unwrap();
        let p = inner_create_project(c.id, "Test Project", pool)
            .await
            .unwrap();
        (c.id, p.id)
    }

    #[tokio::test]
    async fn test_create_time_entry() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup(&pool).await;

        let entry =
            inner_create_time_entry(project_id, "New Issue", "2026-04-07", 2.5, None, &pool)
                .await
                .unwrap();

        assert_eq!(entry.hours, 2.5);
        assert_eq!(entry.issue_title, "New Issue");
        assert_eq!(entry.project_id, project_id);

        // Issue must have been auto-created (New path of inner_resolve_issue)
        let issues = inner_list_issues(project_id, &pool).await.unwrap();
        assert_eq!(issues.len(), 1, "issue must be auto-created");
        assert_eq!(issues[0].title, "New Issue");
    }

    #[tokio::test]
    async fn test_list_time_entries() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup(&pool).await;

        inner_create_time_entry(project_id, "Issue A", "2026-04-07", 1.0, None, &pool)
            .await
            .unwrap();
        inner_create_time_entry(
            project_id,
            "Issue B",
            "2026-04-08",
            2.0,
            Some("note"),
            &pool,
        )
        .await
        .unwrap();

        let in_range = inner_list_time_entries("2026-04-07", "2026-04-08", &pool)
            .await
            .unwrap();
        assert_eq!(in_range.len(), 2, "both entries must appear within range");

        let out_range = inner_list_time_entries("2026-04-01", "2026-04-06", &pool)
            .await
            .unwrap();
        assert!(
            out_range.is_empty(),
            "date filter must exclude out-of-range entries"
        );
    }

    #[tokio::test]
    async fn test_update_time_entry() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup(&pool).await;

        let entry =
            inner_create_time_entry(project_id, "Original Issue", "2026-04-07", 1.0, None, &pool)
                .await
                .unwrap();

        let updated = inner_update_time_entry(
            entry.id,
            project_id,
            "Updated Issue",
            "2026-04-07",
            3.0,
            Some("updated note"),
            &pool,
        )
        .await
        .unwrap();

        assert_eq!(updated.hours, 3.0);
        assert_eq!(updated.issue_title, "Updated Issue");
        assert_eq!(updated.notes.as_deref(), Some("updated note"));
    }

    #[tokio::test]
    async fn test_delete_time_entry() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup(&pool).await;

        let entry = inner_create_time_entry(
            project_id,
            "Deletable Issue",
            "2026-04-07",
            1.0,
            None,
            &pool,
        )
        .await
        .unwrap();

        inner_delete_time_entry(entry.id, &pool).await.unwrap();

        let entries = inner_list_time_entries("2026-04-07", "2026-04-07", &pool)
            .await
            .unwrap();
        assert!(entries.is_empty(), "entry must be gone after delete");
    }
}
