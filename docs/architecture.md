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

- `/` — Today: agenda of todos to act on (due today or overdue, in progress), today's logged time, and a Mon–Sun week strip with progress against the daily hours target
- `/timesheet` — timesheet entry and editing; date filter includes Single Day, and `?day=YYYY-MM-DD` opens it on that day (an invalid `day` is ignored)
- `/todos` — kanban board of standalone todos (drag cards between status columns), optionally linked to an issue
- `/reports/weekly` — ISO-week report view
- `/reports/monthly` — month report view with expandable grouping, the month's travel, and a copy-month text export
- `/travel` — month-scoped list of business trips (date, from/to, optional km and kr) with inline add, edit, repeat, and delete
- `/settings` — customer and project management (cards with inline add, rename, default, and delete), preferences (System/Light/Dark appearance), backup/restore, and dev-only seed tooling

There is also a `/backup` route in the tree, but it is currently still a placeholder while the real backup and restore workflow lives under Settings.

### Backend

- Located under `src-tauri/src/`
- Starts from `main.rs` and delegates to `time_tracker_lib::run()`
- Registers Tauri commands in `lib.rs`
- Stores a shared SQLite pool and logging guard in Tauri managed state

The backend is organized around command modules:

- `customers` and `projects` for reference data (names are trimmed and unique case-insensitively: customers globally, projects per customer)
- `issues` and `time_entries` for core timesheet operations
- `todos` for standalone todos and their optional issue link
- `travel` for business trips, listed per month
- `reports` for weekly and monthly rollups
- `backup` for JSON export and import
- `settings` for app preferences (theme, daily hours target, command palette shortcut and last-used project/issue) stored in `app_settings`
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

### Theme

Dark mode uses Tailwind's `darkMode: "class"`: a `dark` class on `<html>` activates the `dark:` variants in `app.css` and markup. Dark surfaces use Tailwind's `neutral` grays; light mode uses `slate`. The theme mode (`system`, `light`, `dark`) is stored in `app_settings` via `get_theme`/`set_theme`; `src/lib/theme-state.svelte.ts` applies it (class, CSS `color-scheme`, native window theme) and mirrors it to `localStorage` so the inline script in `src/app.html` paints the right theme before the app loads. The setting is not part of backups.

### Command palette

`src/lib/components/CommandPalette.svelte` is mounted in the root layout and listens for its shortcut on `window` in the capture phase, so it opens over text fields and modals and Esc closes only the palette. The shortcut (default `Meta+K`, configurable in Settings › Preferences) and the last project/issue used for Log time live in `app_settings` (`get_palette_settings`, `set_palette_shortcut`, `set_palette_last_used`) and are not part of backups. Shortcut rules (needs ⌘/⌃/⌥, macOS-reserved combos refused), filtering and hours/date parsing are pure helpers in `src/lib/palette.ts`. After creating data the palette bumps a counter in `src/lib/palette-state.svelte.ts` (`changes.todos` / `changes.timeEntries`); the Todos and Timesheet pages reload when it changes. The same module drives the layout's `Toast`. Pages can open the Log time flow with `requestLogTime(prefill?)` in the same module: the palette consumes the request, and a prefill with a project and issue (Today's todo [Log] button) jumps straight to the hours step. If the prefill's project no longer exists, the normal project step is shown.

### Today

Today (`src/routes/+page.svelte`) aggregates in the frontend: it loads the current week's `list_time_entries`, `list_todos` and the daily target, and the pure selection, ordering and totals helpers live in `src/lib/today.ts` (tested in `tests/today.test.mjs`). There are no dedicated list/report commands. Needs attention is non-done todos due today or earlier; In progress is the remaining `in_progress` todos, so no todo appears twice. The week runs Monday–Sunday and the weekly target is 5 × the daily target. The daily target is stored in `app_settings` via `get_daily_target`/`set_daily_target` (default 8, range 0–24, 0 turns the progress UI off, an invalid stored value falls back to 8), edited in Settings › Preferences, and is not part of backups. The page reloads when the palette bumps `changes`.

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
- `travel`

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
