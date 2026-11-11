use chrono::NaiveDate;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{commands::backup::encode_backup, AppError};

/// File name of a daily backup (gzipped JSON).
const BACKUP_FILE: &str = "backup.json.gz";

/// File name of daily backups written before gzip; still listed, trimmed and replaced.
const LEGACY_BACKUP_FILE: &str = "backup.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StoredBackupSummary {
    pub date: String,
    pub size_bytes: u64,
}

pub fn backup_root_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .expect("database path should have a parent")
        .join("backups")
}

fn daily_backup_path(backup_root: &Path, date: NaiveDate) -> PathBuf {
    backup_root
        .join(date.format("%Y-%m-%d").to_string())
        .join(BACKUP_FILE)
}

pub fn write_daily_backup(db_path: &Path, date: &str, json: &str) -> Result<PathBuf, AppError> {
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|err| AppError::Custom(format!("Invalid backup date '{date}': {err}")))?;
    let backup_root = backup_root_dir(db_path);
    let backup_path = daily_backup_path(&backup_root, date);
    let backup_dir = backup_path
        .parent()
        .expect("backup file path should have a parent directory");

    let bytes = encode_backup(json)?;

    fs::create_dir_all(backup_dir)?;

    // Write atomically: write to temp file in same directory, then rename.
    // This preserves the previous valid backup if the write fails,
    // and ensures we never leave a partially written backup in place.
    let temp_path = backup_dir.join(".backup.json.gz.tmp");
    fs::write(&temp_path, bytes)?;
    fs::rename(&temp_path, &backup_path)?;

    // Today's gzip backup supersedes a pre-gzip one written earlier the same day.
    // The new backup is already in place, so failing to clean up is not an error.
    let legacy_path = backup_dir.join(LEGACY_BACKUP_FILE);
    match fs::remove_file(&legacy_path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!(
                "Startup backup: could not remove {:?}: {}",
                legacy_path,
                err
            );
        }
        _ => {}
    }

    Ok(backup_path)
}

/// True when no backup has been written yet for `today` in this run of the app.
pub fn backup_due(last_backup: Option<NaiveDate>, today: NaiveDate) -> bool {
    last_backup != Some(today)
}

pub fn trim_old_backups(db_path: &Path, keep_latest: usize) -> Result<(), AppError> {
    let backup_root = backup_root_dir(db_path);
    let mut dated_dirs = dated_backup_directories(&backup_root)?;
    dated_dirs.sort_by(|(left, ..), (right, ..)| right.cmp(left));

    for (_, path, _) in dated_dirs.into_iter().skip(keep_latest) {
        fs::remove_dir_all(path)?;
    }

    Ok(())
}

pub fn list_startup_backups(db_path: &Path) -> Result<Vec<StoredBackupSummary>, AppError> {
    let backup_root = backup_root_dir(db_path);
    let mut dated_dirs = dated_backup_directories(&backup_root)?;
    dated_dirs.sort_by(|(left, ..), (right, ..)| right.cmp(left));

    dated_dirs
        .into_iter()
        .map(|(date, _, backup_path)| {
            let metadata = fs::metadata(&backup_path)?;

            Ok(StoredBackupSummary {
                date: date.format("%Y-%m-%d").to_string(),
                size_bytes: metadata.len(),
            })
        })
        .collect()
}

/// The backup file in a dated directory: `backup.json.gz`, else a legacy `backup.json`.
fn stored_backup_file(dir: &Path) -> Option<PathBuf> {
    [BACKUP_FILE, LEGACY_BACKUP_FILE]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// Dated backup directories as `(date, directory, backup file)`.
fn dated_backup_directories(
    backup_root: &Path,
) -> Result<Vec<(NaiveDate, PathBuf, PathBuf)>, AppError> {
    if !backup_root.exists() {
        return Ok(Vec::new());
    }

    let mut dated_dirs = Vec::new();
    for entry in fs::read_dir(backup_root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let Ok(date) = NaiveDate::parse_from_str(name, "%Y-%m-%d") else {
            continue;
        };

        let Some(backup_path) = stored_backup_file(&path) else {
            continue;
        };

        dated_dirs.push((date, path, backup_path));
    }

    Ok(dated_dirs)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use chrono::NaiveDate;

    use super::{
        backup_due, backup_root_dir, list_startup_backups, trim_old_backups, write_daily_backup,
        StoredBackupSummary,
    };
    use crate::commands::backup::{decode_backup, encode_backup};

    /// On-disk size of `json` as written by `write_daily_backup`.
    fn gz_len(json: &str) -> u64 {
        encode_backup(json).expect("encoding should succeed").len() as u64
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
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-artifacts")
            .join(format!("{name}-{unique}"))
    }

    #[test]
    fn backup_is_due_at_launch_and_on_each_new_day() {
        let monday = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let tuesday = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();

        assert!(backup_due(None, monday), "launch backs up");
        assert!(
            !backup_due(Some(monday), monday),
            "same day does not repeat"
        );
        assert!(backup_due(Some(monday), tuesday), "new day backs up");
    }

    #[test]
    fn startup_backup_root_is_sibling_of_database_file() {
        let db_path = Path::new("/workspace/.badger-dev/badger.db");

        assert_eq!(
            backup_root_dir(db_path),
            Path::new("/workspace/.badger-dev/backups")
        );
    }

    #[test]
    fn startup_backup_overwrites_same_day_backup_file() {
        let test_dir = unique_test_dir("startup-backup-overwrite");
        let db_path = test_dir.join("badger.db");

        let first_path = write_daily_backup(&db_path, "2026-05-09", r#"{"version":1}"#)
            .expect("first backup write should succeed");
        let second_path = write_daily_backup(&db_path, "2026-05-09", r#"{"version":2}"#)
            .expect("second backup write should succeed");

        assert_eq!(first_path, second_path);
        assert!(second_path.ends_with("2026-05-09/backup.json.gz"));
        assert_eq!(read_backup(&second_path), r#"{"version":2}"#);

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_trims_old_backups_to_newest_days() {
        let test_dir = unique_test_dir("startup-backup-trim");
        let db_path = test_dir.join("badger.db");

        write_daily_backup(&db_path, "2026-05-07", r#"{"day":"oldest"}"#)
            .expect("oldest backup write should succeed");
        write_daily_backup(&db_path, "2026-05-08", r#"{"day":"middle"}"#)
            .expect("middle backup write should succeed");
        write_daily_backup(&db_path, "2026-05-09", r#"{"day":"newest"}"#)
            .expect("newest backup write should succeed");

        trim_old_backups(&db_path, 2).expect("trim should succeed");

        assert_eq!(
            list_startup_backups(&db_path).expect("listing backups should succeed"),
            vec![
                StoredBackupSummary {
                    date: "2026-05-09".to_string(),
                    size_bytes: gz_len(r#"{"day":"newest"}"#),
                },
                StoredBackupSummary {
                    date: "2026-05-08".to_string(),
                    size_bytes: gz_len(r#"{"day":"middle"}"#),
                },
            ]
        );
        assert!(!backup_root_dir(&db_path).join("2026-05-07").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_trim_ignores_dated_directories_without_backup_file() {
        let test_dir = unique_test_dir("startup-backup-trim-ignores-empty-dates");
        let db_path = test_dir.join("badger.db");
        let backup_root = backup_root_dir(&db_path);

        write_daily_backup(&db_path, "2026-05-08", r#"{"day":"older"}"#)
            .expect("older backup write should succeed");
        write_daily_backup(&db_path, "2026-05-09", r#"{"day":"newer"}"#)
            .expect("newer backup write should succeed");
        fs::create_dir_all(backup_root.join("2026-05-10"))
            .expect("empty dated directory should be created");

        trim_old_backups(&db_path, 1).expect("trim should succeed");

        assert_eq!(
            list_startup_backups(&db_path).expect("listing backups should succeed"),
            vec![StoredBackupSummary {
                date: "2026-05-09".to_string(),
                size_bytes: gz_len(r#"{"day":"newer"}"#),
            }]
        );
        assert!(!backup_root.join("2026-05-08").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_trim_with_zero_keeps_no_backups() {
        let test_dir = unique_test_dir("startup-backup-trim-zero");
        let db_path = test_dir.join("badger.db");

        write_daily_backup(&db_path, "2026-05-08", r#"{"day":"older"}"#)
            .expect("older backup write should succeed");
        write_daily_backup(&db_path, "2026-05-09", r#"{"day":"newer"}"#)
            .expect("newer backup write should succeed");

        trim_old_backups(&db_path, 0).expect("trim should succeed");

        assert!(list_startup_backups(&db_path)
            .expect("listing backups should succeed")
            .is_empty());
        assert!(!backup_root_dir(&db_path).join("2026-05-08").exists());
        assert!(!backup_root_dir(&db_path).join("2026-05-09").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_listing_is_empty_when_directory_is_missing() {
        let test_dir = unique_test_dir("startup-backup-empty-list");
        let db_path = test_dir.join("badger.db");

        assert!(list_startup_backups(&db_path)
            .expect("listing missing backups directory should succeed")
            .is_empty());

        if test_dir.exists() {
            fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
        }
    }

    #[test]
    fn startup_backup_atomic_write_leaves_no_temp_file() {
        let test_dir = unique_test_dir("startup-backup-atomic");
        let db_path = test_dir.join("badger.db");

        write_daily_backup(&db_path, "2026-05-09", r#"{"version":1}"#)
            .expect("backup write should succeed");

        let backup_dir = backup_root_dir(&db_path).join("2026-05-09");
        let temp_file = backup_dir.join(".backup.json.gz.tmp");

        assert!(
            !temp_file.exists(),
            "temp file should not exist after write"
        );
        assert!(
            backup_dir.join("backup.json.gz").exists(),
            "backup.json.gz should exist"
        );

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_overwrite_preserves_file_on_same_day() {
        let test_dir = unique_test_dir("startup-backup-preserve");
        let db_path = test_dir.join("badger.db");

        // Write first backup
        write_daily_backup(&db_path, "2026-05-09", r#"{"version":1}"#)
            .expect("first backup write should succeed");

        let backup_path = backup_root_dir(&db_path)
            .join("2026-05-09")
            .join("backup.json.gz");
        let first_content = read_backup(&backup_path);

        assert_eq!(first_content, r#"{"version":1}"#);

        // Overwrite with second backup
        write_daily_backup(&db_path, "2026-05-09", r#"{"version":2}"#)
            .expect("second backup write should succeed");

        let second_content = read_backup(&backup_path);

        assert_eq!(second_content, r#"{"version":2}"#);

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_writes_gzip_of_the_json() {
        let test_dir = unique_test_dir("startup-backup-gzip");
        let db_path = test_dir.join("badger.db");
        let json = r#"{"notes":[{"title":"Å 🦡"}]}"#;

        let path =
            write_daily_backup(&db_path, "2026-05-09", json).expect("backup write should succeed");

        let bytes = fs::read(&path).expect("backup file should exist");
        assert_eq!(&bytes[..2], &[0x1f, 0x8b], "gzip magic");
        assert_eq!(read_backup(&path), json);

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_same_day_rewrite_removes_legacy_json() {
        let test_dir = unique_test_dir("startup-backup-legacy-rewrite");
        let db_path = test_dir.join("badger.db");
        let backup_dir = backup_root_dir(&db_path).join("2026-05-09");
        fs::create_dir_all(&backup_dir).expect("dated directory should be created");
        fs::write(backup_dir.join("backup.json"), r#"{"version":1}"#)
            .expect("legacy backup should be written");

        write_daily_backup(&db_path, "2026-05-09", r#"{"version":2}"#)
            .expect("backup write should succeed");

        assert!(!backup_dir.join("backup.json").exists());
        assert_eq!(
            read_backup(&backup_dir.join("backup.json.gz")),
            r#"{"version":2}"#
        );

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_write_succeeds_when_legacy_json_cannot_be_removed() {
        let test_dir = unique_test_dir("startup-backup-legacy-unremovable");
        let db_path = test_dir.join("badger.db");
        let backup_dir = backup_root_dir(&db_path).join("2026-05-09");
        // A directory named backup.json makes `remove_file` fail with something other
        // than NotFound.
        fs::create_dir_all(backup_dir.join("backup.json/inner"))
            .expect("blocking directory should be created");

        let path = write_daily_backup(&db_path, "2026-05-09", r#"{"version":2}"#)
            .expect("backup write should succeed despite the leftover");

        assert_eq!(read_backup(&path), r#"{"version":2}"#);
        assert!(backup_dir.join("backup.json").is_dir());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_listing_and_trim_handle_legacy_and_mixed_dirs() {
        let test_dir = unique_test_dir("startup-backup-legacy-mixed");
        let db_path = test_dir.join("badger.db");
        let backup_root = backup_root_dir(&db_path);
        let legacy = r#"{"day":"legacy"}"#;

        // 05-07: legacy only. 05-08: both files (gz wins). 05-09: gz only.
        fs::create_dir_all(backup_root.join("2026-05-07")).expect("dir should be created");
        fs::write(backup_root.join("2026-05-07/backup.json"), legacy)
            .expect("legacy backup should be written");
        write_daily_backup(&db_path, "2026-05-08", r#"{"day":"mixed"}"#)
            .expect("mixed backup write should succeed");
        fs::write(backup_root.join("2026-05-08/backup.json"), legacy)
            .expect("legacy backup should be written");
        write_daily_backup(&db_path, "2026-05-09", r#"{"day":"newest"}"#)
            .expect("newest backup write should succeed");

        assert_eq!(
            list_startup_backups(&db_path).expect("listing backups should succeed"),
            vec![
                StoredBackupSummary {
                    date: "2026-05-09".to_string(),
                    size_bytes: gz_len(r#"{"day":"newest"}"#),
                },
                StoredBackupSummary {
                    date: "2026-05-08".to_string(),
                    size_bytes: gz_len(r#"{"day":"mixed"}"#),
                },
                StoredBackupSummary {
                    date: "2026-05-07".to_string(),
                    size_bytes: legacy.len() as u64,
                },
            ]
        );

        trim_old_backups(&db_path, 1).expect("trim should succeed");

        assert!(!backup_root.join("2026-05-07").exists());
        assert!(!backup_root.join("2026-05-08").exists());
        assert!(backup_root.join("2026-05-09").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }
}
