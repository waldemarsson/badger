// src-tauri/src/commands/customers.rs
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
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
    let row = sqlx::query_as::<_, Customer>(
        "INSERT INTO customers (Name, UseAsDefault) VALUES (?, 0) \
         RETURNING id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn inner_update_customer(
    id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
) -> Result<Customer, AppError> {
    let name = clean_name(name)?;
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM customers WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::Custom("Customer not found".to_string()));
    }
    ensure_unique(&name, id, pool).await?;
    sqlx::query_as::<_, Customer>(
        "UPDATE customers SET Name = ? WHERE id = ? \
         RETURNING id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(name)
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::Custom("Customer not found".to_string()))
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
mod tests {
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
}
