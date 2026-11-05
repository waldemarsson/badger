use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

/// SQL aliases: ProjectId→project_id, Title→title
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

pub async fn inner_list_issues(
    project_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<Vec<Issue>, AppError> {
    let rows = sqlx::query_as::<_, Issue>(
        "SELECT id, ProjectId as project_id, Title as title \
         FROM issues WHERE ProjectId = ? ORDER BY Title",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Resolves an issue title to an issue id — creates the issue if not found.
///
/// 3 code paths (all covered by named unit tests):
///   Empty: identifier.trim().is_empty() → AppError::Custom("Issue is required")
///   Found: title match within project → return that id
///   New:   no match → INSERT INTO issues … RETURNING id → return new id
pub async fn inner_resolve_issue(
    project_id: i64,
    identifier: &str,
    pool: &Pool<Sqlite>,
) -> Result<i64, AppError> {
    if identifier.trim().is_empty() {
        return Err(AppError::Custom("Issue is required".to_string()));
    }

    let found: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM issues WHERE Title = ? AND ProjectId = ?")
            .bind(identifier.trim())
            .bind(project_id)
            .fetch_optional(pool)
            .await?;

    if let Some((id,)) = found {
        return Ok(id);
    }

    let row: (i64,) =
        sqlx::query_as("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(project_id)
            .bind(identifier.trim())
            .fetch_one(pool)
            .await?;

    Ok(row.0)
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
    async fn test_resolve_issue_numeric_title_found() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;
        sqlx::query("INSERT INTO issues (ProjectId, Title) VALUES (?, '99')")
            .bind(project_id)
            .execute(&pool)
            .await
            .unwrap();

        let issue_id = inner_resolve_issue(project_id, "99", &pool).await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues WHERE ProjectId = ?")
            .bind(project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count.0, 1,
            "numeric title must match, not create a duplicate"
        );
        assert!(issue_id > 0);
    }

    #[tokio::test]
    async fn test_resolve_issue_by_title_found() {
        let pool = crate::test_db().await;
        let project_id = setup_project(&pool).await;
        sqlx::query("INSERT INTO issues (ProjectId, Title) VALUES (?, 'Backend Work')")
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
        assert_eq!(count.0, 1, "title match must not create a duplicate");
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
        assert!(row.is_some(), "unmatched title must insert a new issue row");
        assert_eq!(
            row.unwrap().0,
            issue_id,
            "returned id must match the inserted row"
        );
    }
}
