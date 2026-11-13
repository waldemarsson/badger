//! Automatic daily backups: one flat file per date in a single backup folder.
//!
//! Files are named `badger-YYYY-MM-DD.json.gz` (gzipped JSON) or
//! `badger-YYYY-MM-DD.json.gz.age` (the same, encrypted with `age`). Listing, trimming and
//! deleting only ever touch files matching that pattern, so the folder can be shared with
//! other files (a synced iCloud Drive or Dropbox folder).

use chrono::NaiveDate;
use sqlx::{Pool, Sqlite};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{app_settings, backup_crypto, commands::backup::encode_backup, AppError};

const FILE_PREFIX: &str = "badger-";
const PLAIN_SUFFIX: &str = ".json.gz";
const ENCRYPTED_SUFFIX: &str = ".json.gz.age";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StoredBackupSummary {
    pub date: String,
    pub size_bytes: u64,
    pub encrypted: bool,
}

/// Outcome of the most recent backup attempts in this run of the app.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct BackupStatus {
    /// Local time of the last successful backup (`%Y-%m-%dT%H:%M:%S`).
    pub last_success: Option<String>,
    /// Error of the last attempt, cleared by the next success.
    pub last_error: Option<String>,
}

/// Managed state: serialises backup writes (daily task and "Back up now") and keeps status.
#[derive(Default)]
pub struct BackupState {
    write_lock: tokio::sync::Mutex<()>,
    status: Mutex<BackupStatus>,
}

impl BackupState {
    pub fn status(&self) -> BackupStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn record(&self, result: &Result<Option<PathBuf>, AppError>) {
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        match result {
            Ok(Some(_)) => {
                status.last_success =
                    Some(chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string());
                status.last_error = None;
            }
            Ok(None) => {}
            Err(e) => status.last_error = Some(e.to_string()),
        }
    }
}

/// The default backup folder: `backups/` beside the database.
pub fn backup_root_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map(|dir| dir.join("backups"))
        .unwrap_or_else(|| PathBuf::from("backups"))
}

/// Where backups go: the configured folder, else the default.
pub async fn resolve_backup_dir(db_path: &Path, pool: &Pool<Sqlite>) -> Result<PathBuf, AppError> {
    Ok(app_settings::get_backup_dir(pool)
        .await?
        .map(PathBuf::from)
        .unwrap_or_else(|| backup_root_dir(db_path)))
}

pub fn backup_file_name(date: NaiveDate, encrypted: bool) -> String {
    let suffix = if encrypted {
        ENCRYPTED_SUFFIX
    } else {
        PLAIN_SUFFIX
    };
    format!("{FILE_PREFIX}{}{suffix}", date.format("%Y-%m-%d"))
}

/// `(date, encrypted)` for a Badger backup file name, `None` for anything else.
pub fn parse_backup_file_name(name: &str) -> Option<(NaiveDate, bool)> {
    let rest = name.strip_prefix(FILE_PREFIX)?;
    let (date, encrypted) = if let Some(date) = rest.strip_suffix(ENCRYPTED_SUFFIX) {
        (date, true)
    } else {
        (rest.strip_suffix(PLAIN_SUFFIX)?, false)
    };
    let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    // Strict: `2026-1-5` parses but is not a name Badger writes.
    (parsed.format("%Y-%m-%d").to_string() == date).then_some((parsed, encrypted))
}

/// True when no backup has been written yet for `today` in this run of the app.
pub fn backup_due(last_backup: Option<NaiveDate>, today: NaiveDate) -> bool {
    last_backup != Some(today)
}

/// Writes today's backup file atomically (dot-prefixed temp file, then rename) and removes
/// the other variant (plain ↔ encrypted) for the same date. `dir` must already exist.
pub fn write_daily_backup(
    dir: &Path,
    date: NaiveDate,
    bytes: &[u8],
    encrypted: bool,
) -> Result<PathBuf, AppError> {
    if !dir.is_dir() {
        return Err(AppError::Custom(format!(
            "Backup folder not found: {}",
            dir.display()
        )));
    }
    let path = dir.join(backup_file_name(date, encrypted));
    let temp_path = dir.join(format!(
        ".{}.{}.tmp",
        backup_file_name(date, encrypted),
        std::process::id()
    ));
    if let Err(e) = fs::write(&temp_path, bytes).and_then(|()| fs::rename(&temp_path, &path)) {
        let _ = fs::remove_file(&temp_path);
        return Err(e.into());
    }

    // The new file is in place, so failing to remove the other variant is not an error.
    let other = dir.join(backup_file_name(date, !encrypted));
    if let Err(e) = fs::remove_file(&other) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Backup: could not remove {:?}: {}", other, e);
        }
    }
    Ok(path)
}

/// Badger backup files in `dir` as `(date, encrypted, path)`; empty when `dir` is missing.
fn backup_files(dir: &Path) -> Result<Vec<(NaiveDate, bool, PathBuf)>, AppError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let Some((date, encrypted)) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(parse_backup_file_name)
        else {
            continue;
        };
        if path.is_file() {
            files.push((date, encrypted, path));
        }
    }
    // Newest first; for one date (only after a failed cleanup) the encrypted file first.
    files.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    Ok(files)
}

/// Keeps the files of the `keep_latest` newest dates and deletes older Badger backups.
pub fn trim_old_backups(dir: &Path, keep_latest: usize) -> Result<(), AppError> {
    let files = backup_files(dir)?;
    let mut dates: Vec<NaiveDate> = files.iter().map(|(d, ..)| *d).collect();
    dates.dedup();
    let Some(&oldest_kept) = dates.get(keep_latest.saturating_sub(1)) else {
        return Ok(());
    };
    for (date, _, path) in files {
        if keep_latest == 0 || date < oldest_kept {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// One entry per date, newest first.
pub fn list_backups(dir: &Path) -> Result<Vec<StoredBackupSummary>, AppError> {
    let mut summaries: Vec<StoredBackupSummary> = Vec::new();
    for (date, encrypted, path) in backup_files(dir)? {
        let date = date.format("%Y-%m-%d").to_string();
        if summaries.last().is_some_and(|s| s.date == date) {
            continue;
        }
        summaries.push(StoredBackupSummary {
            date,
            size_bytes: fs::metadata(&path)?.len(),
            encrypted,
        });
    }
    Ok(summaries)
}

/// Deletes the unencrypted Badger backups in `dir`; returns how many were deleted.
pub fn delete_unencrypted_backups(dir: &Path) -> Result<usize, AppError> {
    let mut deleted = 0;
    for (_, encrypted, path) in backup_files(dir)? {
        if !encrypted {
            fs::remove_file(path)?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// Checks that `dir` is an existing, writable folder by writing and deleting a probe file.
pub fn check_writable_dir(dir: &Path) -> Result<(), AppError> {
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(AppError::Custom(format!(
            "Backup folder not found: {}",
            dir.display()
        )));
    }
    let probe = dir.join(format!(".badger-write-check.{}.tmp", std::process::id()));
    fs::write(&probe, b"")
        .and_then(|()| fs::remove_file(&probe))
        .map_err(|e| {
            AppError::Custom(format!(
                "Can't write to the backup folder {}: {e}",
                dir.display()
            ))
        })
}

/// Writes today's backup with the current folder and encryption settings, then trims.
///
/// Returns `Ok(None)` when automatic backups are off (retention 0) and `force` is false.
/// `force` ("Back up now") writes even then, but trims only when retention > 0.
/// Never falls back to another folder or to plaintext.
pub async fn run_backup(
    db_path: &Path,
    pool: &Pool<Sqlite>,
    state: &BackupState,
    today: NaiveDate,
    force: bool,
) -> Result<Option<PathBuf>, AppError> {
    let _guard = state.write_lock.lock().await;
    let result = write_backup(db_path, pool, today, force).await;
    if let Err(e) = &result {
        tracing::error!("Backup failed: {}", e);
    }
    state.record(&result);
    result
}

async fn write_backup(
    db_path: &Path,
    pool: &Pool<Sqlite>,
    today: NaiveDate,
    force: bool,
) -> Result<Option<PathBuf>, AppError> {
    let retention = app_settings::get_startup_backup_retention(pool).await?;
    if retention == 0 && !force {
        tracing::debug!("Automatic backups disabled (retention = 0)");
        return Ok(None);
    }

    let custom_dir = app_settings::get_backup_dir(pool).await?;
    let dir = match &custom_dir {
        Some(dir) => PathBuf::from(dir),
        None => {
            // Only the default folder is created; a missing custom folder (an unmounted
            // drive, a sync folder not set up yet) is an error, not a new local folder.
            let dir = backup_root_dir(db_path);
            fs::create_dir_all(&dir)?;
            dir
        }
    };
    let public_key = app_settings::get_backup_public_key(pool).await?;

    let json = crate::commands::backup::inner_backup_data(pool).await?;
    let gz = encode_backup(&json)?;
    let (bytes, encrypted) = match &public_key {
        Some(key) => (backup_crypto::encrypt(key, &gz)?, true),
        None => (gz, false),
    };
    let path = write_daily_backup(&dir, today, &bytes, encrypted)?;
    tracing::info!("Backup written to: {:?}", path);

    if retention > 0 {
        if let Err(e) = trim_old_backups(&dir, retention as usize) {
            tracing::error!("Backup: failed to trim old backups: {}", e);
        }
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use chrono::NaiveDate;

    use super::*;
    use crate::commands::backup::{decode_backup, encode_backup};

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn write(dir: &Path, day: &str, json: &str) -> PathBuf {
        write_daily_backup(dir, date(day), &encode_backup(json).unwrap(), false)
            .expect("backup write should succeed")
    }

    fn read_backup(path: &Path) -> String {
        decode_backup(fs::read(path).expect("backup file should exist"))
            .expect("backup file should decode")
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-artifacts")
            .join(format!("{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn backup_is_due_at_launch_and_on_each_new_day() {
        let monday = date("2026-11-09");
        let tuesday = date("2026-11-10");
        assert!(backup_due(None, monday), "launch backs up");
        assert!(
            !backup_due(Some(monday), monday),
            "same day does not repeat"
        );
        assert!(backup_due(Some(monday), tuesday), "new day backs up");
    }

    #[test]
    fn default_backup_dir_is_sibling_of_database_file() {
        assert_eq!(
            backup_root_dir(Path::new("/workspace/.badger-dev/badger.db")),
            Path::new("/workspace/.badger-dev/backups")
        );
    }

    #[test]
    fn file_names_parse_strictly() {
        let d = date("2026-05-09");
        assert_eq!(backup_file_name(d, false), "badger-2026-05-09.json.gz");
        assert_eq!(backup_file_name(d, true), "badger-2026-05-09.json.gz.age");
        assert_eq!(
            parse_backup_file_name("badger-2026-05-09.json.gz"),
            Some((d, false))
        );
        assert_eq!(
            parse_backup_file_name("badger-2026-05-09.json.gz.age"),
            Some((d, true))
        );
        for bad in [
            "badger-2026-5-9.json.gz",
            "badger-2026-02-30.json.gz",
            "badger-2026-05-09 (1).json.gz",
            "badger-2026-05-09.json",
            "backup.json.gz",
            ".badger-2026-05-09.json.gz.123.tmp",
            "xbadger-2026-05-09.json.gz",
        ] {
            assert_eq!(parse_backup_file_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn write_is_gzip_overwrites_same_day_and_leaves_no_temp_file() {
        let dir = unique_test_dir("backup-write");
        let json = r#"{"notes":[{"title":"Å 🦡"}]}"#;
        write(&dir, "2026-05-09", r#"{"version":1}"#);
        let path = write(&dir, "2026-05-09", json);

        assert!(path.ends_with("badger-2026-05-09.json.gz"));
        assert_eq!(&fs::read(&path).unwrap()[..2], &[0x1f, 0x8b], "gzip magic");
        assert_eq!(read_backup(&path), json);
        assert_eq!(names(&dir), vec!["badger-2026-05-09.json.gz"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_fails_when_folder_is_missing() {
        let dir = unique_test_dir("backup-missing").join("gone");
        let err = write_daily_backup(&dir, date("2026-05-09"), b"x", false).unwrap_err();
        assert!(err.to_string().starts_with("Backup folder not found"));
        assert!(!dir.exists(), "a missing folder is not created");
    }

    #[test]
    fn writing_one_variant_removes_the_other_for_the_same_date() {
        let dir = unique_test_dir("backup-variant");
        write(&dir, "2026-05-08", "{}");
        write(&dir, "2026-05-09", "{}");
        write_daily_backup(&dir, date("2026-05-09"), b"age", true).unwrap();
        assert_eq!(
            names(&dir),
            vec!["badger-2026-05-08.json.gz", "badger-2026-05-09.json.gz.age"]
        );
        write(&dir, "2026-05-09", "{}");
        assert_eq!(
            names(&dir),
            vec!["badger-2026-05-08.json.gz", "badger-2026-05-09.json.gz"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trim_keeps_newest_dates_and_never_touches_foreign_files() {
        let dir = unique_test_dir("backup-trim");
        write(&dir, "2026-05-07", "{}");
        write(&dir, "2026-05-08", "{}");
        write_daily_backup(&dir, date("2026-05-09"), b"age", true).unwrap();
        // Foreign files and old dated folders stay.
        fs::write(dir.join("notes.txt"), "mine").unwrap();
        fs::write(dir.join("badger-2026-05-01 (1).json.gz"), "conflict copy").unwrap();
        fs::create_dir_all(dir.join("2026-05-01")).unwrap();
        fs::write(dir.join("2026-05-01/backup.json.gz"), "old layout").unwrap();

        trim_old_backups(&dir, 2).unwrap();

        assert_eq!(
            names(&dir),
            vec![
                "2026-05-01",
                "badger-2026-05-01 (1).json.gz",
                "badger-2026-05-08.json.gz",
                "badger-2026-05-09.json.gz.age",
                "notes.txt",
            ]
        );
        assert!(dir.join("2026-05-01/backup.json.gz").exists());

        trim_old_backups(&dir, 0).unwrap();
        assert_eq!(
            names(&dir),
            vec!["2026-05-01", "badger-2026-05-01 (1).json.gz", "notes.txt"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_is_newest_first_with_encryption_flag_and_ignores_old_layout() {
        let dir = unique_test_dir("backup-list");
        let plain = write(&dir, "2026-05-08", r#"{"day":"older"}"#);
        write_daily_backup(&dir, date("2026-05-09"), b"encrypted!", true).unwrap();
        fs::create_dir_all(dir.join("2026-05-10")).unwrap();
        fs::write(dir.join("2026-05-10/backup.json.gz"), "old layout").unwrap();

        assert_eq!(
            list_backups(&dir).unwrap(),
            vec![
                StoredBackupSummary {
                    date: "2026-05-09".into(),
                    size_bytes: 10,
                    encrypted: true
                },
                StoredBackupSummary {
                    date: "2026-05-08".into(),
                    size_bytes: fs::metadata(plain).unwrap().len(),
                    encrypted: false
                },
            ]
        );
        assert!(list_backups(&dir.join("missing")).unwrap().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_unencrypted_removes_only_plain_badger_backups() {
        let dir = unique_test_dir("backup-delete-plain");
        write(&dir, "2026-05-07", "{}");
        write(&dir, "2026-05-08", "{}");
        write_daily_backup(&dir, date("2026-05-09"), b"age", true).unwrap();
        fs::write(dir.join("other.json.gz"), "mine").unwrap();

        assert_eq!(delete_unencrypted_backups(&dir).unwrap(), 2);
        assert_eq!(
            names(&dir),
            vec!["badger-2026-05-09.json.gz.age", "other.json.gz"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writable_check_accepts_folder_and_rejects_missing_or_relative() {
        let dir = unique_test_dir("backup-writable");
        check_writable_dir(&dir).expect("writable");
        assert!(names(&dir).is_empty(), "probe file removed");
        assert!(check_writable_dir(&dir.join("missing")).is_err());
        assert!(check_writable_dir(Path::new("relative/dir")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    async fn seed_pool() -> sqlx::SqlitePool {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO customers (Name, UseAsDefault) VALUES ('Acme', 0)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn run_backup_writes_to_default_folder() {
        let root = unique_test_dir("backup-run-default");
        let db_path = root.join("badger.db");
        let pool = seed_pool().await;
        let state = BackupState::default();

        let path = run_backup(&db_path, &pool, &state, date("2026-11-12"), false)
            .await
            .unwrap()
            .expect("written");
        assert_eq!(path, root.join("backups/badger-2026-11-12.json.gz"));
        assert!(read_backup(&path).contains("Acme"));
        assert!(state.status().last_success.is_some());
        assert_eq!(state.status().last_error, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn run_backup_encrypts_to_custom_folder_and_decrypts_with_the_key() {
        let root = unique_test_dir("backup-run-encrypted");
        let custom = root.join("Sync Folder å");
        fs::create_dir_all(&custom).unwrap();
        let pool = seed_pool().await;
        let key = backup_crypto::generate_key();
        app_settings::set_backup_dir(&pool, Some(custom.to_str().unwrap()))
            .await
            .unwrap();
        app_settings::set_backup_public_key(&pool, Some(&key.public_key))
            .await
            .unwrap();

        let path = run_backup(
            &root.join("badger.db"),
            &pool,
            &BackupState::default(),
            date("2026-11-12"),
            false,
        )
        .await
        .unwrap()
        .expect("written");

        assert_eq!(path, custom.join("badger-2026-11-12.json.gz.age"));
        assert!(!root.join("backups").exists(), "default folder untouched");
        let bytes = fs::read(&path).unwrap();
        assert!(backup_crypto::is_encrypted(&bytes));
        let gz = backup_crypto::decrypt(&key.secret_key, &bytes).unwrap();
        assert!(decode_backup(gz).unwrap().contains("Acme"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn run_backup_fails_without_fallback_when_custom_folder_is_gone() {
        let root = unique_test_dir("backup-run-gone");
        let gone = root.join("gone");
        let pool = seed_pool().await;
        app_settings::set_backup_dir(&pool, Some(gone.to_str().unwrap()))
            .await
            .unwrap();
        let state = BackupState::default();

        let err = run_backup(
            &root.join("badger.db"),
            &pool,
            &state,
            date("2026-11-12"),
            false,
        )
        .await
        .unwrap_err();

        assert!(err.to_string().starts_with("Backup folder not found"));
        assert_eq!(state.status().last_error, Some(err.to_string()));
        assert!(!gone.exists());
        assert!(
            !root.join("backups").exists(),
            "no fallback to the default folder"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn run_backup_with_bad_public_key_writes_nothing() {
        let root = unique_test_dir("backup-run-bad-key");
        let pool = seed_pool().await;
        // A hand-edited setting: never written by the app, which validates keys.
        sqlx::query(
            "INSERT INTO app_settings (Key, Value) VALUES ('backup_public_key', 'age1bad')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = run_backup(
            &root.join("badger.db"),
            &pool,
            &BackupState::default(),
            date("2026-11-12"),
            false,
        )
        .await;

        assert!(result.is_err());
        assert!(
            names(&root.join("backups")).is_empty(),
            "no plaintext fallback"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn retention_zero_skips_automatic_but_not_forced_backups() {
        let root = unique_test_dir("backup-run-retention-zero");
        let db_path = root.join("badger.db");
        let pool = seed_pool().await;
        app_settings::set_startup_backup_retention(&pool, 0)
            .await
            .unwrap();
        let state = BackupState::default();

        assert_eq!(
            run_backup(&db_path, &pool, &state, date("2026-11-12"), false)
                .await
                .unwrap(),
            None
        );
        assert!(!root.join("backups").exists());
        let forced = run_backup(&db_path, &pool, &state, date("2026-11-12"), true)
            .await
            .unwrap();
        assert!(forced.is_some_and(|p| p.exists()), "forced backup is kept");
        fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn concurrent_backups_do_not_interleave() {
        let root = unique_test_dir("backup-run-concurrent");
        let db_path = root.join("badger.db");
        let pool = seed_pool().await;
        let state = BackupState::default();
        let day = date("2026-11-12");

        let (a, b) = tokio::join!(
            run_backup(&db_path, &pool, &state, day, true),
            run_backup(&db_path, &pool, &state, day, true),
        );
        let path = a.unwrap().unwrap();
        assert_eq!(b.unwrap().unwrap(), path);
        assert!(read_backup(&path).contains("Acme"));
        assert_eq!(
            names(&root.join("backups")),
            vec!["badger-2026-11-12.json.gz"]
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
