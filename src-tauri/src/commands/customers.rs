// src-tauri/src/commands/customers.rs
use crate::note_links::customer_slug;
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection};
use tauri::State;

/// A customer row returned to the frontend.
/// SQL aliases map PascalCase schema columns to snake_case fields — no #[sqlx(rename)] needed.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Customer {
    pub id: i64,
    pub name: String,
    pub use_as_default: bool,
}

// ── Validation helpers ────────────────────────────────────────────────────────

/// Trims `name` and rejects it if empty. Shared with projects.
pub(crate) fn clean_name(name: &str) -> Result<String, AppError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(AppError::Custom("Name is required".to_string()));
    }
    Ok(n.to_string())
}

/// Case-insensitive (Unicode-aware) match against `existing`.
/// Done in Rust because SQLite's LOWER() only folds ASCII.
pub(crate) fn is_duplicate(name: &str, existing: &[String]) -> bool {
    let n = name.to_lowercase();
    existing.iter().any(|e| e.trim().to_lowercase() == n)
}

/// Fails if another customer (other than `exclude_id`) already has this name.
async fn ensure_unique(name: &str, exclude_id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let names: Vec<String> = sqlx::query_scalar("SELECT Name FROM customers WHERE id != ?")
        .bind(exclude_id)
        .fetch_all(pool)
        .await?;
    if is_duplicate(name, &names) {
        return Err(AppError::Custom(format!(
            "A customer named {name} already exists"
        )));
    }
    Ok(())
}

/// The error for a name whose tag slug is taken (`#acme` for both "Acme" and "ACME!").
pub(crate) fn tag_clash(tag: &str, other: &str) -> AppError {
    AppError::Custom(format!("Name clashes with tag #{tag} ({other})"))
}

/// Fails if another customer (other than `exclude_id`) has the same tag slug.
async fn ensure_no_tag_clash(
    name: &str,
    exclude_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    let Some(tag) = customer_slug(name) else {
        return Ok(());
    };
    let others: Vec<String> =
        sqlx::query_scalar("SELECT Name FROM customers WHERE id != ? ORDER BY id")
            .bind(exclude_id)
            .fetch_all(pool)
            .await?;
    match others
        .iter()
        .find(|n| customer_slug(n).as_deref() == Some(&tag))
    {
        Some(other) => Err(tag_clash(&tag, other)),
        None => Ok(()),
    }
}

/// Does customer `id` own the tag `cslug`, i.e. no customer with a lower id has it too?
pub(crate) async fn owns_tag(
    id: i64,
    cslug: &str,
    conn: &mut SqliteConnection,
) -> Result<bool, sqlx::Error> {
    let lower: Vec<String> = sqlx::query_scalar("SELECT Name FROM customers WHERE id < ?")
        .bind(id)
        .fetch_all(conn)
        .await?;
    Ok(!lower
        .iter()
        .any(|n| customer_slug(n).as_deref() == Some(cslug)))
}

// ── Inner functions (testable without a Tauri app) ────────────────────────────

pub async fn inner_list_customers(pool: &Pool<Sqlite>) -> Result<Vec<Customer>, AppError> {
    let rows = sqlx::query_as::<_, Customer>(
        "SELECT id, Name as name, UseAsDefault as use_as_default \
         FROM customers \
         ORDER BY Name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn inner_create_customer(name: &str, pool: &Pool<Sqlite>) -> Result<Customer, AppError> {
    let name = clean_name(name)?;
    ensure_unique(&name, 0, pool).await?;
    ensure_no_tag_clash(&name, 0, pool).await?;
    let row = sqlx::query_as::<_, Customer>(
        "INSERT INTO customers (Name, UseAsDefault) VALUES (?, 0) \
         RETURNING id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Renames a customer. When it owns its tag (`#acme`, see `owns_tag`), tags in notes
/// follow the rename (`#acme`, `#acme/web` → `#globex`, `#globex/web`) in the same
/// transaction.
pub async fn inner_update_customer(
    id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
) -> Result<Customer, AppError> {
    let name = clean_name(name)?;
    let old: Option<String> = sqlx::query_scalar("SELECT Name FROM customers WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let Some(old) = old else {
        return Err(AppError::Custom("Customer not found".to_string()));
    };
    ensure_unique(&name, id, pool).await?;
    ensure_no_tag_clash(&name, id, pool).await?;
    let mut tx = pool.begin().await?;
    // The write comes first, so the transaction holds the write lock before it reads.
    let row = sqlx::query_as::<_, Customer>(
        "UPDATE customers SET Name = ? WHERE id = ? \
         RETURNING id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(&name)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Custom("Customer not found".to_string()))?;
    if let (Some(old_tag), Some(new_tag)) = (customer_slug(&old), customer_slug(&name)) {
        if old_tag != new_tag && owns_tag(id, &old_tag, &mut tx).await? {
            crate::commands::notes::rewrite_tag_refs(&old_tag, &new_tag, &mut tx).await?;
        }
    }
    tx.commit().await?;
    Ok(row)
}

pub async fn inner_delete_customer(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE CustomerId = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if count > 0 {
        return Err(AppError::Custom(format!(
            "Has {} project{} — delete them first",
            count,
            if count == 1 { "" } else { "s" }
        )));
    }
    sqlx::query("DELETE FROM customers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Clears UseAsDefault on ALL customers, then sets it on the one with `id`.
/// Runs in a single transaction — no intermediate state is ever visible.
pub async fn inner_set_default_customer(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE customers SET UseAsDefault = 0")
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE customers SET UseAsDefault = 1 WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_customers(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<Customer>, AppError> {
    inner_list_customers(pool.inner()).await
}

#[tauri::command]
pub async fn create_customer(
    name: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Customer, AppError> {
    inner_create_customer(&name, pool.inner()).await
}

#[tauri::command]
pub async fn update_customer(
    id: i64,
    name: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Customer, AppError> {
    inner_update_customer(id, &name, pool.inner()).await
}

#[tauri::command]
pub async fn delete_customer(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_customer(id, pool.inner()).await
}

#[tauri::command]
pub async fn set_default_customer(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_set_default_customer(id, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_list_customers() {
        let pool = crate::test_db().await;

        let c = inner_create_customer("Acme Corp", &pool).await.unwrap();
        assert_eq!(c.name, "Acme Corp");
        assert!(
            !c.use_as_default,
            "newly created customer must not be default"
        );

        let customers = inner_list_customers(&pool).await.unwrap();
        assert_eq!(customers.len(), 1);
        assert_eq!(customers[0].name, "Acme Corp");
    }

    #[tokio::test]
    async fn test_delete_customer() {
        let pool = crate::test_db().await;

        let c = inner_create_customer("To Delete", &pool).await.unwrap();
        inner_delete_customer(c.id, &pool).await.unwrap();

        let customers = inner_list_customers(&pool).await.unwrap();
        assert!(customers.is_empty(), "customer should be gone after delete");
    }

    #[tokio::test]
    async fn test_set_default_customer_clears_previous() {
        let pool = crate::test_db().await;

        let c1 = inner_create_customer("Alpha", &pool).await.unwrap();
        let c2 = inner_create_customer("Beta", &pool).await.unwrap();

        inner_set_default_customer(c1.id, &pool).await.unwrap();
        inner_set_default_customer(c2.id, &pool).await.unwrap();

        let customers = inner_list_customers(&pool).await.unwrap();
        let defaults: Vec<_> = customers.iter().filter(|c| c.use_as_default).collect();

        assert_eq!(defaults.len(), 1, "exactly one default must exist");
        assert_eq!(defaults[0].id, c2.id, "Beta should be the active default");
    }

    #[tokio::test]
    async fn test_create_customer_validates_name() {
        let pool = crate::test_db().await;

        let c = inner_create_customer("  Region Skåne  ", &pool)
            .await
            .unwrap();
        assert_eq!(c.name, "Region Skåne", "name is stored trimmed");

        let err = inner_create_customer("   ", &pool).await.unwrap_err();
        assert_eq!(err.to_string(), "Name is required");

        let err = inner_create_customer("REGION SKÅNE", &pool)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "A customer named REGION SKÅNE already exists"
        );
    }

    #[tokio::test]
    async fn test_update_customer() {
        let pool = crate::test_db().await;

        let a = inner_create_customer("Acme", &pool).await.unwrap();
        inner_create_customer("Beta", &pool).await.unwrap();

        let renamed = inner_update_customer(a.id, " Acme Corp ", &pool)
            .await
            .unwrap();
        assert_eq!(renamed.id, a.id);
        assert_eq!(renamed.name, "Acme Corp");

        // Same name and case change of itself are not duplicates
        inner_update_customer(a.id, "Acme Corp", &pool)
            .await
            .unwrap();
        let recased = inner_update_customer(a.id, "ACME corp", &pool)
            .await
            .unwrap();
        assert_eq!(recased.name, "ACME corp");

        let err = inner_update_customer(a.id, "beta", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "A customer named beta already exists");

        let err = inner_update_customer(a.id, "", &pool).await.unwrap_err();
        assert_eq!(err.to_string(), "Name is required");

        let err = inner_update_customer(9999, "Ghost", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Customer not found");

        // Missing id wins over a duplicate name
        let err = inner_update_customer(9999, "Beta", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Customer not found");
    }

    // ── Tags ──────────────────────────────────────────────────────────────────

    const OLD: &str = "2000-01-01T00:00:00.000";

    /// A note with `body` and an old `UpdatedAt`, so rewrites can be shown not to bump it.
    pub(crate) async fn tagged_note(title: &str, body: &str, pool: &Pool<Sqlite>) -> i64 {
        use crate::commands::notes::{inner_create_note_titled, inner_update_note_body};
        let id = inner_create_note_titled(title, None, pool)
            .await
            .unwrap()
            .id;
        inner_update_note_body(id, body, pool).await.unwrap();
        sqlx::query("UPDATE notes SET UpdatedAt = ? WHERE id = ?")
            .bind(OLD)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    pub(crate) async fn note_body(id: i64, pool: &Pool<Sqlite>) -> (String, String) {
        let n = crate::commands::notes::inner_get_note(id, pool)
            .await
            .unwrap();
        (n.body, n.updated_at)
    }

    #[tokio::test]
    async fn test_customer_tag_clash_blocks_create_and_rename() {
        let pool = crate::test_db().await;
        let acme = inner_create_customer("Acme", &pool).await.unwrap();
        let beta = inner_create_customer("Beta", &pool).await.unwrap();
        for name in ["ACME!", "acme."] {
            let err = inner_create_customer(name, &pool).await.unwrap_err();
            assert_eq!(
                err.to_string(),
                "Name clashes with tag #acme (Acme)",
                "{name}"
            );
        }
        let err = inner_update_customer(beta.id, "Acme Inc.", &pool).await;
        assert!(err.is_ok(), "different slug is fine");
        let err = inner_update_customer(beta.id, " acme ", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "A customer named acme already exists");
        let err = inner_update_customer(beta.id, "(Acme)", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Name clashes with tag #acme (Acme)");
        // Renaming to a name with the same slug as itself is fine; untaggable names too.
        inner_update_customer(acme.id, "ACME", &pool).await.unwrap();
        inner_create_customer("123", &pool).await.unwrap();
        inner_create_customer("!!!", &pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_customer_rename_rewrites_its_tags_in_notes() {
        let pool = crate::test_db().await;
        let acme = inner_create_customer("Acme", &pool).await.unwrap();
        let body = "#acme #Acme/web #acme/web/x #acmeco #acme-x `#acme` [#acme](https://a.se/#acme)\n\n```\n#acme\n```";
        let n = tagged_note("Tagged", body, &pool).await;
        let other = tagged_note("Other", "#acmeco only", &pool).await;

        inner_update_customer(acme.id, "Globex Inc", &pool)
            .await
            .unwrap();
        assert_eq!(
            note_body(n, &pool).await,
            (
                "#globex-inc #globex-inc/web #globex-inc/web/x #acmeco #acme-x `#acme` [#acme](https://a.se/#acme)\n\n```\n#acme\n```".to_string(),
                OLD.to_string()
            )
        );
        assert_eq!(note_body(other, &pool).await.0, "#acmeco only");
        let hits = crate::commands::notes::inner_search_notes("#globex-inc/web", None, &pool)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "refs re-indexed");
        assert!(
            crate::commands::notes::inner_search_notes("#acme", None, &pool)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_customer_rename_skips_tags_it_does_not_own_or_cannot_have() {
        let pool = crate::test_db().await;
        // Existing data that already collides: the lowest id owns `#acme`.
        sqlx::raw_sql("INSERT INTO customers (id, Name) VALUES (1, 'Acme'), (2, 'ACME!');")
            .execute(&pool)
            .await
            .unwrap();
        let n = tagged_note("Tagged", "#acme #acme/web", &pool).await;
        inner_update_customer(2, "Beta", &pool).await.unwrap();
        assert_eq!(note_body(n, &pool).await.0, "#acme #acme/web", "not owned");
        inner_update_customer(1, "2024", &pool).await.unwrap();
        assert_eq!(note_body(n, &pool).await.0, "#acme #acme/web", "no new tag");
        inner_update_customer(1, "Acme", &pool).await.unwrap();
        inner_update_customer(1, "ACME", &pool).await.unwrap();
        assert_eq!(note_body(n, &pool).await.0, "#acme #acme/web", "same slug");
    }

    #[tokio::test]
    async fn test_reserved_customer_slugs_get_no_tag() {
        let pool = crate::test_db().await;
        // `issue` / `todo` are chip namespaces: such customers are allowed but untagged,
        // so they never clash with each other.
        let issue = inner_create_customer("Issue", &pool).await.unwrap();
        inner_create_customer("ISSUE!", &pool).await.unwrap();
        let acme = inner_create_customer("Acme", &pool).await.unwrap();
        let n = tagged_note("Tagged", "#issue/12 #issue #acme #acme/web #todo/3", &pool).await;
        inner_update_customer(issue.id, "X", &pool).await.unwrap();
        assert_eq!(
            note_body(n, &pool).await.0,
            "#issue/12 #issue #acme #acme/web #todo/3",
            "renaming from a reserved slug rewrites nothing"
        );
        inner_update_customer(acme.id, "Todo", &pool).await.unwrap();
        assert_eq!(
            note_body(n, &pool).await.0,
            "#issue/12 #issue #acme #acme/web #todo/3",
            "renaming to a reserved slug rewrites nothing"
        );
        inner_update_customer(acme.id, "Acme", &pool).await.unwrap();
        assert_eq!(
            note_body(n, &pool).await.0,
            "#issue/12 #issue #acme #acme/web #todo/3",
            "Todo had no tag to rename from"
        );
    }
}
