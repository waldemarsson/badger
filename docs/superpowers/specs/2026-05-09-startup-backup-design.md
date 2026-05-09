# Startup backup design

## Problem

TimeTracker currently supports manual backup export and restore from the Settings page, but it does not create automatic backups when the app starts.

That leaves two gaps:

- users do not get routine backups unless they remember to export them manually
- the app cannot show whether recent backups exist or how current they are

## Goal

Create automatic startup backups that are stored beside the active database, retained by day, and manageable from the existing Backup section in Settings.

## Non-goals

- Replacing the existing manual export and restore workflow
- Changing the JSON backup format
- Adding cloud sync or off-device backup storage
- Blocking app startup on backup completion

## Recommended approach

Add a small Rust-owned startup backup service that runs in the background after the app has initialized its database connection.

- Store a persisted retention setting in SQLite with a default value of `3`
- Resolve the backup root as a sibling to the active database file:
  - development example: `.timetracker-dev/backups/`
  - non-debug example: `~/.timetracker/backups/`
- Create one dated folder per day using ISO date format, for example `backups/2026-01-01/`
- Write a single stable backup file inside that folder, such as `backup.json`
- If the folder for the current day already exists, overwrite the existing backup file
- After writing the current backup, delete older dated folders so only the newest configured number remain
- Expose backend commands for the Settings page to read and update retention and to list existing backups with date and size

## Why this approach

1. Automatic backups remain available even before the frontend is fully loaded.
2. Settings and filesystem access stay in Rust, which matches the current app boundary for backup logic.
3. The startup path can stay non-blocking by spawning the work in the background.
4. The Settings page can display authoritative backup metadata without directly touching the filesystem.

## Design details

### Persisted backup setting

Add a small app-settings persistence mechanism in SQLite for startup backup retention.

- Key: startup backup retention count
- Default: `3`
- Valid values: integers greater than or equal to `0`
- `0` means automatic startup backups are disabled

The backend should own validation so invalid values are rejected consistently regardless of caller.

### Backup storage layout

Backups should live beside the active database rather than in a fixed global location.

- Database path example: `.../timetracker.db`
- Backup root: sibling directory `.../backups/`
- Daily folder: `.../backups/YYYY-MM-DD/`
- Backup file: `.../backups/YYYY-MM-DD/backup.json`

This keeps development and non-debug backups aligned with the existing runtime path selection logic.

### Startup backup flow

During app startup, after the database pool has been created and managed, spawn a background task that:

1. reads the retention setting from SQLite
2. exits immediately if the value is `0`
3. exports the current database contents using the existing backup serialization logic
4. ensures the dated backup directory exists
5. overwrites that day’s backup file with the latest exported JSON
6. scans the `backups/` directory for dated folders
7. keeps the newest `N` folders and removes older ones

The startup path should not wait for that background task to finish before allowing the app window to continue opening.

### Backup listing for Settings

Expose a backend command that returns backup metadata for the Settings page.

Each list item should include:

- date, derived from the folder name
- total size for that backup folder

The command should sort newest first so the most recent backup status is visible at a glance.

If the `backups/` directory does not exist yet, return an empty list rather than an error.

### Settings UI changes

Extend the existing Backup tab in `src/routes/settings/+page.svelte` with:

- a numeric input for startup backup retention
- save feedback for successful or failed updates
- a read-only list of created backups showing date and size

Manual export and restore stay in the same tab and should continue to work unchanged.

### Error handling

Automatic startup backup is best-effort.

- Failures in the background startup backup flow should be logged in Rust
- Backup failures should not block app startup or prevent the UI from opening
- Manual export and restore should keep their current user-visible error behavior
- Invalid retention updates from the UI should return explicit command errors

### Testing

Add focused backend coverage for:

- backup root resolution beside the database path
- default settings value and settings persistence
- overwrite behavior when a backup already exists for the current day
- retention trimming that keeps only the newest configured dated folders
- backup listing metadata and empty-directory behavior

Frontend coverage should stay limited to the Settings page behavior:

- loading the retention value
- saving valid updates
- rendering returned backup metadata

## Affected files

- `src-tauri/src/lib.rs`
- `src-tauri/src/runtime_paths.rs`
- `src-tauri/src/commands/backup.rs`
- likely a new backend module for persisted settings or startup backup orchestration
- `src/routes/settings/+page.svelte`
- `docs/data-and-backup.md`
- `docs/architecture.md`
- `docs/configuration.md`

