use sqlx::{Pool, Sqlite, SqliteConnection};

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
pub const DAILY_TARGET_KEY: &str = "daily_target";
pub const DEFAULT_DAILY_TARGET: f64 = 8.0;
const MAX_DAILY_TARGET: f64 = 24.0;
/// JSON array of the folder ids expanded in the notes tree.
pub const EXPANDED_FOLDERS_KEY: &str = "notes_expanded_folders";
/// "true" once the semantic search model is downloaded and verified.
pub const SEMANTIC_ENABLED_KEY: &str = "semantic_search_enabled";

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

fn is_valid_daily_target(target: f64) -> bool {
    target.is_finite() && (0.0..=MAX_DAILY_TARGET).contains(&target)
}

/// Returns the daily hours target, falling back to 8 when unset or invalid.
pub async fn get_daily_target(pool: &Pool<Sqlite>) -> Result<f64, AppError> {
    let target = get_value(pool, DAILY_TARGET_KEY)
        .await?
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|target| is_valid_daily_target(*target))
        .unwrap_or(DEFAULT_DAILY_TARGET);

    Ok(target)
}

/// Stores the daily hours target (0 = off). Allowed range is 0–24.
pub async fn set_daily_target(pool: &Pool<Sqlite>, target: f64) -> Result<f64, AppError> {
    if !is_valid_daily_target(target) {
        return Err(AppError::Custom(
            "Daily target must be between 0 and 24 hours".to_string(),
        ));
    }

    set_value(pool, DAILY_TARGET_KEY, &target.to_string()).await?;
    Ok(target)
}

/// Whether semantic search is enabled; unset or anything but "true" is off.
pub async fn get_semantic_enabled(pool: &Pool<Sqlite>) -> Result<bool, AppError> {
    Ok(get_value(pool, SEMANTIC_ENABLED_KEY).await?.as_deref() == Some("true"))
}

pub async fn set_semantic_enabled(pool: &Pool<Sqlite>, enabled: bool) -> Result<(), AppError> {
    set_value(
        pool,
        SEMANTIC_ENABLED_KEY,
        if enabled { "true" } else { "false" },
    )
    .await
}

/// Parses a stored id list; anything unparseable reads as empty.
fn parse_folder_ids(value: Option<String>) -> Vec<i64> {
    value
        .and_then(|v| serde_json::from_str::<Vec<i64>>(&v).ok())
        .unwrap_or_default()
}

/// `ids` without duplicates and without ids that are not (or no longer) folders, in order.
async fn existing_folder_ids(
    ids: Vec<i64>,
    conn: &mut SqliteConnection,
) -> Result<Vec<i64>, AppError> {
    let existing: std::collections::HashSet<i64> = sqlx::query_scalar("SELECT id FROM folders")
        .fetch_all(conn)
        .await?
        .into_iter()
        .collect();
    let mut seen = std::collections::HashSet::new();
    Ok(ids
        .into_iter()
        .filter(|id| existing.contains(id) && seen.insert(*id))
        .collect())
}

/// Returns the expanded notes-tree folder ids; stale ids (deleted folders) are skipped.
pub async fn get_expanded_folders(pool: &Pool<Sqlite>) -> Result<Vec<i64>, AppError> {
    let mut conn = pool.acquire().await?;
    let stored = parse_folder_ids(
        sqlx::query_scalar::<_, String>("SELECT Value FROM app_settings WHERE Key = ?")
            .bind(EXPANDED_FOLDERS_KEY)
            .fetch_optional(&mut *conn)
            .await?,
    );
    existing_folder_ids(stored, &mut conn).await
}

/// Stores the expanded folder ids (unknown ids and duplicates dropped) and returns them.
pub async fn set_expanded_folders(
    pool: &Pool<Sqlite>,
    ids: Vec<i64>,
) -> Result<Vec<i64>, AppError> {
    let mut tx = pool.begin().await?;
    let ids = existing_folder_ids(ids, &mut tx).await?;
    let json = serde_json::to_string(&ids).map_err(|e| AppError::Custom(e.to_string()))?;
    set_value(&mut *tx, EXPANDED_FOLDERS_KEY, &json).await?;
    tx.commit().await?;
    Ok(ids)
}

/// Drops a (deleted) folder id from the stored expanded set; part of the caller's
/// transaction.
pub(crate) async fn forget_expanded_folder(
    id: i64,
    conn: &mut SqliteConnection,
) -> Result<(), AppError> {
    let Some(value) =
        sqlx::query_scalar::<_, String>("SELECT Value FROM app_settings WHERE Key = ?")
            .bind(EXPANDED_FOLDERS_KEY)
            .fetch_optional(&mut *conn)
            .await?
    else {
        return Ok(());
    };
    let ids: Vec<i64> = parse_folder_ids(Some(value))
        .into_iter()
        .filter(|stored| *stored != id)
        .collect();
    let json = serde_json::to_string(&ids).map_err(|e| AppError::Custom(e.to_string()))?;
    set_value(conn, EXPANDED_FOLDERS_KEY, &json).await
}

#[cfg(test)]
mod tests {
    use super::{
        get_daily_target, get_expanded_folders, get_palette_settings, get_startup_backup_retention,
        get_theme, set_daily_target, set_expanded_folders, set_palette_last_used,
        set_palette_shortcut, set_startup_backup_retention, set_theme, DEFAULT_DAILY_TARGET,
        DEFAULT_PALETTE_SHORTCUT, DEFAULT_STARTUP_BACKUP_RETENTION,
    };

    #[tokio::test]
    async fn daily_target_defaults_to_8() {
        let pool = crate::test_db().await;

        let target = get_daily_target(&pool).await.expect("target should load");

        assert_eq!(target, DEFAULT_DAILY_TARGET);
        assert_eq!(target, 8.0);
    }

    #[tokio::test]
    async fn daily_target_round_trips_including_zero_and_fractions() {
        let pool = crate::test_db().await;

        for value in [7.5, 0.0, 24.0, 6.25] {
            assert_eq!(
                set_daily_target(&pool, value)
                    .await
                    .expect("target should save"),
                value
            );
            assert_eq!(
                get_daily_target(&pool).await.expect("target should load"),
                value
            );
        }
    }

    #[tokio::test]
    async fn daily_target_rejects_out_of_range_and_non_finite() {
        let pool = crate::test_db().await;
        set_daily_target(&pool, 6.0)
            .await
            .expect("target should save");

        for value in [-1.0, 24.5, f64::NAN, f64::INFINITY] {
            let error = set_daily_target(&pool, value)
                .await
                .expect_err("invalid target should be rejected");
            assert_eq!(
                error.to_string(),
                "Daily target must be between 0 and 24 hours"
            );
        }

        assert_eq!(get_daily_target(&pool).await.unwrap(), 6.0);
    }

    #[tokio::test]
    async fn daily_target_falls_back_to_default_for_invalid_stored_value() {
        let pool = crate::test_db().await;

        for stored in ["abc", "", "-3", "30", "NaN", "inf"] {
            sqlx::query(
                "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
                 ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
            )
            .bind(super::DAILY_TARGET_KEY)
            .bind(stored)
            .execute(&pool)
            .await
            .expect("invalid target row should insert");

            assert_eq!(
                get_daily_target(&pool).await.expect("target should load"),
                DEFAULT_DAILY_TARGET,
                "stored value {stored:?} should fall back"
            );
        }
    }

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
    async fn insert_folders(pool: &sqlx::SqlitePool, ids: &[i64]) {
        for id in ids {
            sqlx::query("INSERT INTO folders (id, Name) VALUES (?, ?)")
                .bind(id)
                .bind(format!("F{id}"))
                .execute(pool)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn expanded_folders_default_to_empty_and_round_trip() {
        let pool = crate::test_db().await;
        insert_folders(&pool, &[1, 2, 3]).await;

        assert_eq!(
            get_expanded_folders(&pool).await.unwrap(),
            Vec::<i64>::new()
        );
        assert_eq!(
            set_expanded_folders(&pool, vec![3, 1]).await.unwrap(),
            vec![3, 1]
        );
        assert_eq!(get_expanded_folders(&pool).await.unwrap(), vec![3, 1]);
        assert_eq!(
            set_expanded_folders(&pool, vec![]).await.unwrap(),
            Vec::<i64>::new()
        );
        assert_eq!(
            get_expanded_folders(&pool).await.unwrap(),
            Vec::<i64>::new()
        );
    }

    #[tokio::test]
    async fn expanded_folders_ignore_stale_unknown_and_duplicate_ids() {
        let pool = crate::test_db().await;
        insert_folders(&pool, &[1, 2, 3]).await;

        assert_eq!(
            set_expanded_folders(&pool, vec![2, 99, 2, 1])
                .await
                .unwrap(),
            vec![2, 1],
            "unknown and duplicate ids are not stored"
        );
        sqlx::query("DELETE FROM folders WHERE id = 2")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            get_expanded_folders(&pool).await.unwrap(),
            vec![1],
            "a folder deleted behind the setting's back is skipped"
        );
    }

    #[tokio::test]
    async fn expanded_folders_fall_back_to_empty_for_invalid_stored_value() {
        let pool = crate::test_db().await;
        insert_folders(&pool, &[1]).await;

        for stored in ["", "nope", "{\"a\":1}", "[1,\"x\"]"] {
            sqlx::query(
                "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
                 ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
            )
            .bind(super::EXPANDED_FOLDERS_KEY)
            .bind(stored)
            .execute(&pool)
            .await
            .unwrap();

            assert_eq!(
                get_expanded_folders(&pool).await.unwrap(),
                Vec::<i64>::new(),
                "stored value {stored:?} should read as empty"
            );
        }
    }
}
