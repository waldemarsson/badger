# Development notes

## Backend command pattern

The Rust backend exposes Tauri commands that return `Result<T, AppError>`. `AppError` is serialized as a display string, so frontend code receives user-visible error text instead of a custom error object graph.

This keeps command boundaries simple:

- frontend calls `invoke()`
- backend validates and queries
- success returns JSON data
- failure returns a serialized error string

## Shared test database fixture

Rust tests use a shared `test_db()` helper defined at crate root. It creates an in-memory SQLite pool and applies the schema before each test module uses it.

That pattern matters because it keeps backend tests:

- isolated from the real user database
- fast enough for unit-level command testing
- aligned with the production schema

## Development-only seed data

`src-tauri/src/dev_seed.rs` is compiled only in debug builds. The command is registered only when `debug_assertions` are enabled, which means:

- seed tooling is available during local development
- release builds do not expose the seed command

Debug builds also keep their runtime database and logs under repo-local `.timetracker-dev/`, which keeps seed workflows away from the live `~/.timetracker/` data used by non-debug builds.

The seed workflow populates sample customers, projects, issues, and time entries anchored to recent dates so reporting screens have realistic data to display.

## Report and restore behavior worth knowing

- Weekly and monthly reports are computed in Rust rather than assembled entirely in the frontend
- Backup export/import also stays in Rust so file access and SQLite writes remain on the native side
- The Settings page currently contains the active backup/restore UI even though a `/backup` route still exists as a placeholder

## Formatting and lint split

Frontend formatting and linting are handled by the Node toolchain, while Rust quality checks are handled by Cargo commands. `eslint.config.js` explicitly ignores generated directories and the Rust backend tree, so JavaScript linting expectations stop at the frontend boundary.
