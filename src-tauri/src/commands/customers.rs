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
    let row = sqlx::query_as::<_, Customer>(
        "INSERT INTO customers (Name, UseAsDefault) VALUES (?, 0) \
         RETURNING id, Name as name, UseAsDefault as use_as_default",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
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
}
