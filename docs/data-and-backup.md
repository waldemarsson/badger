# Data and backup

## Data model

TimeTracker stores its working data in a local SQLite database. The schema is intentionally small and centered on the time-entry workflow:

- `customers` — top-level client records, including a single default marker
- `projects` — customer-owned projects, also with a default marker
- `issues` — project-owned work items with an optional external identifier
- `time_entries` — dated hour records linked to issues
- `todos` — standalone kanban todos (`Status`: not started / in progress / done, ordered by `Position`) with an optional link to an issue
- `travel` — standalone business trips: date, from/to place, and optional km and amount (kr), stored as NULL when not entered

Deletes cascade downward from customers to projects, issues, and time entries, so the hierarchy stays consistent without manual cleanup across tables. Todos are never deleted by a cascade: deleting a linked issue only clears the todo's link.

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

TimeTracker automatically creates a backup on each application startup. This behavior is controlled by the startup backup retention setting, which defaults to `3`:

- **Retention > 0**: A backup is written to `backups/YYYY-MM-DD/backup.json` beside the active database, overwriting any existing backup for today. After creation, old backups are automatically trimmed to keep only the most recent `N` backups.
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

Backup export writes a JSON document containing six arrays:

- `customers`
- `projects`
- `issues`
- `time_entries`
- `todos` (optional on import, so backups made before todos existed still restore)
- `travel` (optional on import, so backups made before travel existed still restore)

The backup commands preserve record identifiers so a restore can rebuild the same relationships between records.

## Restore

Restore always overwrites. In one transaction it:

- deletes all customers, which cascades to projects, issues, and time entries
- clears todos and travel explicitly
- re-inserts the full backup

Data not in the backup is lost. If the backup file is invalid, nothing is changed.

There is no merge mode. It was removed because `INSERT OR REPLACE` deletes and re-inserts rows, which cascaded to local time entries and cleared todo links.

## Where backup and restore live in the UI

The active backup and restore workflow is currently exposed in the Settings page, where the frontend:

- opens a save dialog for export
- opens a file picker for import
- previews record counts before restore
- warns that restore replaces all current data

There is also a dedicated `/backup` route in the app, but it is still a placeholder and does not contain the live workflow yet.

## Error handling

Rust command errors are serialized as strings and surfaced back to the frontend. That means backup and restore failures are presented as user-visible text rather than hidden behind silent retries or background recovery.
