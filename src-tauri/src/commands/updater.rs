//! In-app updates from GitHub Releases (`tauri-plugin-updater`). The manifest and the signed
//! bundle are fetched and verified in Rust; the frontend only calls these commands.
//!
//! `AppHandle::restart` ignores `prevent_exit`, so the flush guard cannot hold a restart.
//! `install_update` therefore flushes unsaved notes itself before restarting.
//!
//! On macOS it does not use `restart` either: Tauri relaunches with `open -n` while this
//! process is still alive, so the new instance gets its own Dock tile beside a pinned one,
//! and keeps it. A detached shell instead waits for this process to exit, then runs a
//! plain `open` on the bundle, which LaunchServices attaches to the existing tile.
use crate::{flush_guard, AppError};
use serde::Serialize;
#[cfg(any(target_os = "macos", test))]
use std::path::Path;
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
    if spawn_relauncher() {
        app.exit(0); // the guard has flushed, so this exits at once
    } else {
        app.request_restart();
    }
    Ok(())
}

/// Waits (at most 30 s) for process `$1` to exit, then runs `$3 $2` (the opener on the
/// bundle). Detached from this process, so it outlives it.
#[cfg(any(target_os = "macos", all(test, unix)))]
const RELAUNCH_SCRIPT: &str = r#"i=0
while kill -0 "$1" 2>/dev/null && [ "$i" -lt 300 ]; do
  sleep 0.1
  i=$((i + 1))
done
exec "$3" "$2""#;

/// The `.app` bundle that contains `exe` (`…/Badger.app/Contents/MacOS/badger`).
#[cfg(any(target_os = "macos", test))]
fn bundle_root(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let root = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && root.extension()? == "app";
    is_bundle.then_some(root)
}

/// Starts the shell that runs `opener bundle` once process `pid` has exited.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn spawn_wait_then_open(
    pid: u32,
    bundle: &Path,
    opener: &Path,
) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    Command::new("/bin/sh")
        .arg("-c")
        .arg(RELAUNCH_SCRIPT)
        .arg("badger-relaunch")
        .arg(pid.to_string())
        .arg(bundle)
        .arg(opener)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
}

/// Relaunches through LaunchServices after exit (macOS app bundles only). `false` when it
/// could not, so the caller falls back to Tauri's restart.
fn spawn_relauncher() -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let Some(bundle) = bundle_root(&exe) else {
            return false;
        };
        match spawn_wait_then_open(std::process::id(), bundle, Path::new("/usr/bin/open")) {
            Ok(_) => return true,
            Err(e) => tracing::error!("Updater: relaunch helper not started: {}", e),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_app_bundle_of_the_executable() {
        assert_eq!(
            bundle_root(Path::new("/Applications/Badger.app/Contents/MacOS/badger")),
            Some(Path::new("/Applications/Badger.app"))
        );
        assert_eq!(bundle_root(Path::new("/repo/target/debug/badger")), None);
        assert_eq!(
            bundle_root(Path::new("/x/Badger/Contents/MacOS/badger")),
            None
        );
        assert_eq!(bundle_root(Path::new("badger")), None);
    }

    /// The helper opens the bundle only after the watched process has exited.
    #[cfg(unix)]
    #[test]
    fn relaunch_helper_waits_for_the_process_then_opens() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("badger-relaunch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("opened");
        let opener = dir.join("opener.sh");
        std::fs::write(
            &opener,
            format!("#!/bin/sh\necho \"$1\" > '{}'\n", out.display()),
        )
        .unwrap();
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&opener)
            .status()
            .unwrap();

        let mut watched = std::process::Command::new("sleep")
            .arg("0.5")
            .spawn()
            .unwrap();
        let pid = watched.id();
        // Reap it as soon as it exits: `kill -0` succeeds on a zombie.
        let reaper = std::thread::spawn(move || watched.wait());
        let start = Instant::now();
        let mut helper =
            spawn_wait_then_open(pid, Path::new("/Applications/Badger.app"), &opener).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!out.exists(), "opened while the process was still running");
        helper.wait().unwrap();
        reaper.join().unwrap().unwrap();
        assert!(start.elapsed() >= Duration::from_millis(400));
        assert_eq!(
            std::fs::read_to_string(&out).unwrap().trim(),
            "/Applications/Badger.app"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
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
