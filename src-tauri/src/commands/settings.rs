/// App preference commands backed by the `app_settings` table.
use crate::AppError;
use sqlx::{Pool, Sqlite};
use tauri::State;

/// Reads the theme mode ("system", "light" or "dark").
pub async fn inner_get_theme(pool: &Pool<Sqlite>) -> Result<String, AppError> {
    crate::app_settings::get_theme(pool).await
}

/// Validates and stores the theme mode.
pub async fn inner_set_theme(theme: &str, pool: &Pool<Sqlite>) -> Result<String, AppError> {
    crate::app_settings::set_theme(pool, theme).await
}

#[tauri::command]
pub async fn get_theme(pool: State<'_, Pool<Sqlite>>) -> Result<String, AppError> {
    inner_get_theme(&pool).await
}

#[tauri::command]
pub async fn set_theme(theme: String, pool: State<'_, Pool<Sqlite>>) -> Result<String, AppError> {
    inner_set_theme(&theme, &pool).await
}

#[cfg(test)]
mod tests {
    use super::{inner_get_theme, inner_set_theme};

    #[tokio::test]
    async fn set_theme_then_get_theme_returns_saved_mode() {
        let pool = crate::test_db().await;

        inner_set_theme("light", &pool)
            .await
            .expect("theme should save");

        assert_eq!(
            inner_get_theme(&pool).await.expect("theme should load"),
            "light"
        );
    }
}
