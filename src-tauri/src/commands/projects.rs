// src-tauri/src/commands/projects.rs
use crate::commands::customers::{clean_name, is_duplicate};
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

/// A project row returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Project {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub use_as_default: bool,
}

/// Fails if another project of `customer_id` (other than `exclude_id`) already has this name.
async fn ensure_unique(
    customer_id: i64,
    name: &str,
    exclude_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT Name FROM projects WHERE CustomerId = ? AND id != ?")
            .bind(customer_id)
            .bind(exclude_id)
            .fetch_all(pool)
            .await?;
    if is_duplicate(name, &names) {
        return Err(AppError::Custom(format!(
            "A project named {name} already exists"
        )));
    }
    Ok(())
}

// ── Inner functions (testable without a Tauri app) ────────────────────────────

pub async fn inner_list_projects(
    customer_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<Vec<Project>, AppError> {
    let rows = sqlx::query_as::<_, Project>(
        "SELECT id, CustomerId as customer_id, Name as name, UseAsDefault as use_as_default \
         FROM projects \
         WHERE CustomerId = ? \
         ORDER BY Name",
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn inner_create_project(
    customer_id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
) -> Result<Project, AppError> {
    let name = clean_name(name)?;
    ensure_unique(customer_id, &name, 0, pool).await?;
    let row = sqlx::query_as::<_, Project>(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, 0) \
         RETURNING id, CustomerId as customer_id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(customer_id)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn inner_update_project(
    id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
) -> Result<Project, AppError> {
    let name = clean_name(name)?;
    let customer_id: i64 = sqlx::query_scalar("SELECT CustomerId FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::Custom("Project not found".to_string()))?;
    ensure_unique(customer_id, &name, id, pool).await?;
    let row = sqlx::query_as::<_, Project>(
        "UPDATE projects SET Name = ? WHERE id = ? \
         RETURNING id, CustomerId as customer_id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(name)
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn inner_delete_project(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM time_entries te \
         JOIN issues i ON i.id = te.IssueId \
         WHERE i.ProjectId = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    if count > 0 {
        return Err(AppError::Custom(format!(
            "Has {} time {} — delete them first",
            count,
            if count == 1 { "entry" } else { "entries" }
        )));
    }
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Clears UseAsDefault for all projects belonging to the same customer as `project_id`,
/// then sets UseAsDefault = 1 on `project_id`.  Runs in a single transaction.
///
/// Scoping rule (BE-08): only one project per customer can be default.
/// Projects belonging to different customers are unaffected.
pub async fn inner_set_default_project(
    project_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;

    // Clear defaults only within the same customer — other customers' projects untouched
    sqlx::query(
        "UPDATE projects SET UseAsDefault = 0 \
         WHERE CustomerId = (SELECT CustomerId FROM projects WHERE id = ?)",
    )
    .bind(project_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE projects SET UseAsDefault = 1 WHERE id = ?")
        .bind(project_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_projects(
    customer_id: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<Project>, AppError> {
    inner_list_projects(customer_id, pool.inner()).await
}

#[tauri::command]
pub async fn create_project(
    customer_id: i64,
    name: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Project, AppError> {
    inner_create_project(customer_id, &name, pool.inner()).await
}

#[tauri::command]
pub async fn update_project(
    id: i64,
    name: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Project, AppError> {
    inner_update_project(id, &name, pool.inner()).await
}

#[tauri::command]
pub async fn delete_project(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_project(id, pool.inner()).await
}

#[tauri::command]
pub async fn set_default_project(
    project_id: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<(), AppError> {
    inner_set_default_project(project_id, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::customers::inner_create_customer;

    #[tokio::test]
    async fn test_create_and_list_projects() {
        let pool = crate::test_db().await;

        let customer = inner_create_customer("Acme Corp", &pool).await.unwrap();
        let project = inner_create_project(customer.id, "Website Redesign", &pool)
            .await
            .unwrap();

        assert_eq!(project.name, "Website Redesign");
        assert_eq!(project.customer_id, customer.id);
        assert!(
            !project.use_as_default,
            "newly created project must not be default"
        );

        let projects = inner_list_projects(customer.id, &pool).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Website Redesign");
    }

    #[tokio::test]
    async fn test_delete_project() {
        let pool = crate::test_db().await;

        let customer = inner_create_customer("Acme Corp", &pool).await.unwrap();
        let project = inner_create_project(customer.id, "To Delete", &pool)
            .await
            .unwrap();

        inner_delete_project(project.id, &pool).await.unwrap();

        let projects = inner_list_projects(customer.id, &pool).await.unwrap();
        assert!(projects.is_empty(), "project should be gone after delete");
    }

    #[tokio::test]
    async fn test_set_default_project_per_customer() {
        let pool = crate::test_db().await;

        // Two separate customers — each gets two projects
        let c1 = inner_create_customer("Alpha Corp", &pool).await.unwrap();
        let c2 = inner_create_customer("Beta LLC", &pool).await.unwrap();

        let p1a = inner_create_project(c1.id, "Alpha P1", &pool)
            .await
            .unwrap();
        let p1b = inner_create_project(c1.id, "Alpha P2", &pool)
            .await
            .unwrap();
        let p2a = inner_create_project(c2.id, "Beta P1", &pool).await.unwrap();
        let _p2b = inner_create_project(c2.id, "Beta P2", &pool).await.unwrap();

        // Set default for c1 → p1a, then change to p1b
        inner_set_default_project(p1a.id, &pool).await.unwrap();
        inner_set_default_project(p1b.id, &pool).await.unwrap();

        // Set default for c2 → p2a
        inner_set_default_project(p2a.id, &pool).await.unwrap();

        // c1: exactly 1 default, must be p1b
        let c1_projects = inner_list_projects(c1.id, &pool).await.unwrap();
        let c1_defaults: Vec<_> = c1_projects.iter().filter(|p| p.use_as_default).collect();
        assert_eq!(
            c1_defaults.len(),
            1,
            "c1 must have exactly one default project"
        );
        assert_eq!(c1_defaults[0].id, p1b.id, "c1's default must be p1b");

        // c2: exactly 1 default, must be p2a (unaffected by c1 operations)
        let c2_projects = inner_list_projects(c2.id, &pool).await.unwrap();
        let c2_defaults: Vec<_> = c2_projects.iter().filter(|p| p.use_as_default).collect();
        assert_eq!(
            c2_defaults.len(),
            1,
            "c2 must have exactly one default project"
        );
        assert_eq!(c2_defaults[0].id, p2a.id, "c2's default must be p2a");
    }

    #[tokio::test]
    async fn test_delete_customer_guard() {
        let pool = crate::test_db().await;

        let customer = inner_create_customer("Acme Corp", &pool).await.unwrap();
        inner_create_project(customer.id, "Project A", &pool)
            .await
            .unwrap();
        inner_create_project(customer.id, "Project B", &pool)
            .await
            .unwrap();

        // Delete guard: cannot delete a customer that has projects
        let result = crate::commands::customers::inner_delete_customer(customer.id, &pool).await;
        assert!(
            result.is_err(),
            "Should block delete when customer has projects"
        );
        assert!(result.unwrap_err().to_string().contains("Has 2 projects"));

        // After deleting projects manually, customer delete should succeed
        let projects = inner_list_projects(customer.id, &pool).await.unwrap();
        for p in projects {
            inner_delete_project(p.id, &pool).await.unwrap();
        }
        crate::commands::customers::inner_delete_customer(customer.id, &pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_project_validates_name() {
        let pool = crate::test_db().await;

        let c1 = inner_create_customer("Acme", &pool).await.unwrap();
        let c2 = inner_create_customer("Beta", &pool).await.unwrap();

        let p = inner_create_project(c1.id, " Web ", &pool).await.unwrap();
        assert_eq!(p.name, "Web", "name is stored trimmed");

        let err = inner_create_project(c1.id, "\t", &pool).await.unwrap_err();
        assert_eq!(err.to_string(), "Name is required");

        let err = inner_create_project(c1.id, "WEB", &pool).await.unwrap_err();
        assert_eq!(err.to_string(), "A project named WEB already exists");

        // Same name under another customer is fine
        inner_create_project(c2.id, "Web", &pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_update_project() {
        let pool = crate::test_db().await;

        let c1 = inner_create_customer("Acme", &pool).await.unwrap();
        let c2 = inner_create_customer("Beta", &pool).await.unwrap();
        let web = inner_create_project(c1.id, "Web", &pool).await.unwrap();
        inner_create_project(c1.id, "App", &pool).await.unwrap();
        inner_create_project(c2.id, "Portal", &pool).await.unwrap();

        let renamed = inner_update_project(web.id, " Portal ", &pool)
            .await
            .unwrap();
        assert_eq!(renamed.id, web.id);
        assert_eq!(renamed.customer_id, c1.id);
        assert_eq!(
            renamed.name, "Portal",
            "other customer's project name is allowed"
        );

        inner_update_project(web.id, "portal", &pool).await.unwrap();

        let err = inner_update_project(web.id, "app", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "A project named app already exists");

        let err = inner_update_project(web.id, " ", &pool).await.unwrap_err();
        assert_eq!(err.to_string(), "Name is required");

        let err = inner_update_project(9999, "Ghost", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Project not found");
    }
}
