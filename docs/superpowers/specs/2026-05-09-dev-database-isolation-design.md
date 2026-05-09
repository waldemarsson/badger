# Dev database isolation design

## Problem

`npm run dev` currently launches the Tauri backend against the same runtime location used by the live app:

- `~/.timetracker/timetracker.db`
- `~/.timetracker/logs/`

That creates unnecessary risk during development because local debugging, seed operations, or schema experiments can affect the live user data directory.

## Goal

Make debug and development runs use repo-local runtime data by default, while preserving the existing home-directory behavior for non-debug builds.

## Non-goals

- Changing the production database location
- Introducing user-configurable database switching in the app UI
- Changing the SQLite schema or backup format

## Recommended approach

Add a small backend path-resolution helper that returns runtime locations for both the database and logs.

- In debug builds, resolve paths under the repository directory, for example:
  - `<repo>/.timetracker-dev/timetracker.db`
  - `<repo>/.timetracker-dev/logs/`
- In non-debug builds, keep the current locations:
  - `~/.timetracker/timetracker.db`
  - `~/.timetracker/logs/`

## Why this approach

1. It makes the safe behavior automatic for normal development.
2. It removes reliance on developers remembering an alternate command or environment variable.
3. It keeps release behavior unchanged and predictable.

## Design details

### Runtime path resolution

Introduce a helper in the Rust backend that centralizes runtime path selection.

- Debug build behavior should use a repo-relative directory derived from the backend source location, not from the process working directory.
- Release behavior should continue using `app.path().home_dir()?` and `.timetracker`.

Using a compile-time repo reference avoids accidental variation when commands are started from different shells or IDEs.

### Startup wiring

Update the Tauri setup flow so both logging and database initialization use the shared runtime paths helper rather than duplicating path construction inline.

### Data isolation

The repo-local dev directory should be outside source folders that are already committed, but inside the repository so it is easy to inspect and remove locally. A hidden folder such as `.timetracker-dev/` is appropriate.

The folder should be gitignored so dev database files and logs are never accidentally committed.

### Documentation

Update development-facing docs to make the split explicit:

- `npm run dev` uses repo-local runtime data
- packaged/release builds continue using `~/.timetracker`

## Affected files

- `src-tauri/src/lib.rs`
- potentially a new backend helper module for runtime paths
- `.gitignore`
- development/configuration docs that describe runtime paths

## Testing

- Add backend tests for the path-resolution helper to verify debug and non-debug path selection logic where practical.
- Keep existing database initialization behavior unchanged apart from the resolved path.
- Confirm docs reflect the new development behavior.
