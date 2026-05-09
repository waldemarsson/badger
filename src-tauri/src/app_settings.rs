use sqlx::{Pool, Sqlite};

use crate::AppError;

pub const STARTUP_BACKUP_RETENTION_KEY: &str = "startup_backup_retention";
pub const DEFAULT_STARTUP_BACKUP_RETENTION: i64 = 3;

fn validate_startup_backup_retention(retention: i64) -> Result<i64, AppError> {
    if retention < 0 {
        return Err(AppError::Custom(
            "Startup backup retention must be greater than or equal to 0.".to_string(),
        ));
    }

    Ok(retention)
}

fn parse_startup_backup_retention(value: &str) -> Result<i64, AppError> {
    let retention = value.parse::<i64>().map_err(|_| {
        AppError::Custom(format!(
            "Invalid startup backup retention value stored in app_settings: {value}"
        ))
    })?;

    validate_startup_backup_retention(retention)
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

#[cfg(test)]
mod tests {
    use super::{
        get_startup_backup_retention, set_startup_backup_retention,
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
}
