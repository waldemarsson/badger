# Data and backup

## Data model

TimeTracker stores its working data in a local SQLite database. The schema is intentionally small and centered on the time-entry workflow:

- `customers` — top-level client records, including a single default marker
- `projects` — customer-owned projects, also with a default marker
- `issues` — project-owned work items with an optional external identifier
- `time_entries` — dated hour records linked to issues

Deletes cascade downward from customers to projects, issues, and time entries, so the hierarchy stays consistent without manual cleanup across tables.

## Database behavior

The database is initialized on startup with:

- WAL journal mode
- foreign keys enabled
- idempotent schema creation

The database location depends on the build type:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/timetracker.db
```

Installed and other non-debug builds:

```text
~/.timetracker/timetracker.db
```

## Automatic startup backups

TimeTracker automatically creates a daily backup on application startup. This behavior is controlled by the startup backup retention setting, which defaults to `3`:

- **Retention > 0**: A backup is created at `backups/YYYY-MM-DD/backup.json` beside the active database if one does not already exist for today. After creation, old backups are automatically trimmed to keep only the most recent `N` backups.
- **Retention = 0**: Automatic startup backups are disabled entirely.

The retention setting can be adjusted in the Settings > Backup & Restore tab.

### Startup backup locations

The backup root directory is determined by the database location:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/backups/YYYY-MM-DD/backup.json
```

Installed and other non-debug builds:

```text
~/.timetracker/backups/YYYY-MM-DD/backup.json
```

Startup backup creation, trimming, and listing are handled entirely in Rust and do not involve the frontend beyond displaying the available backups and retention setting in the UI.

## Backup format

Backup export writes a JSON document containing four arrays:

- `customers`
- `projects`
- `issues`
- `time_entries`

The backup commands preserve record identifiers so a restore can rebuild the same relationships between records.

## Restore modes

The restore workflow supports two modes:

### Merge

- Upserts records from the backup
- Keeps unrelated existing records that are not present in the import
- Useful when you want to bring backup data into a database that already contains other records

### Replace

- Deletes all current customer data first
- Relies on cascading foreign keys to clear projects, issues, and time entries
- Re-inserts the full backup afterward

Use replace carefully: it wipes the current dataset before restoring.

## Where backup and restore live in the UI

The active backup and restore workflow is currently exposed in the Settings page, where the frontend:

- opens a save dialog for export
- opens a file picker for import
- previews record counts before restore
- lets the user choose merge or replace mode

There is also a dedicated `/backup` route in the app, but it is still a placeholder and does not contain the live workflow yet.

## Error handling

Rust command errors are serialized as strings and surfaced back to the frontend. That means backup and restore failures are presented as user-visible text rather than hidden behind silent retries or background recovery.
