/// App preference commands backed by the `app_settings` table.
use crate::app_settings::PaletteSettings;
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

/// Reads the command palette shortcut and last-used project/issue.
pub async fn inner_get_palette_settings(pool: &Pool<Sqlite>) -> Result<PaletteSettings, AppError> {
    crate::app_settings::get_palette_settings(pool).await
}

pub async fn inner_set_palette_shortcut(
    shortcut: &str,
    pool: &Pool<Sqlite>,
) -> Result<String, AppError> {
    crate::app_settings::set_palette_shortcut(pool, shortcut).await
}

/// Remembers the project and issue of the last time entry logged from the palette.
pub async fn inner_set_palette_last_used(
    project_id: i64,
    issue_id: i64,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    crate::app_settings::set_palette_last_used(pool, project_id, issue_id).await
}

#[tauri::command]
pub async fn get_palette_settings(
    pool: State<'_, Pool<Sqlite>>,
) -> Result<PaletteSettings, AppError> {
    inner_get_palette_settings(&pool).await
}

#[tauri::command]
pub async fn set_palette_shortcut(
    shortcut: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<String, AppError> {
    inner_set_palette_shortcut(&shortcut, &pool).await
}

#[tauri::command]
pub async fn set_palette_last_used(
    project_id: i64,
    issue_id: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<(), AppError> {
    inner_set_palette_last_used(project_id, issue_id, &pool).await
}

/// Reads the daily hours target (default 8, 0 = off).
pub async fn inner_get_daily_target(pool: &Pool<Sqlite>) -> Result<f64, AppError> {
    crate::app_settings::get_daily_target(pool).await
}

/// Validates (0–24) and stores the daily hours target.
pub async fn inner_set_daily_target(target: f64, pool: &Pool<Sqlite>) -> Result<f64, AppError> {
    crate::app_settings::set_daily_target(pool, target).await
}

#[tauri::command]
pub async fn get_daily_target(pool: State<'_, Pool<Sqlite>>) -> Result<f64, AppError> {
    inner_get_daily_target(&pool).await
}

#[tauri::command]
pub async fn set_daily_target(target: f64, pool: State<'_, Pool<Sqlite>>) -> Result<f64, AppError> {
    inner_set_daily_target(target, &pool).await
}

/// Reads the expanded notes-tree folder ids (stale ids skipped).
pub async fn inner_get_expanded_folders(pool: &Pool<Sqlite>) -> Result<Vec<i64>, AppError> {
    crate::app_settings::get_expanded_folders(pool).await
}

/// Stores the expanded notes-tree folder ids; returns what was stored.
pub async fn inner_set_expanded_folders(
    ids: Vec<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Vec<i64>, AppError> {
    crate::app_settings::set_expanded_folders(pool, ids).await
}

#[tauri::command]
pub async fn get_expanded_folders(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<i64>, AppError> {
    inner_get_expanded_folders(&pool).await
}

#[tauri::command]
pub async fn set_expanded_folders(
    ids: Vec<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<i64>, AppError> {
    inner_set_expanded_folders(ids, &pool).await
}

#[cfg(test)]
mod tests {
    use super::{
        inner_get_daily_target, inner_get_expanded_folders, inner_get_palette_settings,
        inner_get_theme, inner_set_daily_target, inner_set_expanded_folders,
        inner_set_palette_last_used, inner_set_palette_shortcut, inner_set_theme,
    };

    #[tokio::test]
    async fn daily_target_commands_round_trip_and_validate() {
        let pool = crate::test_db().await;

        assert_eq!(inner_get_daily_target(&pool).await.unwrap(), 8.0);
        assert_eq!(inner_set_daily_target(7.5, &pool).await.unwrap(), 7.5);
        assert_eq!(inner_get_daily_target(&pool).await.unwrap(), 7.5);

        let error = inner_set_daily_target(25.0, &pool).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Daily target must be between 0 and 24 hours"
        );
        assert_eq!(inner_get_daily_target(&pool).await.unwrap(), 7.5);
    }

    #[tokio::test]
    async fn palette_settings_commands_round_trip() {
        let pool = crate::test_db().await;

        inner_set_palette_shortcut("Alt+Space", &pool)
            .await
            .expect("shortcut should save");
        inner_set_palette_last_used(2, 5, &pool)
            .await
            .expect("last used should save");

        let settings = inner_get_palette_settings(&pool)
            .await
            .expect("settings should load");
        assert_eq!(settings.shortcut, "Alt+Space");
        assert_eq!(settings.last_project_id, Some(2));
        assert_eq!(settings.last_issue_id, Some(5));
    }

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

    #[tokio::test]
    async fn expanded_folders_commands_round_trip() {
        let pool = crate::test_db().await;
        let f = crate::commands::folders::inner_create_folder(Some("F"), None, &pool)
            .await
            .unwrap();

        assert_eq!(
            inner_set_expanded_folders(vec![f.id, 404], &pool)
                .await
                .unwrap(),
            vec![f.id]
        );
        assert_eq!(inner_get_expanded_folders(&pool).await.unwrap(), vec![f.id]);
    }
}
