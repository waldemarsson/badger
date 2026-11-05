# Architecture

## Overview

TimeTracker is a local-first desktop application built as a Tauri shell around a SvelteKit single-page app. The frontend renders the user interface and calls Rust commands through Tauri's `invoke()` bridge. The Rust backend owns persistence, validation, reporting queries, backup/restore logic, and file-system access.

## Main parts

### Frontend

- Located under `src/`
- Uses SvelteKit in SPA mode with `ssr = false`
- Uses Tauri's WebView runtime instead of a server-rendered deployment model
- Calls backend commands with `@tauri-apps/api/core`

The main user-facing areas are:

- `/` — timesheet entry and editing
- `/todos` — kanban board of standalone todos (drag cards between status columns), optionally linked to an issue
- `/customers` — customer and project management
- `/reports/weekly` — ISO-week report view
- `/reports/monthly` — month report view with expandable grouping
- `/settings` — settings, backup/restore, and dev-only seed tooling

There is also a `/backup` route in the tree, but it is currently still a placeholder while the real backup and restore workflow lives under Settings.

### Backend

- Located under `src-tauri/src/`
- Starts from `main.rs` and delegates to `time_tracker_lib::run()`
- Registers Tauri commands in `lib.rs`
- Stores a shared SQLite pool and logging guard in Tauri managed state

The backend is organized around command modules:

- `customers` and `projects` for reference data
- `issues` and `time_entries` for core timesheet operations
- `todos` for standalone todos and their optional issue link
- `reports` for weekly and monthly rollups
- `backup` for JSON export and import
- `startup_backup` for automatic daily backups on application startup
- `dev_seed` for debug-only sample data

## Data flow

The core request path is:

```text
Svelte component
  -> invoke("command_name", payload)
  -> Tauri command in Rust
  -> sqlx query against SQLite
  -> serialized result back to the frontend
```

The frontend keeps TypeScript interfaces in `src/lib/types.ts` aligned with the Rust response shapes so route components can work with typed command results.

## Storage model

TimeTracker keeps runtime files in different locations by build type:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/timetracker.db
.timetracker-dev/logs/
.timetracker-dev/backups/
```

Non-debug builds:

```text
~/.timetracker/timetracker.db
~/.timetracker/logs/
~/.timetracker/backups/
```

The schema is created on startup if it does not exist. The main tables are:

- `customers`
- `projects`
- `issues`
- `time_entries`
- `todos`

Foreign keys are enabled, and cascading deletes are used to keep related records consistent.

## Runtime behavior with weight

### SPA mode is required

The root layout disables SSR because Tauri APIs only exist inside the WebView at runtime. Frontend code is therefore written as a client-only application.

### Rust owns persistence and file access

The frontend does not talk to SQLite directly and does not read or write backup files on its own. File-system work stays in Rust commands, which keeps native access behind a single boundary.

### Development-only seed data is gated by debug builds

`dev_seed` is compiled only with `debug_assertions`, and the seed command is registered only in debug builds. Production bundles therefore do not ship the seed workflow.

### Logging is file-based

The backend initializes daily-rotating logs under repo-local `.timetracker-dev/logs/` in debug builds and `~/.timetracker/logs/` in non-debug builds. It keeps the logging guard alive in managed Tauri state so writes do not stop early.

### Automatic startup backups

On each application startup, TimeTracker creates a backup if the retention setting is greater than zero. The backup is written to `backups/YYYY-MM-DD/backup.json` beside the active database, overwriting any existing backup for today. After backup creation, old backups are automatically trimmed to keep only the most recent backups based on the retention setting (default: 3). Setting retention to 0 disables automatic startup backups entirely. Backup creation, trimming, and listing logic stays in Rust; the frontend only displays the available backups and retention setting.
