use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

/// SQL aliases: ProjectId→project_id, ExternalId→external_id, Title→title
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub external_id: Option<String>,
    pub title: String,
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

pub async fn inner_list_issues(
    project_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<Vec<Issue>, AppError> {
    let rows = sqlx::query_as::<_, Issue>(
        "SELECT id, ProjectId as project_id, ExternalId as external_id, Title as title \
         FROM issues WHERE ProjectId = ? ORDER BY Title",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Resolves an issue identifier to an issue id — creates the issue if not found.
///
/// 5 code paths (all covered by named unit tests):
///   Path 5: identifier.trim().is_empty() → AppError::Custom("Issue is required")
///   Path 1: parses as i64 → ExternalId match found → return that id
///   Path 2: parses as i64 → ExternalId NOT found → fall to title search → found → return
///   Path 3: does NOT parse as i64 → title match found → return
///   Path 4: no match → INSERT INTO issues … RETURNING id → return new id
pub async fn inner_resolve_issue(
    project_id: i64,
    identifier: &str,
    pool: &Pool<Sqlite>,
) -> Result<i64, AppError> {
    // Path 5: empty → validation error
    if identifier.trim().is_empty() {
        return Err(AppError::Custom("Issue is required".to_string()));
    }

    // Paths 1 & 2: numeric identifier → try ExternalId lookup first
    if let Ok(ext_id) = identifier.trim().parse::<i64>() {
        let found: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM issues WHERE ExternalId = ? AND ProjectId = ?")
                .bind(ext_id.to_string()) // normalise e.g. "042" → "42"
                .bind(project_id)
                .fetch_optional(pool)
                .await?;

        if let Some((id,)) = found {
            return Ok(id); // Path 1: ExternalId found
        }
        // Path 2: ExternalId not found — fall through to title search below
    }

    // Path 3: title match within project
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM issues WHERE Title = ? AND ProjectId = ?")
            .bind(identifier.trim())
            .bind(project_id)
            .fetch_optional(pool)
            .await?;

    if let Some((id,)) = found {
        return Ok(id); // Path 3: title found
    }

    // Path 4: not found → auto-create
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, NULL, ?) RETURNING id",
    )
    .bind(project_id)
    .bind(identifier.trim())
    .fetch_one(pool)
    .await?;

    Ok(row.0) // Path 4: auto-created
}

// ── Tauri command wrapper ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_issues(
    project_id: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<Issue>, AppError> {
    inner_list_issues(project_id, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::customers::inner_create_customer;
    use crate::commands::projects::inner_create_project;

    /// Creates a customer + project and returns the project_id.
    async fn setup_project(pool: &Pool<Sqlite>) -> i64 {
        let c = inner_create_customer("Test Corp", pool).await.unwrap();
        let p = inner_create_project(c.id, "Test Project", pool)
            .await
            .unwrap();
        p.id
    }

    #[tokio::test]
    async fn test_resolve_issue_empty_identifier() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;
        let result = inner_resolve_issue(project_id, "", &pool).await;
        assert!(result.is_err(), "empty identifier must return Err");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Issue is required"),
            "error message must say 'Issue is required'"
        );
    }

    #[tokio::test]
    async fn test_resolve_issue_by_external_id_found() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;
        sqlx::query(
            "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, '42', 'Ticket 42')",
        )
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();

        let issue_id = inner_resolve_issue(project_id, "42", &pool).await.unwrap();

        // No duplicate should be created
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues WHERE ProjectId = ?")
            .bind(project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "Path 1 must not create a duplicate issue");
        assert!(issue_id > 0);
    }

    #[tokio::test]
    async fn test_resolve_issue_by_external_id_not_found() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;
        // Issue with Title "99" but ExternalId = NULL
        sqlx::query("INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, NULL, '99')")
            .bind(project_id)
            .execute(&pool)
            .await
            .unwrap();

        // "99" parses as i64; ExternalId lookup returns nothing; falls to title lookup → found
        let issue_id = inner_resolve_issue(project_id, "99", &pool).await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues WHERE ProjectId = ?")
            .bind(project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count.0, 1,
            "Path 2 title-fallback must not create a duplicate"
        );
        assert!(issue_id > 0);
    }

    #[tokio::test]
    async fn test_resolve_issue_by_title_found() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;
        sqlx::query(
            "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, NULL, 'Backend Work')",
        )
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();

        let issue_id = inner_resolve_issue(project_id, "Backend Work", &pool)
            .await
            .unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues WHERE ProjectId = ?")
            .bind(project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "Path 3 must not create a duplicate");
        assert!(issue_id > 0);
    }

    #[tokio::test]
    async fn test_resolve_issue_auto_creates() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;

        let issue_id = inner_resolve_issue(project_id, "Brand New Issue", &pool)
            .await
            .unwrap();

        // Verify the row was actually inserted
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM issues WHERE Title = ? AND ProjectId = ?")
                .bind("Brand New Issue")
                .bind(project_id)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(row.is_some(), "Path 4 must insert a new issue row");
        assert_eq!(
            row.unwrap().0,
            issue_id,
            "returned id must match the inserted row"
        );
    }
}
