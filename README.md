# Badger

**Your time, your todos, your notes — on your Mac, and nowhere else.**

Badger is a fast, keyboard-friendly desktop app for consultants and developers who bill by the hour. Log time against customers, projects, and issues; run your week from a kanban board; keep markdown notes next to the work; and get a month-end report you can paste straight into an invoice.

No account, no cloud, no subscription, no telemetry. Everything lives in one SQLite file in your home directory.

![Today view](docs/screenshots/today.png)

## Why Badger

- **Local-first, private by default.** All data is stored in `~/.badger/badger.db` on your machine. The app makes no network calls for your data and works fully offline.
- **Yours to keep.** Plain SQLite plus one-click JSON export. Automatic daily backups on startup, with configurable retention.
- **Built for the workflow, not the database.** A Today agenda, a drag-and-drop board, and a ⌘K command palette replace form-filling.
- **Native and lightweight.** A Tauri 2 app with a Rust backend: small binary, quick start, low memory.
- **Light and dark.** Follows the system appearance, or pick one.

## A tour

### Today — start the day here

Overdue and due-today todos, today's reminders, what's in progress, hours logged today, and a Mon–Sun strip showing progress against your daily target. Complete a todo or log time on it in one click.

| Light | Dark |
| --- | --- |
| ![Today](docs/screenshots/today.png) | ![Today, dark](docs/screenshots/today-dark.png) |

### Command palette — do anything with ⌘K

Log time, create a todo or note, set a reminder, jump to any page, or search notes without touching the mouse. It remembers your last project and issue, so a time entry is a few keystrokes. The shortcut is configurable.

![Command palette](docs/screenshots/palette.png)

### Timesheet — every entry, editable inline

Add, edit, and delete entries in place, with pickers for customer → project → issue. Filter by day, week, month, or a custom range; the toolbar shows the period total.

![Timesheet](docs/screenshots/timesheet.png)

### Todos — a kanban board, not a table

Drag cards between *Not started*, *In progress*, and *Done*. Due dates turn amber for today and red when overdue. Link a todo to an issue (or create the issue from the card) so it's one click from logging time. A bell on the card shows its next reminder; set one with *Remind me…* in the card's editor.

| Light | Dark |
| --- | --- |
| ![Todos](docs/screenshots/todos.png) | ![Todos, dark](docs/screenshots/todos-dark.png) |

### Reminders — Slack-style, from anywhere

One-off or recurring reminders: daily, weekdays, weekly, or monthly. Type `remind call Anna in 20 min` (or Swedish `påminn…`) in ⌘K and a live preview shows exactly when it will fire, or set one from a todo or a note. When it's due you get a native macOS notification, or an in-app toast with *Done* and *Snooze* while the app is in front. Overdue reminders are counted on the nav tab and the dock badge, and ones that came due while the app was closed show up on the next launch.

| Light | Dark |
| --- | --- |
| ![Reminders](docs/screenshots/reminders.png) | ![Reminders, dark](docs/screenshots/reminders-dark.png) |

### Notes — markdown with live preview

Notes with a live-preview editor: headings, task lists, quotes, and syntax-highlighted code render as you type. Organise them in nested folders with drag and drop or a right-click menu, link notes with `[[wikilinks]]` and see what links back, and tag them with `#tags` — including your customers, projects, issues, and todos, which show up as live chips. Pin the important ones, search (or filter by `#tag`) everything with ranked full-text search and highlighted matches, optionally adding meaning-based results from a downloaded on-device model (Settings › Preferences), switch to a clean read view, and copy a note as HTML for email or chat. Edits save automatically, including on quit.

| Light | Dark |
| --- | --- |
| ![Notes](docs/screenshots/notes.png) | ![Notes, dark](docs/screenshots/notes-dark.png) |

### Weekly report

An ISO-week grid grouped by customer and project, with per-day and per-issue totals.

![Weekly report](docs/screenshots/weekly.png)

### Monthly report — invoice-ready

Hours per customer, project, and issue, with expandable groups and the month's travel. Copy a project or the whole month as text for your invoice or client portal.

![Monthly report](docs/screenshots/monthly.png)

### Travel

Log business trips (date, from/to, km, amount) per month. Places autocomplete, and a trip can be repeated with one click. Totals feed into the monthly report.

![Travel](docs/screenshots/travel.png)

### Settings

Manage customers and projects as cards, with defaults that pre-fill new entries. Preferences cover appearance, daily hours target, the palette shortcut, and the search index. Backup & Restore exports to JSON, restores from a file, and lists automatic startup backups.

![Settings](docs/screenshots/settings.png)

> Screenshots show sample data, rendered from the built frontend with a mocked backend.

## Your data

| What | Where |
| --- | --- |
| Database (installed app) | `~/.badger/badger.db` |
| Database (dev builds) | `.badger-dev/` in the repo |
| Manual backups | JSON file, wherever you save it |
| Automatic backups | Daily on startup, kept for a configurable number of days |

Restore replaces the current data with the backup's contents. See [Data and backup](docs/data-and-backup.md) for the schema and backup format.

## Install (macOS)

Pre-built `.dmg` releases are published when a `v*` tag is pushed (see [Releasing](#releasing)).

1. Go to the [Releases](../../releases) page and download the latest `.dmg`.
2. Open the `.dmg` and drag **Badger** into your `Applications` folder.
3. On first launch macOS blocks the app because it is unsigned. Remove the
   Gatekeeper quarantine flag from a terminal:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Badger.app
   ```

   Then open the app normally. If macOS still reports the app as "damaged",
   re-sign it ad-hoc and retry:

   ```bash
   codesign --force --deep --sign - /Applications/Badger.app
   xattr -dr com.apple.quarantine /Applications/Badger.app
   ```

The build is arm64-only (Apple Silicon) and unsigned/un-notarized, which is why
the quarantine step above is required.

## Development

```bash
npm install
npm run dev
```

This starts the Vite dev server and launches the Tauri shell against it. Development runs store their SQLite database and logs under repo-local `.badger-dev/`, isolated from the live `~/.badger/` data used by release builds.

**Stack:** SvelteKit SPA (Svelte 5, Tailwind) in a Tauri 2 WebView, with a Rust backend that owns a SQLite database via sqlx.

| Command | Purpose |
| --- | --- |
| `npm run dev` | Run the desktop app in development mode |
| `npm run build` | Build the production app bundle |
| `npm run check` | Run Svelte and TypeScript checks |
| `npm run lint` | Run frontend linting |
| `npm run format` | Format the repo with Prettier |
| `node --test tests/*.mjs` | Run frontend unit tests |
| `cargo test` | Run Rust unit tests |
| `cargo clippy --all -- -D warnings` | Run Rust linting |
| `cargo fmt --check` | Check Rust formatting |
| `make build` | Build the macOS app bundle |
| `make install` | Copy the built `.app` into `/Applications` |

```text
src/              SvelteKit frontend
src-tauri/        Rust Tauri backend
assets/           App assets used by the project
docs/             Documentation and screenshots
```

### Releasing

Merging to `main` does not build anything. To publish a release, tag a commit on `main` and push the tag:

```bash
git tag v1.0.0-31   # v<version> or v<version>-<suffix>
git push origin v1.0.0-31
```

`<version>` must match `version` in `src-tauri/tauri.conf.json` (bump it there, in `src-tauri/Cargo.toml` and in `package.json` for a new version), or the workflow fails before building. The tag becomes the release name.

## Documentation

- [Getting started](docs/getting-started.md)
- [Architecture](docs/architecture.md)
- [Configuration](docs/configuration.md)
- [Data and backup](docs/data-and-backup.md)
- [Development notes](docs/development-notes.md)
- [Roadmap](docs/roadmap.md)
