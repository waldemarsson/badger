# Configuration

## Runtime locations

TimeTracker uses different runtime roots depending on how the app is started:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/timetracker.db
.timetracker-dev/logs/
```

Non-debug builds (installed app / release bundle):

```text
~/.timetracker/timetracker.db
~/.timetracker/logs/
```

In development, `.timetracker-dev/` lives at the repo root. The database file is created on startup if it does not exist, and log files are written with daily rotation.

## Key configuration files

| File | Purpose |
| --- | --- |
| `src-tauri/tauri.conf.json` | Tauri app metadata, build hooks, bundle targets, window config, icon list |
| `vite.config.ts` | Frontend dev server port, strict-port behavior, optional Tauri host/HMR wiring |
| `svelte.config.js` | SvelteKit static adapter setup with `index.html` fallback |
| `.cargo/config.toml` | Forces SQLx offline mode |
| `eslint.config.js` | Frontend linting rules and ignored generated/backend paths |
| `.devcontainer/devcontainer.json` | Optional containerized development environment |

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

### Ad-hoc macOS signing is enabled

`src-tauri/tauri.conf.json` sets:

```json
"macOS": {
  "signingIdentity": "-"
}
```

That tells Tauri to build with ad-hoc signing rather than expecting a configured signing identity.

## Optional devcontainer configuration

The devcontainer adds an opinionated environment for working inside a container. It includes Node, Docker-in-Docker, Rust tooling, Copilot CLI, and Claude Code integration. It also bind-mounts `.devcontainer/.env` into `/run/secrets/dev.env` so the post-create script can source local secrets without copying them into the repo.

Use this only if you want the container workflow; it is not required for normal project setup.
