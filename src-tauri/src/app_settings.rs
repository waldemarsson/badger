use sqlx::{Pool, Sqlite};

use crate::AppError;

pub const STARTUP_BACKUP_RETENTION_KEY: &str = "startup_backup_retention";
pub const DEFAULT_STARTUP_BACKUP_RETENTION: i64 = 3;
pub const THEME_KEY: &str = "theme";
pub const DEFAULT_THEME: &str = "system";
const THEMES: [&str; 3] = ["system", "light", "dark"];

fn validate_startup_backup_retention(retention: i64) -> Result<i64, AppError> {
    if retention < 0 {
        return Err(AppError::Custom(
            "Startup backup retention must be greater than or equal to 0.".to_string(),
        ));
    }

    Ok(retention)
}

fn parse_startup_backup_retention(value: &str) -> Result<i64, AppError> {
    value.parse::<i64>().map_err(|_| {
        AppError::Custom(format!(
            "Invalid startup backup retention value stored in app_settings: {value}"
        ))
    })
}

pub async fn get_startup_backup_retention(pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    let retention = sqlx::query_scalar::<_, String>("SELECT Value FROM app_settings WHERE Key = ?")
        .bind(STARTUP_BACKUP_RETENTION_KEY)
        .fetch_optional(pool)
        .await?
        .map(|value| parse_startup_backup_retention(&value))
        .transpose()?
        .unwrap_or(DEFAULT_STARTUP_BACKUP_RETENTION);

    Ok(retention)
}

pub async fn set_startup_backup_retention(
    pool: &Pool<Sqlite>,
    retention: i64,
) -> Result<i64, AppError> {
    let retention = validate_startup_backup_retention(retention)?;

    sqlx::query(
        "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
         ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
    )
    .bind(STARTUP_BACKUP_RETENTION_KEY)
    .bind(retention.to_string())
    .execute(pool)
    .await?;

    Ok(retention)
}

/// Returns the stored theme mode, falling back to "system" when unset or unrecognised.
pub async fn get_theme(pool: &Pool<Sqlite>) -> Result<String, AppError> {
    let theme = sqlx::query_scalar::<_, String>("SELECT Value FROM app_settings WHERE Key = ?")
        .bind(THEME_KEY)
        .fetch_optional(pool)
        .await?
        .filter(|value| THEMES.contains(&value.as_str()))
        .unwrap_or_else(|| DEFAULT_THEME.to_string());

    Ok(theme)
}

pub async fn set_theme(pool: &Pool<Sqlite>, theme: &str) -> Result<String, AppError> {
    if !THEMES.contains(&theme) {
        return Err(AppError::Custom(
            "Theme must be system, light, or dark.".to_string(),
        ));
    }

    sqlx::query(
        "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
         ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
    )
    .bind(THEME_KEY)
    .bind(theme)
    .execute(pool)
    .await?;

    Ok(theme.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        get_startup_backup_retention, get_theme, set_startup_backup_retention, set_theme,
        DEFAULT_STARTUP_BACKUP_RETENTION,
    };

    #[tokio::test]
    async fn app_settings_default_value_is_3() {
        let pool = crate::test_db().await;

        let retention = get_startup_backup_retention(&pool)
            .await
            .expect("default retention should load");

        assert_eq!(retention, DEFAULT_STARTUP_BACKUP_RETENTION);
    }

    #[tokio::test]
    async fn app_settings_round_trips_persisted_values_including_zero() {
        let pool = crate::test_db().await;

        let saved = set_startup_backup_retention(&pool, 5)
            .await
            .expect("retention of 5 should save");
        assert_eq!(saved, 5);
        assert_eq!(
            get_startup_backup_retention(&pool)
                .await
                .expect("retention of 5 should load"),
            5
        );

        let saved = set_startup_backup_retention(&pool, 0)
            .await
            .expect("retention of 0 should save");
        assert_eq!(saved, 0);
        assert_eq!(
            get_startup_backup_retention(&pool)
                .await
                .expect("retention of 0 should load"),
            0
        );
    }

    #[tokio::test]
    async fn app_settings_rejects_negative_values() {
        let pool = crate::test_db().await;

        let error = set_startup_backup_retention(&pool, -1)
            .await
            .expect_err("negative retention should be rejected");

        assert!(
            error
                .to_string()
                .contains("Startup backup retention must be greater than or equal to 0."),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn app_settings_rejects_invalid_stored_values() {
        let pool = crate::test_db().await;

        sqlx::query("INSERT INTO app_settings (Key, Value) VALUES (?, ?)")
            .bind(super::STARTUP_BACKUP_RETENTION_KEY)
            .bind("invalid")
            .execute(&pool)
            .await
            .expect("invalid retention row should insert");

        let error = get_startup_backup_retention(&pool)
            .await
            .expect_err("invalid stored retention should fail to parse");

        assert!(
            error
                .to_string()
                .contains("Invalid startup backup retention value stored in app_settings"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn theme_defaults_to_system() {
        let pool = crate::test_db().await;

        assert_eq!(get_theme(&pool).await.expect("theme should load"), "system");
    }

    #[tokio::test]
    async fn theme_round_trips_every_mode() {
        let pool = crate::test_db().await;

        for mode in ["dark", "light", "system"] {
            assert_eq!(
                set_theme(&pool, mode).await.expect("theme should save"),
                mode
            );
            assert_eq!(get_theme(&pool).await.expect("theme should load"), mode);
        }
    }

    #[tokio::test]
    async fn theme_rejects_unknown_mode() {
        let pool = crate::test_db().await;
        set_theme(&pool, "dark").await.expect("theme should save");

        let error = set_theme(&pool, "sepia")
            .await
            .expect_err("unknown theme should be rejected");

        assert_eq!(error.to_string(), "Theme must be system, light, or dark.");
        assert_eq!(get_theme(&pool).await.expect("theme should load"), "dark");
    }

    #[tokio::test]
    async fn theme_falls_back_to_system_for_invalid_stored_value() {
        let pool = crate::test_db().await;

        sqlx::query("INSERT INTO app_settings (Key, Value) VALUES (?, ?)")
            .bind(super::THEME_KEY)
            .bind("sepia")
            .execute(&pool)
            .await
            .expect("invalid theme row should insert");

        assert_eq!(get_theme(&pool).await.expect("theme should load"), "system");
    }
}
