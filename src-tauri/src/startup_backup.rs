use chrono::NaiveDate;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn backup_root_for_db_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .expect("database path should have a parent")
        .join("backups")
}

pub fn daily_backup_path(backup_root: &Path, date: NaiveDate) -> PathBuf {
    backup_root
        .join(date.format("%Y-%m-%d").to_string())
        .join("backup.json")
}

pub fn write_daily_backup(backup_root: &Path, date: NaiveDate, json: &str) -> io::Result<PathBuf> {
    let backup_path = daily_backup_path(backup_root, date);
    let backup_dir = backup_path
        .parent()
        .expect("backup file path should have a parent directory");

    fs::create_dir_all(backup_dir)?;
    fs::write(&backup_path, json)?;

    Ok(backup_path)
}

pub fn trim_daily_backups(backup_root: &Path, keep_latest: usize) -> io::Result<()> {
    let mut dated_dirs = dated_backup_directories(backup_root)?;
    dated_dirs.sort_by(|(left, _), (right, _)| right.cmp(left));

    for (_, path) in dated_dirs.into_iter().skip(keep_latest) {
        fs::remove_dir_all(path)?;
    }

    Ok(())
}

pub fn list_daily_backups(backup_root: &Path) -> io::Result<Vec<PathBuf>> {
    if !backup_root.exists() {
        return Ok(Vec::new());
    }

    let mut dated_dirs = dated_backup_directories(backup_root)?;
    dated_dirs.sort_by(|(left, _), (right, _)| right.cmp(left));

    Ok(dated_dirs
        .into_iter()
        .map(|(_, path)| path.join("backup.json"))
        .filter(|path| path.is_file())
        .collect())
}

fn dated_backup_directories(backup_root: &Path) -> io::Result<Vec<(NaiveDate, PathBuf)>> {
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

        dated_dirs.push((date, path));
    }

    Ok(dated_dirs)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        backup_root_for_db_path, list_daily_backups, trim_daily_backups, write_daily_backup,
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
            backup_root_for_db_path(db_path),
            Path::new("/workspace/.timetracker-dev/backups")
        );
    }

    #[test]
    fn startup_backup_overwrites_same_day_backup_file() {
        let test_dir = unique_test_dir("startup-backup-overwrite");
        let backup_root = test_dir.join("backups");
        let date = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();

        let first_path = write_daily_backup(&backup_root, date, r#"{"version":1}"#)
            .expect("first backup write should succeed");
        let second_path = write_daily_backup(&backup_root, date, r#"{"version":2}"#)
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
        let backup_root = test_dir.join("backups");

        write_daily_backup(
            &backup_root,
            NaiveDate::from_ymd_opt(2026, 5, 7).unwrap(),
            r#"{"day":"oldest"}"#,
        )
        .expect("oldest backup write should succeed");
        write_daily_backup(
            &backup_root,
            NaiveDate::from_ymd_opt(2026, 5, 8).unwrap(),
            r#"{"day":"middle"}"#,
        )
        .expect("middle backup write should succeed");
        write_daily_backup(
            &backup_root,
            NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
            r#"{"day":"newest"}"#,
        )
        .expect("newest backup write should succeed");

        trim_daily_backups(&backup_root, 2).expect("trim should succeed");

        assert_eq!(
            list_daily_backups(&backup_root).expect("listing backups should succeed"),
            vec![
                backup_root.join("2026-05-09").join("backup.json"),
                backup_root.join("2026-05-08").join("backup.json"),
            ]
        );
        assert!(!backup_root.join("2026-05-07").exists());

        fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
    }

    #[test]
    fn startup_backup_listing_is_empty_when_directory_is_missing() {
        let test_dir = unique_test_dir("startup-backup-empty-list");
        let backup_root = test_dir.join("backups");

        assert!(list_daily_backups(&backup_root)
            .expect("listing missing backups directory should succeed")
            .is_empty());

        if test_dir.exists() {
            fs::remove_dir_all(&test_dir).expect("test directory cleanup should succeed");
        }
    }
}
