# TimeTracker

TimeTracker is a Tauri desktop app for logging time locally against customers, projects, and issues. The frontend is a SvelteKit single-page app running inside a Tauri WebView, and the backend is a Rust command layer that stores data in a local SQLite database.

## Quick start

```bash
npm install
npm run dev
```

The app starts the Vite dev server and launches the Tauri shell against it.

## Install (macOS)

Pre-built `.dmg` releases are published automatically on every push to `main`.

1. Go to the [Releases](../../releases) page and download the latest `.dmg`.
2. Open the `.dmg` and drag **TimeTracker** into your `Applications` folder.
3. On first launch macOS blocks the app because it is unsigned. Remove the
   Gatekeeper quarantine flag from a terminal:

   ```bash
   xattr -dr com.apple.quarantine /Applications/TimeTracker.app
   ```

   Then open the app normally. If macOS still reports the app as "damaged",
   re-sign it ad-hoc and retry:

   ```bash
   codesign --force --deep --sign - /Applications/TimeTracker.app
   xattr -dr com.apple.quarantine /Applications/TimeTracker.app
   ```

The build is arm64-only (Apple Silicon) and unsigned/un-notarized, which is why
the quarantine step above is required.

Development runs started with `npm run dev` store their SQLite database and logs under repo-local `.timetracker-dev/` so they stay isolated from the live `~/.timetracker/` data used by non-debug builds.

## Common commands

| Command | Purpose |
| --- | --- |
| `npm run dev` | Run the desktop app in development mode |
| `npm run build` | Build the production app bundle |
| `npm run check` | Run Svelte and TypeScript checks |
| `npm run lint` | Run frontend linting |
| `npm run format` | Format the repo with Prettier |
| `cargo test` | Run Rust unit tests |
| `cargo clippy --all -- -D warnings` | Run Rust linting |
| `cargo fmt --check` | Check Rust formatting |
| `make build` | Build the macOS app bundle |
| `make install` | Copy the built `.app` into `/Applications` |

## Documentation

- [Getting started](docs/getting-started.md)
- [Architecture](docs/architecture.md)
- [Configuration](docs/configuration.md)
- [Data and backup](docs/data-and-backup.md)
- [Development notes](docs/development-notes.md)
- [Roadmap](docs/roadmap.md)

## Project structure

```text
src/              SvelteKit frontend
src-tauri/        Rust Tauri backend
assets/           App assets used by the project
```
