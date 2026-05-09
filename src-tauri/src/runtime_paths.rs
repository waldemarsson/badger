use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub db_path: PathBuf,
    pub log_dir: PathBuf,
    pub backup_dir: PathBuf,
}

pub fn repo_root_from_manifest_dir(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .parent()
        .expect("manifest dir should have a parent")
        .to_path_buf()
}

pub fn resolve_runtime_paths(
    home_dir: &Path,
    manifest_dir: &Path,
    debug_build: bool,
) -> RuntimePaths {
    let base_dir = if debug_build {
        repo_root_from_manifest_dir(manifest_dir).join(".timetracker-dev")
    } else {
        home_dir.join(".timetracker")
    };

    RuntimePaths {
        db_path: base_dir.join("timetracker.db"),
        log_dir: base_dir.join("logs"),
        backup_dir: base_dir.join("backups"),
    }
}

pub fn runtime_paths(home_dir: &Path) -> RuntimePaths {
    resolve_runtime_paths(
        home_dir,
        Path::new(env!("CARGO_MANIFEST_DIR")),
        cfg!(debug_assertions),
    )
}

#[cfg(test)]
mod tests {
    use super::{repo_root_from_manifest_dir, resolve_runtime_paths};
    use std::path::Path;

    #[test]
    fn runtime_paths_uses_repo_local_dev_directory_for_debug_builds() {
        let home_dir = Path::new("/home/tester");
        let manifest_dir = Path::new("/workspace/src-tauri");

        let paths = resolve_runtime_paths(home_dir, manifest_dir, true);

        assert_eq!(
            paths.db_path,
            Path::new("/workspace/.timetracker-dev/timetracker.db")
        );
        assert_eq!(paths.log_dir, Path::new("/workspace/.timetracker-dev/logs"));
        assert_eq!(
            paths.backup_dir,
            Path::new("/workspace/.timetracker-dev/backups")
        );
    }

    #[test]
    fn runtime_paths_uses_home_directory_for_non_debug_builds() {
        let home_dir = Path::new("/home/tester");
        let manifest_dir = Path::new("/workspace/src-tauri");

        let paths = resolve_runtime_paths(home_dir, manifest_dir, false);

        assert_eq!(
            paths.db_path,
            Path::new("/home/tester/.timetracker/timetracker.db")
        );
        assert_eq!(paths.log_dir, Path::new("/home/tester/.timetracker/logs"));
        assert_eq!(
            paths.backup_dir,
            Path::new("/home/tester/.timetracker/backups")
        );
        assert_eq!(
            repo_root_from_manifest_dir(manifest_dir),
            Path::new("/workspace")
        );
    }

    #[test]
    fn runtime_paths_startup_backup_dir_lives_beside_database() {
        let home_dir = Path::new("/home/tester");
        let manifest_dir = Path::new("/workspace/src-tauri");

        let paths = resolve_runtime_paths(home_dir, manifest_dir, false);

        assert_eq!(
            paths.backup_dir,
            paths
                .db_path
                .parent()
                .expect("database path should have a parent")
                .join("backups")
        );
    }
}
