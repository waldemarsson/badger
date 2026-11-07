// src-tauri/src/commands/projects.rs
use crate::commands::customers::{clean_name, is_duplicate, owns_tag, tag_clash};
use crate::note_links::{customer_slug, slug};
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

/// Fails if another project of `customer_id` (other than `exclude_id`) has the same
/// tag slug. The message names the full tag (`#acme/web`) when the customer has one.
async fn ensure_no_tag_clash(
    customer_id: i64,
    name: &str,
    exclude_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    let Some(tag) = slug(name) else {
        return Ok(());
    };
    let others: Vec<String> = sqlx::query_scalar(
        "SELECT Name FROM projects WHERE CustomerId = ? AND id != ? ORDER BY id",
    )
    .bind(customer_id)
    .bind(exclude_id)
    .fetch_all(pool)
    .await?;
    let Some(other) = others.iter().find(|n| slug(n).as_deref() == Some(&tag)) else {
        return Ok(());
    };
    let customer: Option<String> = sqlx::query_scalar("SELECT Name FROM customers WHERE id = ?")
        .bind(customer_id)
        .fetch_optional(pool)
        .await?;
    let full = match customer.as_deref().and_then(customer_slug) {
        Some(cslug) => format!("{cslug}/{tag}"),
        None => tag,
    };
    Err(tag_clash(&full, other))
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
    ensure_no_tag_clash(customer_id, &name, 0, pool).await?;
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

/// Renames a project. When it owns its tag (`#acme/web`: its customer owns `#acme` and
/// no lower-id project of that customer has the same slug), tags in notes follow the
/// rename (`#acme/web`, `#acme/web/x` → `#acme/portal`, `#acme/portal/x`) in the same
/// transaction.
pub async fn inner_update_project(
    id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
) -> Result<Project, AppError> {
    let name = clean_name(name)?;
    let (customer_id, old): (i64, String) =
        sqlx::query_as("SELECT CustomerId, Name FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::Custom("Project not found".to_string()))?;
    ensure_unique(customer_id, &name, id, pool).await?;
    ensure_no_tag_clash(customer_id, &name, id, pool).await?;
    let mut tx = pool.begin().await?;
    // The write comes first, so the transaction holds the write lock before it reads.
    let row = sqlx::query_as::<_, Project>(
        "UPDATE projects SET Name = ? WHERE id = ? \
         RETURNING id, CustomerId as customer_id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(&name)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if let (Some(old_tag), Some(new_tag)) = (slug(&old), slug(&name)) {
        if old_tag != new_tag {
            let customer: String = sqlx::query_scalar("SELECT Name FROM customers WHERE id = ?")
                .bind(customer_id)
                .fetch_one(&mut *tx)
                .await?;
            let lower: Vec<String> =
                sqlx::query_scalar("SELECT Name FROM projects WHERE CustomerId = ? AND id < ?")
                    .bind(customer_id)
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await?;
            let owns_project_tag = !lower.iter().any(|n| slug(n).as_deref() == Some(&old_tag));
            if let Some(cslug) = customer_slug(&customer) {
                if owns_project_tag && owns_tag(customer_id, &cslug, &mut tx).await? {
                    let old_path = format!("{cslug}/{old_tag}");
                    crate::commands::notes::rewrite_tag_refs(&old_path, &new_tag, &mut tx).await?;
                }
            }
        }
    }
    tx.commit().await?;
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

/// Renames a project, then queues every note for the semantic index: the rename may have
/// rewritten `#tags` in note bodies.
pub async fn update_project_and_queue(
    id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
    semantic: &crate::semantic::SemanticState,
) -> Result<Project, AppError> {
    let row = inner_update_project(id, name, pool).await?;
    crate::commands::notes::queue_all_after_tag_rename(pool, semantic).await;
    Ok(row)
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
    semantic: State<'_, crate::semantic::SemanticState>,
) -> Result<Project, AppError> {
    update_project_and_queue(id, &name, pool.inner(), semantic.inner()).await
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

    // ── Tags ──────────────────────────────────────────────────────────────────

    use crate::commands::customers::tests::{note_body, tagged_note};

    #[tokio::test]
    async fn test_project_tag_clash_blocks_create_and_rename_within_customer() {
        let pool = crate::test_db().await;
        let acme = inner_create_customer("Acme", &pool).await.unwrap();
        let beta = inner_create_customer("Beta", &pool).await.unwrap();
        let web = inner_create_project(acme.id, "Web App", &pool)
            .await
            .unwrap();
        let err = inner_create_project(acme.id, "web-app", &pool)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Name clashes with tag #acme/web-app (Web App)"
        );
        inner_create_project(beta.id, "web-app", &pool)
            .await
            .unwrap();
        let api = inner_create_project(acme.id, "API", &pool).await.unwrap();
        let err = inner_update_project(api.id, "Web/App", &pool)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Name clashes with tag #acme/web-app (Web App)"
        );
        inner_update_project(web.id, "WEB APP", &pool)
            .await
            .unwrap();
        // A customer without a tag: the message names the project slug only.
        let digits = inner_create_customer("42", &pool).await.unwrap();
        inner_create_project(digits.id, "Ops", &pool).await.unwrap();
        let err = inner_create_project(digits.id, "ops!", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Name clashes with tag #ops (Ops)");
    }

    #[tokio::test]
    async fn test_project_rename_rewrites_its_tags_in_notes() {
        let pool = crate::test_db().await;
        let acme = inner_create_customer("Acme", &pool).await.unwrap();
        let web = inner_create_project(acme.id, "Web", &pool).await.unwrap();
        let n = tagged_note(
            "Tagged",
            "#acme/web #Acme/Web/x #acme/webby #acme #web `#acme/web`",
            &pool,
        )
        .await;
        inner_update_project(web.id, "Portal 2", &pool)
            .await
            .unwrap();
        assert_eq!(
            note_body(n, &pool).await,
            (
                "#acme/portal-2 #Acme/portal-2/x #acme/webby #acme #web `#acme/web`".to_string(),
                "2000-01-01T00:00:00.000".to_string()
            )
        );
    }

    #[tokio::test]
    async fn test_project_rename_skips_tags_it_does_not_own() {
        let pool = crate::test_db().await;
        // Customer 2 collides with 1 on `#acme`; projects 3 and 4 collide on `web`.
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'Acme'), (2, 'ACME!');
             INSERT INTO projects (id, CustomerId, Name) VALUES
                 (3, 1, 'Web'), (4, 1, 'web!'), (5, 2, 'Web');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let n = tagged_note("Tagged", "#acme/web", &pool).await;
        inner_update_project(5, "Other", &pool).await.unwrap();
        assert_eq!(
            note_body(n, &pool).await.0,
            "#acme/web",
            "customer not owner"
        );
        inner_update_project(4, "Else", &pool).await.unwrap();
        assert_eq!(
            note_body(n, &pool).await.0,
            "#acme/web",
            "project not owner"
        );
        inner_update_project(3, "123", &pool).await.unwrap();
        assert_eq!(note_body(n, &pool).await.0, "#acme/web", "no new tag");
        inner_update_project(3, "Site", &pool).await.unwrap();
        assert_eq!(note_body(n, &pool).await.0, "#acme/web", "123 had no tag");
    }

    #[tokio::test]
    async fn test_project_under_reserved_customer_has_no_tag() {
        let pool = crate::test_db().await;
        let issue = inner_create_customer("Issue", &pool).await.unwrap();
        let web = inner_create_project(issue.id, "12", &pool).await.unwrap();
        let api = inner_create_project(issue.id, "Web", &pool).await.unwrap();
        let n = tagged_note("Tagged", "#issue/12 #issue/web", &pool).await;
        inner_update_project(web.id, "Thirteen", &pool)
            .await
            .unwrap();
        inner_update_project(api.id, "Site", &pool).await.unwrap();
        assert_eq!(note_body(n, &pool).await.0, "#issue/12 #issue/web");
        // Clash message names the project slug only, as for any untagged customer.
        let err = inner_create_project(issue.id, "site!", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Name clashes with tag #site (Site)");
    }

    // ── Semantic index ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rename_queues_all_notes_and_survives_a_queue_failure() {
        let pool = crate::test_db().await;
        let c = crate::commands::customers::inner_create_customer("Acme", &pool)
            .await
            .unwrap();
        let id = inner_create_project(c.id, "Web", &pool).await.unwrap().id;
        let tagged =
            crate::commands::customers::tests::tagged_note("Tagged", "#acme/web notes", &pool)
                .await;
        let plain = crate::commands::customers::tests::tagged_note("Plain", "nothing", &pool).await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;

        update_project_and_queue(id, "Portal", &pool, &state)
            .await
            .unwrap();
        let mut queued = state.queued();
        queued.sort();
        assert_eq!(queued, vec![tagged, plain]);

        // Queueing fails after the rename committed: the rename still succeeds.
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        sqlx::raw_sql("DROP TABLE notes_fts; DROP TABLE note_refs; DROP TABLE note_embeddings; DROP TABLE notes;")
            .execute(&pool)
            .await
            .unwrap();
        let renamed = update_project_and_queue(id, "PORTAL", &pool, &state)
            .await
            .unwrap();
        assert_eq!(renamed.name, "PORTAL");
        assert!(state.queued().is_empty());
    }
}
