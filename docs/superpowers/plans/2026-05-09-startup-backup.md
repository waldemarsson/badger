# Startup Backup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add automatic daily backups on app startup, keep only the newest configured backup days, and expose backup retention plus backup status in the Settings page.

**Architecture:** Keep backup orchestration on the Rust side. Add a tiny SQLite-backed settings module for the retention count, extend runtime path handling so backup storage lives beside the active database, and introduce a focused startup-backup module that writes `backups/YYYY-MM-DD/backup.json`, overwrites the current day, trims older folders, and lists stored backups for the UI. Wire that flow into Tauri startup with a background task and expose the settings/list operations through backup commands already used by the Settings page.

**Tech Stack:** Rust, Tauri 2, SQLx, SQLite, Chrono, Svelte 5, TypeScript, Markdown docs

---

## File structure

- `src-tauri/src/db.rs` — SQLite schema creation; add the persistent settings table here
- `src-tauri/src/app_settings.rs` — validation and persistence for startup backup retention
- `src-tauri/src/runtime_paths.rs` — resolve the backup root directory beside the active database
- `src-tauri/src/startup_backup.rs` — create, overwrite, trim, and list startup backups on disk
- `src-tauri/src/commands/backup.rs` — keep manual export/restore and add commands for retention + backup listing
- `src-tauri/src/lib.rs` — manage shared runtime paths, register new commands, and spawn the background startup-backup task
- `src/routes/settings/+page.svelte` — load/save retention and render the stored backups list
- `docs/data-and-backup.md` — document startup backups, retention, and on-disk layout
- `docs/architecture.md` — document the Rust-owned startup backup flow
- `docs/configuration.md` — document the new runtime backup location beside the database

### Task 1: Add persisted startup-backup settings

**Files:**
- Create: `src-tauri/src/app_settings.rs`
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/app_settings.rs`
- Test: `src-tauri/src/db.rs`

- [ ] **Step 1: Write the failing backend tests**

Create `src-tauri/src/app_settings.rs` with tests first so the schema and settings API are driven from behavior:

```rust
use crate::AppError;
use sqlx::{Pool, Sqlite};

pub const STARTUP_BACKUP_RETENTION_KEY: &str = "startup_backup_retention";
pub const DEFAULT_STARTUP_BACKUP_RETENTION: i64 = 3;

pub async fn get_startup_backup_retention(_pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    unimplemented!("implement in step 3");
}

pub async fn set_startup_backup_retention(
    _pool: &Pool<Sqlite>,
    _value: i64,
) -> Result<i64, AppError> {
    unimplemented!("implement in step 3");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn startup_backup_retention_defaults_to_three() {
        let pool = crate::test_db().await;

        let value = get_startup_backup_retention(&pool).await.unwrap();

        assert_eq!(value, 3);
    }

    #[tokio::test]
    async fn startup_backup_retention_round_trips_saved_values() {
        let pool = crate::test_db().await;

        let saved = set_startup_backup_retention(&pool, 0).await.unwrap();
        let loaded = get_startup_backup_retention(&pool).await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(loaded, 0);
    }

    #[tokio::test]
    async fn startup_backup_retention_rejects_negative_values() {
        let pool = crate::test_db().await;

        let error = set_startup_backup_retention(&pool, -1).await.unwrap_err();

        assert!(error.to_string().contains("greater than or equal to 0"));
    }
}
```

Extend the schema assertion in `src-tauri/src/db.rs` so the table must exist:

```rust
assert!(names.contains(&"app_settings"), "app_settings table missing");
```

Expose the new module from `src-tauri/src/lib.rs`:

```rust
pub mod app_settings;
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml app_settings`

Expected: FAIL because the `app_settings` table does not exist and both settings functions are still `unimplemented!()`.

- [ ] **Step 3: Implement the schema and settings module**

Add the persistent settings table in `src-tauri/src/db.rs`:

```rust
CREATE TABLE IF NOT EXISTS app_settings (
    Key    TEXT PRIMARY KEY,
    Value  TEXT NOT NULL
);
```

Replace the stubs in `src-tauri/src/app_settings.rs` with the real implementation:

```rust
use crate::AppError;
use sqlx::{Pool, Sqlite};

pub const STARTUP_BACKUP_RETENTION_KEY: &str = "startup_backup_retention";
pub const DEFAULT_STARTUP_BACKUP_RETENTION: i64 = 3;

fn validate_startup_backup_retention(value: i64) -> Result<(), AppError> {
    if value < 0 {
        return Err(AppError::Custom(
            "Startup backup retention must be greater than or equal to 0.".into(),
        ));
    }

    Ok(())
}

pub async fn get_startup_backup_retention(pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT Value FROM app_settings WHERE Key = ?")
            .bind(STARTUP_BACKUP_RETENTION_KEY)
            .fetch_optional(pool)
            .await?;

    match stored {
        Some(value) => value
            .parse::<i64>()
            .map_err(|_| AppError::Custom("Stored startup backup retention is invalid.".into())),
        None => Ok(DEFAULT_STARTUP_BACKUP_RETENTION),
    }
}

pub async fn set_startup_backup_retention(
    pool: &Pool<Sqlite>,
    value: i64,
) -> Result<i64, AppError> {
    validate_startup_backup_retention(value)?;

    sqlx::query(
        "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
         ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
    )
    .bind(STARTUP_BACKUP_RETENTION_KEY)
    .bind(value.to_string())
    .execute(pool)
    .await?;

    Ok(value)
}
```

- [ ] **Step 4: Run the targeted tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml app_settings`

Expected: PASS with the default, round-trip, and validation tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/app_settings.rs src-tauri/src/db.rs src-tauri/src/lib.rs
git commit -m "Add persisted startup backup retention setting"
```

### Task 2: Add backup path helpers and filesystem backup behavior

**Files:**
- Create: `src-tauri/src/startup_backup.rs`
- Modify: `src-tauri/src/runtime_paths.rs`
- Test: `src-tauri/src/startup_backup.rs`
- Test: `src-tauri/src/runtime_paths.rs`

- [ ] **Step 1: Write the failing filesystem tests**

Create `src-tauri/src/startup_backup.rs` with behavior-driven tests for backup layout, overwrite, trimming, and listing:

```rust
use crate::AppError;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StoredBackupSummary {
    pub date: String,
    pub size_bytes: u64,
}

pub fn backup_root_dir(_db_path: &Path) -> PathBuf {
    unimplemented!("implement in step 3");
}

pub fn write_daily_backup(
    _db_path: &Path,
    _date: &str,
    _json: &str,
) -> Result<(), AppError> {
    unimplemented!("implement in step 3");
}

pub fn trim_old_backups(_db_path: &Path, _retention: usize) -> Result<(), AppError> {
    unimplemented!("implement in step 3");
}

pub fn list_startup_backups(_db_path: &Path) -> Result<Vec<StoredBackupSummary>, AppError> {
    unimplemented!("implement in step 3");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    fn temp_db_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("timetracker-startup-backup-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root.join("timetracker.db")
    }

    #[test]
    fn backup_root_dir_is_created_beside_the_database() {
        let db_path = Path::new("/tmp/timetracker/timetracker.db");

        assert_eq!(backup_root_dir(db_path), Path::new("/tmp/timetracker/backups"));
    }

    #[test]
    fn write_daily_backup_overwrites_the_same_day_file() {
        let db_path = temp_db_path();

        write_daily_backup(&db_path, "2026-01-01", "{\"version\":1}").unwrap();
        write_daily_backup(&db_path, "2026-01-01", "{\"version\":2}").unwrap();

        let saved = fs::read_to_string(
            backup_root_dir(&db_path).join("2026-01-01").join("backup.json"),
        )
        .unwrap();

        assert_eq!(saved, "{\"version\":2}");
    }

    #[test]
    fn trim_old_backups_keeps_only_the_newest_days() {
        let db_path = temp_db_path();

        write_daily_backup(&db_path, "2026-01-01", "{}").unwrap();
        write_daily_backup(&db_path, "2026-01-02", "{}").unwrap();
        write_daily_backup(&db_path, "2026-01-03", "{}").unwrap();
        write_daily_backup(&db_path, "2026-01-04", "{}").unwrap();

        trim_old_backups(&db_path, 3).unwrap();

        let backups = list_startup_backups(&db_path).unwrap();
        let dates: Vec<&str> = backups.iter().map(|entry| entry.date.as_str()).collect();

        assert_eq!(dates, vec!["2026-01-04", "2026-01-03", "2026-01-02"]);
    }

    #[test]
    fn list_startup_backups_returns_empty_when_no_backup_directory_exists() {
        let db_path = temp_db_path();

        let backups = list_startup_backups(&db_path).unwrap();

        assert!(backups.is_empty());
    }
}
```

Add the backup root assertion in `src-tauri/src/runtime_paths.rs`:

```rust
#[test]
fn backup_root_is_a_sibling_of_the_database() {
    let db_path = Path::new("/workspace/.timetracker-dev/timetracker.db");
    assert_eq!(
        crate::startup_backup::backup_root_dir(db_path),
        Path::new("/workspace/.timetracker-dev/backups")
    );
}
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml startup_backup`

Expected: FAIL because the new helpers are still `unimplemented!()` and the backup layout behavior does not exist yet.

- [ ] **Step 3: Implement the backup filesystem module**

Expose the module from `src-tauri/src/lib.rs`:

```rust
pub mod startup_backup;
```

Replace the stubs in `src-tauri/src/startup_backup.rs` with concrete filesystem behavior:

```rust
use crate::AppError;
use std::{fs, path::{Path, PathBuf}};

pub const BACKUP_FILE_NAME: &str = "backup.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StoredBackupSummary {
    pub date: String,
    pub size_bytes: u64,
}

pub fn backup_root_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .expect("database path should have a parent directory")
        .join("backups")
}

pub fn write_daily_backup(db_path: &Path, date: &str, json: &str) -> Result<(), AppError> {
    let day_dir = backup_root_dir(db_path).join(date);
    fs::create_dir_all(&day_dir)?;
    fs::write(day_dir.join(BACKUP_FILE_NAME), json)?;
    Ok(())
}

pub fn trim_old_backups(db_path: &Path, retention: usize) -> Result<(), AppError> {
    let root = backup_root_dir(db_path);
    if !root.exists() {
        return Ok(());
    }

    let mut dated_dirs: Vec<_> = fs::read_dir(&root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();

    dated_dirs.sort_by_key(|entry| entry.file_name());
    dated_dirs.reverse();

    for entry in dated_dirs.into_iter().skip(retention) {
        fs::remove_dir_all(entry.path())?;
    }

    Ok(())
}

pub fn list_startup_backups(db_path: &Path) -> Result<Vec<StoredBackupSummary>, AppError> {
    let root = backup_root_dir(db_path);
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut backups = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let date = entry.file_name().to_string_lossy().to_string();
        let file_path = entry.path().join(BACKUP_FILE_NAME);
        let size_bytes = fs::metadata(file_path)?.len();

        backups.push(StoredBackupSummary { date, size_bytes });
    }

    backups.sort_by(|left, right| right.date.cmp(&left.date));
    Ok(backups)
}
```

- [ ] **Step 4: Run the targeted tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml startup_backup`

Expected: PASS with the sibling-path, overwrite, trim, and empty-list tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/startup_backup.rs src-tauri/src/runtime_paths.rs src-tauri/src/lib.rs
git commit -m "Add startup backup filesystem helpers"
```

### Task 3: Wire startup backup execution and backup commands

**Files:**
- Modify: `src-tauri/src/commands/backup.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/commands/backup.rs`

- [ ] **Step 1: Write the failing command-level tests**

Add inner helper tests to `src-tauri/src/commands/backup.rs` before adding the Tauri wrappers:

```rust
#[tokio::test]
async fn startup_backup_retention_commands_round_trip() {
    let pool = crate::test_db().await;

    let saved = inner_set_startup_backup_retention(5, &pool).await.unwrap();
    let loaded = inner_get_startup_backup_retention(&pool).await.unwrap();

    assert_eq!(saved, 5);
    assert_eq!(loaded, 5);
}

#[tokio::test]
async fn startup_backup_listing_reads_existing_backup_directories() {
    let pool = crate::test_db().await;
    let db_path = std::env::temp_dir().join("timetracker-command-test").join("timetracker.db");

    crate::startup_backup::write_daily_backup(&db_path, "2026-01-01", "{\"ok\":true}").unwrap();

    let backups = inner_list_startup_backups(&db_path, &pool).await.unwrap();

    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].date, "2026-01-01");
    assert!(backups[0].size_bytes > 0);
}
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup`

Expected: FAIL because `inner_get_startup_backup_retention`, `inner_set_startup_backup_retention`, and `inner_list_startup_backups` do not exist yet.

- [ ] **Step 3: Implement the inner helpers, commands, and startup task**

Add the shared inner helpers and Tauri commands in `src-tauri/src/commands/backup.rs`:

```rust
pub async fn inner_get_startup_backup_retention(pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    crate::app_settings::get_startup_backup_retention(pool).await
}

pub async fn inner_set_startup_backup_retention(
    value: i64,
    pool: &Pool<Sqlite>,
) -> Result<i64, AppError> {
    crate::app_settings::set_startup_backup_retention(pool, value).await
}

pub async fn inner_list_startup_backups(
    db_path: &std::path::Path,
    _pool: &Pool<Sqlite>,
) -> Result<Vec<crate::startup_backup::StoredBackupSummary>, AppError> {
    crate::startup_backup::list_startup_backups(db_path)
}

#[tauri::command]
pub async fn get_startup_backup_retention(
    pool: State<'_, Pool<Sqlite>>,
) -> Result<i64, AppError> {
    inner_get_startup_backup_retention(&pool).await
}

#[tauri::command]
pub async fn set_startup_backup_retention(
    value: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<i64, AppError> {
    inner_set_startup_backup_retention(value, &pool).await
}

#[tauri::command]
pub async fn list_startup_backups(
    pool: State<'_, Pool<Sqlite>>,
    runtime_paths: State<'_, crate::runtime_paths::RuntimePaths>,
) -> Result<Vec<crate::startup_backup::StoredBackupSummary>, AppError> {
    inner_list_startup_backups(&runtime_paths.db_path, &pool).await
}
```

Update `src-tauri/src/lib.rs` so startup backup runs in the background and the new commands are registered:

```rust
let home = app.path().home_dir()?;
let runtime_paths = runtime_paths::runtime_paths(&home);
let guard = logger::init_logger(&runtime_paths.log_dir);
app.manage(guard);

let pool = tauri::async_runtime::block_on(db::init_db(&runtime_paths.db_path))?;
app.manage(pool.clone());
app.manage(runtime_paths.clone());

let startup_pool = pool.clone();
let startup_paths = runtime_paths.clone();
tauri::async_runtime::spawn(async move {
    let retention = match crate::app_settings::get_startup_backup_retention(&startup_pool).await {
        Ok(value) => value,
        Err(error) => {
            tracing::error!("Failed to load startup backup retention: {error}");
            return;
        }
    };

    if retention == 0 {
        return;
    }

    match crate::commands::backup::inner_backup_data(&startup_pool).await {
        Ok(json) => {
            let today = chrono::Local::now().date_naive().format("%F").to_string();
            if let Err(error) = crate::startup_backup::write_daily_backup(&startup_paths.db_path, &today, &json)
            {
                tracing::error!("Failed to write startup backup: {error}");
                return;
            }

            if let Err(error) = crate::startup_backup::trim_old_backups(
                &startup_paths.db_path,
                retention as usize,
            ) {
                tracing::error!("Failed to trim startup backups: {error}");
            }
        }
        Err(error) => tracing::error!("Failed to export startup backup data: {error}"),
    }
});
```

Register the commands in both handler branches:

```rust
commands::backup::get_startup_backup_retention,
commands::backup::set_startup_backup_retention,
commands::backup::list_startup_backups,
```

- [ ] **Step 4: Run the backend tests to verify the wiring passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: PASS with the new backup command tests and the existing backup/restore tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/backup.rs src-tauri/src/lib.rs
git commit -m "Wire startup backup commands and background task"
```

### Task 4: Extend the Settings page for retention and backup status

**Files:**
- Modify: `src/routes/settings/+page.svelte`

- [ ] **Step 1: Add the new Settings state and loaders**

Add the types, state, and loader/save helpers near the existing backup state in `src/routes/settings/+page.svelte`:

```ts
type StoredBackup = { date: string; size_bytes: number };

let startupBackupRetention = $state(3);
let startupBackupRetentionDraft = $state('3');
let startupBackupRetentionState = $state<'idle' | 'saving' | 'success' | 'error'>('idle');
let startupBackupRetentionError = $state('');
let storedBackups = $state<StoredBackup[]>([]);
let storedBackupsError = $state('');

function formatBackupSize(sizeBytes: number): string {
  if (sizeBytes < 1024) return `${sizeBytes} B`;
  if (sizeBytes < 1024 * 1024) return `${(sizeBytes / 1024).toFixed(1)} KB`;
  return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`;
}

async function loadStartupBackupSettings() {
  startupBackupRetention = await invoke<number>('get_startup_backup_retention');
  startupBackupRetentionDraft = String(startupBackupRetention);
  storedBackupsError = '';

  try {
    storedBackups = await invoke<StoredBackup[]>('list_startup_backups');
  } catch (e) {
    storedBackups = [];
    storedBackupsError = String(e);
  }
}

async function saveStartupBackupRetention() {
  startupBackupRetentionState = 'saving';
  startupBackupRetentionError = '';

  try {
    const parsed = Number.parseInt(startupBackupRetentionDraft, 10);
    if (!Number.isInteger(parsed) || parsed < 0) {
      throw new Error('Retention must be 0 or greater.');
    }
    const saved = await invoke<number>('set_startup_backup_retention', { value: parsed });
    startupBackupRetention = saved;
    startupBackupRetentionDraft = String(saved);
    storedBackupsError = '';
    storedBackups = await invoke<StoredBackup[]>('list_startup_backups');
    startupBackupRetentionState = 'success';
  } catch (e) {
    startupBackupRetentionError = String(e);
    startupBackupRetentionState = 'error';
  }
}
```

Call the loader from the existing `onMount` block:

```ts
onMount(async () => {
  customers = await invoke<Customer[]>('list_customers');
  await loadStartupBackupSettings();
});
```

- [ ] **Step 2: Run the frontend checker after wiring the state**

Run: `npm run check`

Expected: PASS, confirming the new state and invoke calls type-check before the markup changes land.

- [ ] **Step 3: Add the retention control and backup list markup**

Extend the backup tab section in `src/routes/settings/+page.svelte` with the new controls:

```svelte
<div class="panel-muted space-y-3">
  <div>
    <p class="form-label">Automatic startup backups</p>
    <p class="text-sm text-slate-500">
      Keep the latest startup backup days beside the active database. Set to 0 to disable.
    </p>
  </div>

  <div class="flex items-end gap-3">
    <label class="flex flex-col gap-1">
      <span class="text-sm font-medium text-slate-700">Retention days</span>
      <input
        class="form-input w-28"
        type="number"
        min="0"
        bind:value={startupBackupRetentionDraft}
      />
    </label>

    <button
      class="btn-secondary"
      onclick={saveStartupBackupRetention}
      disabled={startupBackupRetentionState === 'saving'}
    >
      {startupBackupRetentionState === 'saving' ? 'Saving…' : 'Save Retention'}
    </button>
  </div>

  {#if startupBackupRetentionState === 'success'}
    <div class="alert-panel alert-success">
      <p class="alert-success-title">Backup setting saved</p>
      <p class="alert-success-copy">Automatic backup retention is now {startupBackupRetention}.</p>
    </div>
  {/if}

  {#if startupBackupRetentionState === 'error'}
    <div class="alert-panel alert-error">
      <p class="alert-error-title">Could not save backup setting</p>
      <p class="alert-error-copy">{startupBackupRetentionError}</p>
    </div>
  {/if}
</div>

<div class="panel-muted space-y-3">
  <div>
    <p class="form-label">Created backups</p>
    <p class="text-sm text-slate-500">Newest first. Sizes reflect the stored backup folder.</p>
  </div>

  {#if storedBackups.length === 0}
    <p class="text-sm text-slate-500">No automatic backups have been created yet.</p>
  {:else}
    <div class="space-y-2">
      {#each storedBackups as backup (backup.date)}
        <div class="list-card flex items-center justify-between">
          <span class="font-medium text-slate-900">{backup.date}</span>
          <span class="text-sm text-slate-500">{formatBackupSize(backup.size_bytes)}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if storedBackupsError}
    <div class="alert-panel alert-error">
      <p class="alert-error-title">Could not load backups</p>
      <p class="alert-error-copy">{storedBackupsError}</p>
    </div>
  {/if}
</div>
```

- [ ] **Step 4: Run the frontend checks to verify the page passes**

Run: `npm run check && npm run lint`

Expected: PASS with no Svelte or ESLint errors from the new backup settings UI.

- [ ] **Step 5: Commit**

```bash
git add src/routes/settings/+page.svelte
git commit -m "Add startup backup settings to the settings page"
```

### Task 5: Update documentation and run full verification

**Files:**
- Modify: `docs/data-and-backup.md`
- Modify: `docs/architecture.md`
- Modify: `docs/configuration.md`

- [ ] **Step 1: Update the backup and architecture docs**

Add the startup-backup behavior to `docs/data-and-backup.md`:

```md
## Automatic startup backups

On startup, TimeTracker can create a background backup beside the active database:

- backup root: `backups/`
- daily folder format: `YYYY-MM-DD`
- file name: `backup.json`

The retention count is configured in Settings. The default is `3`, and `0` disables automatic startup backups.
```

Add the backend ownership note to `docs/architecture.md`:

```md
Backup export, restore, startup-backup creation, retention trimming, and backup listing all stay in Rust so filesystem access and SQLite-backed settings remain behind the native command boundary.
```

Add the runtime-location note to `docs/configuration.md`:

```md
The active database directory also contains `backups/`, so development runs store automatic backups under `.timetracker-dev/backups/` while non-debug builds store them under `~/.timetracker/backups/`.
```

- [ ] **Step 2: Run the full automated verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && npm run check && npm run lint`

Expected: PASS with backend tests, Svelte checks, and lint all green.

- [ ] **Step 3: Run a manual startup smoke test**

Run: `npm run dev`

Expected: the app opens normally, startup is not visibly blocked, `.timetracker-dev/backups/<today>/backup.json` exists after launch, and the Settings > Backup & Restore tab shows retention `3` plus a backup row for today.

- [ ] **Step 4: Commit**

```bash
git add docs/data-and-backup.md docs/architecture.md docs/configuration.md
git commit -m "Document automatic startup backups"
```
