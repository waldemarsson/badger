//! In-app updates from GitHub Releases (`tauri-plugin-updater`). The manifest and the signed
//! bundle are fetched and verified in Rust; the frontend only calls these commands.
//!
//! `AppHandle::restart` ignores `prevent_exit`, so the flush guard cannot hold a restart.
//! `install_update` therefore flushes unsaved notes itself before restarting.
use crate::{flush_guard, AppError};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

/// An available update, as shown in Settings.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    /// Release notes from `latest.json`, if any.
    pub notes: Option<String>,
}

impl From<&Update> for UpdateInfo {
    fn from(update: &Update) -> Self {
        Self {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
            notes: update.body.clone().filter(|b| !b.trim().is_empty()),
        }
    }
}

/// One install at a time: a second click must not race the first over the app bundle.
static INSTALLING: AtomicBool = AtomicBool::new(false);

async fn find_update(app: &AppHandle) -> Result<Option<Update>, AppError> {
    let updater = app
        .updater()
        .map_err(|e| AppError::Custom(format!("Could not check for updates: {e}")))?;
    updater
        .check()
        .await
        .map_err(|e| AppError::Custom(format!("Could not check for updates: {e}")))
}

/// The newer release, or `None` when this version is the latest.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, AppError> {
    Ok(find_update(&app).await?.as_ref().map(UpdateInfo::from))
}

/// Downloads, verifies and installs the latest release, flushes unsaved notes, then restarts.
/// Returns only on failure; on success the app restarts into the new version.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), AppError> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(AppError::Custom(
            "An update is already being installed".to_string(),
        ));
    }
    let result = download_and_install(&app).await;
    if result.is_err() {
        INSTALLING.store(false, Ordering::SeqCst);
        return result;
    }
    flush_guard::flush_frontend(&app).await;
    app.request_restart();
    Ok(())
}

async fn download_and_install(app: &AppHandle) -> Result<(), AppError> {
    let Some(update) = find_update(app).await? else {
        return Err(AppError::Custom("Badger is already up to date".to_string()));
    };
    tracing::info!(
        "Updater: installing {} (current {})",
        update.version,
        update.current_version
    );
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| AppError::Custom(format!("Update failed: {e}")))
}
