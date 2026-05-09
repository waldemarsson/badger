// src-tauri/src/lib.rs
pub mod app_settings;
pub mod commands;
pub mod db;
pub mod error;
pub mod logger;
pub mod runtime_paths;
pub mod startup_backup;

#[cfg(debug_assertions)]
pub mod dev_seed;

use std::path::PathBuf;
use tauri::Manager;

pub use error::AppError;

/// In-memory test fixture used by all command test modules.
///
/// `pub(crate)` at crate root so submodules (commands::customers, commands::projects)
/// can call `crate::test_db()`.  Still `#[cfg(test)]` — not compiled in release.
#[cfg(test)]
pub(crate) async fn test_db() -> sqlx::SqlitePool {
    use sqlx::sqlite::SqlitePoolOptions;

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory SQLite pool");

    // raw_sql handles the multi-statement DDL in SCHEMA_SQL
    sqlx::raw_sql(crate::db::SCHEMA_SQL)
        .execute(&pool)
        .await
        .expect("Schema init failed");

    pool
}

async fn run_startup_backup(
    db_path: &PathBuf,
    pool: &sqlx::SqlitePool,
) -> Result<(), AppError> {
    let retention = app_settings::get_startup_backup_retention(pool).await?;
    if retention == 0 {
        tracing::debug!("Startup backup disabled (retention = 0)");
        return Ok(());
    }

    let json = commands::backup::inner_backup_data(pool).await?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let backup_path = startup_backup::write_daily_backup(db_path, &today, &json)?;
    tracing::info!("Startup backup written to: {:?}", backup_path);

    startup_backup::trim_old_backups(db_path, retention as usize)?;
    tracing::debug!("Startup backup trimmed to {} most recent", retention);

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // ── Logging ──────────────────────────────────────────────────────
            // init_logger() returns a WorkerGuard. It MUST be stored in managed
            // state — dropping it here would stop log writes before app exits.
            let home = app.path().home_dir()?;
            let runtime_paths = runtime_paths::runtime_paths(&home);
            let guard = logger::init_logger(&runtime_paths.log_dir);
            app.manage(guard);

            // ── Database ─────────────────────────────────────────────────────
            // tauri::async_runtime::block_on() reuses Tauri's embedded Tokio
            // runtime — correct for async work inside this sync setup closure.
            // Fallback if this import fails:
            //   tokio::runtime::Builder::new_current_thread()
            //       .build().unwrap().block_on(db::init_db(&db_path))
            let pool = tauri::async_runtime::block_on(db::init_db(&runtime_paths.db_path))?;
            app.manage(pool.clone());
            app.manage(runtime_paths.db_path.clone());

            tracing::info!("TimeTracker started. DB: {:?}", runtime_paths.db_path);

            // ── Startup Backup ───────────────────────────────────────────────
            // Background task writes daily backup if retention > 0.
            // Non-blocking: spawns onto Tauri's async runtime, logs any errors.
            let db_path_for_backup = runtime_paths.db_path.clone();
            let pool_for_backup = pool.clone();
            tauri::async_runtime::spawn(async move {
                match run_startup_backup(&db_path_for_backup, &pool_for_backup).await {
                    Ok(()) => {}
                    Err(e) => tracing::error!("Startup backup failed: {}", e),
                }
            });

            Ok(())
        })
        .invoke_handler({
            #[cfg(debug_assertions)]
            {
                tauri::generate_handler![
                    // Customers (BE-06, BE-08)
                    commands::customers::list_customers,
                    commands::customers::create_customer,
                    commands::customers::delete_customer,
                    commands::customers::set_default_customer,
                    // Projects (BE-06, BE-08)
                    commands::projects::list_projects,
                    commands::projects::create_project,
                    commands::projects::delete_project,
                    commands::projects::set_default_project,
                    // Issues (BE-04)
                    commands::issues::list_issues,
                    // Time Entries (BE-04)
                    commands::time_entries::list_time_entries,
                    commands::time_entries::create_time_entry,
                    commands::time_entries::update_time_entry,
                    commands::time_entries::delete_time_entry,
                    // Reports (BE-05)
                    commands::reports::weekly_report,
                    commands::reports::monthly_report,
                    // Time Entries helper (Phase 4 delete guard)
                    commands::time_entries::count_project_time_entries,
                    // Backup (BE-07)
                    commands::backup::backup_data_to_file,
                    commands::backup::read_backup_file,
                    commands::backup::restore_data,
                    commands::backup::get_startup_backup_retention,
                    commands::backup::set_startup_backup_retention,
                    commands::backup::list_startup_backups,
                    // Dev-only
                    dev_seed::seed_database,
                ]
            }
            #[cfg(not(debug_assertions))]
            {
                tauri::generate_handler![
                    // Customers (BE-06, BE-08)
                    commands::customers::list_customers,
                    commands::customers::create_customer,
                    commands::customers::delete_customer,
                    commands::customers::set_default_customer,
                    // Projects (BE-06, BE-08)
                    commands::projects::list_projects,
                    commands::projects::create_project,
                    commands::projects::delete_project,
                    commands::projects::set_default_project,
                    // Issues (BE-04)
                    commands::issues::list_issues,
                    // Time Entries (BE-04)
                    commands::time_entries::list_time_entries,
                    commands::time_entries::create_time_entry,
                    commands::time_entries::update_time_entry,
                    commands::time_entries::delete_time_entry,
                    // Reports (BE-05)
                    commands::reports::weekly_report,
                    commands::reports::monthly_report,
                    // Time Entries helper (Phase 4 delete guard)
                    commands::time_entries::count_project_time_entries,
                    // Backup (BE-07)
                    commands::backup::backup_data_to_file,
                    commands::backup::read_backup_file,
                    commands::backup::restore_data,
                    commands::backup::get_startup_backup_retention,
                    commands::backup::set_startup_backup_retention,
                    commands::backup::list_startup_backups,
                ]
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_db_opens() {
        let pool = crate::test_db().await;
        let row: (i64,) = sqlx::query_as("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("query failed");
        assert_eq!(row.0, 1);
    }
}
