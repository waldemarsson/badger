# Configuration

## Runtime locations

TimeTracker uses different runtime roots depending on how the app is started:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/timetracker.db
.timetracker-dev/logs/
.timetracker-dev/backups/
```

Non-debug builds (installed app / release bundle):

```text
~/.timetracker/timetracker.db
~/.timetracker/logs/
~/.timetracker/backups/
```

In development, `.timetracker-dev/` lives at the repo root. The database file is created on startup if it does not exist, and log files are written with daily rotation. Automatic startup backups are stored in the `backups/` subdirectory when enabled.

## Key configuration files

| File | Purpose |
| --- | --- |
| `src-tauri/tauri.conf.json` | Tauri app metadata, build hooks, bundle targets, window config, icon list |
| `vite.config.ts` | Frontend dev server port, strict-port behavior, optional Tauri host/HMR wiring |
| `svelte.config.js` | SvelteKit static adapter setup with `index.html` fallback |
| `.cargo/config.toml` | Forces SQLx offline mode |
| `eslint.config.js` | Frontend linting rules and ignored generated/backend paths |

## Build and runtime settings that matter

### Tauri frontend handoff

`src-tauri/tauri.conf.json` tells Tauri to:

- run `npm run vite:dev` before dev launches
- use `http://localhost:1420` as the dev URL
- run `npm run vite:build` before production builds
- load the built frontend from `../build`

That means the Vite output directory and the Tauri config have to stay aligned.

### SvelteKit is configured for a desktop SPA

`svelte.config.js` uses `@sveltejs/adapter-static` with an `index.html` fallback. Combined with `src/routes/+layout.ts` setting `ssr = false`, this keeps the frontend compatible with the Tauri WebView runtime.

### SQLx runs in offline mode

`.cargo/config.toml` sets:

```toml
[env]
SQLX_OFFLINE = "true"
```

This means compile-time query checking uses offline metadata instead of requiring a live SQLite database during builds.

### Logging can be filtered through environment

The logger uses `tracing_subscriber::EnvFilter::try_from_default_env()` and falls back to `info`. Set `RUST_LOG` when you need more or less backend logging detail during development.

### Startup backup retention

TimeTracker automatically creates a backup on each application startup. The retention setting controls how many recent backups are kept:

- **Default**: 3 backups
- **Retention > 0**: Backup is created on each startup (overwriting any existing backup for today) and old backups are trimmed to keep the specified number of recent backups
- **Retention = 0**: Automatic startup backups are disabled

The retention setting can be configured in the Settings > Backup & Restore tab. The setting is stored in the `app_settings` table with key `startup_backup_retention`.

### Ad-hoc macOS signing is enabled

`src-tauri/tauri.conf.json` sets:

```json
"macOS": {
  "signingIdentity": "-"
}
```

That tells Tauri to build with ad-hoc signing rather than expecting a configured signing identity.
