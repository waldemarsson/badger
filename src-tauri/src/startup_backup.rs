use chrono::NaiveDate;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::AppError;

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
        .join("backup.json")
}

pub fn write_daily_backup(db_path: &Path, date: &str, json: &str) -> Result<PathBuf, AppError> {
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|err| AppError::Custom(format!("Invalid backup date '{date}': {err}")))?;
    let backup_root = backup_root_dir(db_path);
    let backup_path = daily_backup_path(&backup_root, date);
    let backup_dir = backup_path
        .parent()
        .expect("backup file path should have a parent directory");

    fs::create_dir_all(backup_dir)?;
    fs::write(&backup_path, json)?;

    Ok(backup_path)
}

pub fn trim_old_backups(db_path: &Path, keep_latest: usize) -> Result<(), AppError> {
    let backup_root = backup_root_dir(db_path);
    let mut dated_dirs = dated_backup_directories(&backup_root)?;
    dated_dirs.sort_by(|(left, _), (right, _)| right.cmp(left));

    for (_, path) in dated_dirs.into_iter().skip(keep_latest) {
        fs::remove_dir_all(path)?;
    }

    Ok(())
}

pub fn list_startup_backups(db_path: &Path) -> Result<Vec<StoredBackupSummary>, AppError> {
    let backup_root = backup_root_dir(db_path);
    let mut dated_dirs = dated_backup_directories(&backup_root)?;
    dated_dirs.sort_by(|(left, _), (right, _)| right.cmp(left));

    dated_dirs
        .into_iter()
        .map(|(date, path)| {
            let backup_path = path.join("backup.json");
            let metadata = fs::metadata(&backup_path)?;

            Ok(StoredBackupSummary {
                date: date.format("%Y-%m-%d").to_string(),
                size_bytes: metadata.len(),
            })
        })
        .collect()
}

fn dated_backup_directories(backup_root: &Path) -> Result<Vec<(NaiveDate, PathBuf)>, AppError> {
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

        let backup_path = path.join("backup.json");
        match fs::metadata(&backup_path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) | Err(_) => continue,
        }

        dated_dirs.push((date, path));
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

    use super::{
        backup_root_dir, list_startup_backups, trim_old_backups, write_daily_backup,
        StoredBackupSummary,
    };

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
    fn startup_backup_root_is_sibling_of_database_file() {
        let db_path = Path::new("/workspace/.timetracker-dev/timetracker.db");

        assert_eq!(
            backup_root_dir(db_path),
            Path::new("/workspace/.timetracker-dev/backups")
        );
    }

    #[test]
    fn startup_backup_overwrites_same_day_backup_file() {
        let test_dir = unique_test_dir("startup-backup-overwrite");
        let db_path = test_dir.join("timetracker.db");

        let first_path = write_daily_backup(&db_path, "2026-05-09", r#"{"version":1}"#)
            .expect("first backup write should succeed");
        let second_path = write_daily_backup(&db_path, "2026-05-09", r#"{"version":2}"#)
            .expect("second backup write should succeed");

        assert_eq!(first_path, second_path);
        assert_eq!(
            fs::read_to_string(&second_path).expect("backup file should exist"),
            r#"{"version":2}"#
        );

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_trims_old_backups_to_newest_days() {
        let test_dir = unique_test_dir("startup-backup-trim");
        let db_path = test_dir.join("timetracker.db");

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
                    size_bytes: r#"{"day":"newest"}"#.len() as u64,
                },
                StoredBackupSummary {
                    date: "2026-05-08".to_string(),
                    size_bytes: r#"{"day":"middle"}"#.len() as u64,
                },
            ]
        );
        assert!(!backup_root_dir(&db_path).join("2026-05-07").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_trim_ignores_dated_directories_without_backup_file() {
        let test_dir = unique_test_dir("startup-backup-trim-ignores-empty-dates");
        let db_path = test_dir.join("timetracker.db");
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
                size_bytes: r#"{"day":"newer"}"#.len() as u64,
            }]
        );
        assert!(!backup_root.join("2026-05-08").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_trim_with_zero_keeps_no_backups() {
        let test_dir = unique_test_dir("startup-backup-trim-zero");
        let db_path = test_dir.join("timetracker.db");

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
        let db_path = test_dir.join("timetracker.db");

        assert!(list_startup_backups(&db_path)
            .expect("listing missing backups directory should succeed")
            .is_empty());

        if test_dir.exists() {
            fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
        }
    }
}
