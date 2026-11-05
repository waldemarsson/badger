use sqlx::{Pool, Sqlite};

use crate::AppError;

pub const STARTUP_BACKUP_RETENTION_KEY: &str = "startup_backup_retention";
pub const DEFAULT_STARTUP_BACKUP_RETENTION: i64 = 3;
pub const THEME_KEY: &str = "theme";
pub const DEFAULT_THEME: &str = "system";
const THEMES: [&str; 3] = ["system", "light", "dark"];
pub const PALETTE_SHORTCUT_KEY: &str = "palette_shortcut";
pub const DEFAULT_PALETTE_SHORTCUT: &str = "Meta+K";
pub const PALETTE_LAST_PROJECT_KEY: &str = "palette_last_project_id";
pub const PALETTE_LAST_ISSUE_KEY: &str = "palette_last_issue_id";
const MAX_SHORTCUT_LEN: usize = 64;

/// Command palette preferences. Last-used ids may point at deleted rows;
/// the frontend falls back when they do.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PaletteSettings {
    pub shortcut: String,
    pub last_project_id: Option<i64>,
    pub last_issue_id: Option<i64>,
}

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

async fn get_value(pool: &Pool<Sqlite>, key: &str) -> Result<Option<String>, AppError> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT Value FROM app_settings WHERE Key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?,
    )
}

async fn set_value(
    executor: impl sqlx::SqliteExecutor<'_>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
         ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
    )
    .bind(key)
    .bind(value)
    .execute(executor)
    .await?;
    Ok(())
}

/// Reads palette preferences; unset or unparseable ids load as None.
pub async fn get_palette_settings(pool: &Pool<Sqlite>) -> Result<PaletteSettings, AppError> {
    let shortcut = get_value(pool, PALETTE_SHORTCUT_KEY)
        .await?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PALETTE_SHORTCUT.to_string());
    let last_project_id = get_value(pool, PALETTE_LAST_PROJECT_KEY)
        .await?
        .and_then(|value| value.parse().ok());
    let last_issue_id = get_value(pool, PALETTE_LAST_ISSUE_KEY)
        .await?
        .and_then(|value| value.parse().ok());

    Ok(PaletteSettings {
        shortcut,
        last_project_id,
        last_issue_id,
    })
}

/// Stores the palette shortcut. Combination rules live in the frontend
/// (`src/lib/palette.ts`); this only guards against empty or oversized values.
pub async fn set_palette_shortcut(pool: &Pool<Sqlite>, shortcut: &str) -> Result<String, AppError> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return Err(AppError::Custom("Shortcut is required".to_string()));
    }
    if shortcut.len() > MAX_SHORTCUT_LEN {
        return Err(AppError::Custom("Shortcut is too long".to_string()));
    }

    set_value(pool, PALETTE_SHORTCUT_KEY, shortcut).await?;
    Ok(shortcut.to_string())
}

pub async fn set_palette_last_used(
    pool: &Pool<Sqlite>,
    project_id: i64,
    issue_id: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    set_value(&mut *tx, PALETTE_LAST_PROJECT_KEY, &project_id.to_string()).await?;
    set_value(&mut *tx, PALETTE_LAST_ISSUE_KEY, &issue_id.to_string()).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        get_palette_settings, get_startup_backup_retention, get_theme, set_palette_last_used,
        set_palette_shortcut, set_startup_backup_retention, set_theme, DEFAULT_PALETTE_SHORTCUT,
        DEFAULT_STARTUP_BACKUP_RETENTION,
    };

    #[tokio::test]
    async fn palette_settings_default_to_meta_k_and_no_last_used() {
        let pool = crate::test_db().await;

        let settings = get_palette_settings(&pool)
            .await
            .expect("settings should load");

        assert_eq!(settings.shortcut, DEFAULT_PALETTE_SHORTCUT);
        assert_eq!(settings.last_project_id, None);
        assert_eq!(settings.last_issue_id, None);
    }

    #[tokio::test]
    async fn palette_shortcut_round_trips_trimmed() {
        let pool = crate::test_db().await;

        let saved = set_palette_shortcut(&pool, " Ctrl+Shift+P ")
            .await
            .expect("shortcut should save");

        assert_eq!(saved, "Ctrl+Shift+P");
        assert_eq!(
            get_palette_settings(&pool).await.unwrap().shortcut,
            "Ctrl+Shift+P"
        );
    }

    #[tokio::test]
    async fn palette_shortcut_rejects_empty_and_oversized() {
        let pool = crate::test_db().await;

        let empty = set_palette_shortcut(&pool, "  ").await.unwrap_err();
        assert_eq!(empty.to_string(), "Shortcut is required");

        let long = set_palette_shortcut(&pool, &"K".repeat(65))
            .await
            .unwrap_err();
        assert_eq!(long.to_string(), "Shortcut is too long");

        assert_eq!(
            get_palette_settings(&pool).await.unwrap().shortcut,
            DEFAULT_PALETTE_SHORTCUT
        );
    }

    #[tokio::test]
    async fn palette_last_used_round_trips_and_overwrites() {
        let pool = crate::test_db().await;

        set_palette_last_used(&pool, 3, 7)
            .await
            .expect("should save");
        set_palette_last_used(&pool, 4, 9)
            .await
            .expect("should overwrite");

        let settings = get_palette_settings(&pool).await.unwrap();
        assert_eq!(settings.last_project_id, Some(4));
        assert_eq!(settings.last_issue_id, Some(9));
    }

    #[tokio::test]
    async fn palette_last_used_ignores_unparseable_stored_values() {
        let pool = crate::test_db().await;

        sqlx::query("INSERT INTO app_settings (Key, Value) VALUES (?, 'x'), (?, '')")
            .bind(super::PALETTE_LAST_PROJECT_KEY)
            .bind(super::PALETTE_LAST_ISSUE_KEY)
            .execute(&pool)
            .await
            .unwrap();

        let settings = get_palette_settings(&pool).await.unwrap();
        assert_eq!(settings.last_project_id, None);
        assert_eq!(settings.last_issue_id, None);
    }

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
