# Configuration

## Runtime locations

TimeTracker uses different runtime roots depending on how the app is started:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/timetracker.db
.timetracker-dev/logs/
.timetracker-dev/backups/
.timetracker-dev/models/
```

Non-debug builds (installed app / release bundle):

```text
~/.timetracker/timetracker.db
~/.timetracker/logs/
~/.timetracker/backups/
~/.timetracker/models/
```

In development, `.timetracker-dev/` lives at the repo root. The database file is created on startup if it does not exist, and log files are written with daily rotation. Automatic startup backups are stored in the `backups/` subdirectory when enabled. `models/` holds the semantic search model once the user enables it (see `docs/architecture.md`, Semantic search); it is created on first download.

## Key configuration files

| File | Purpose |
| --- | --- |
| `src-tauri/tauri.conf.json` | Tauri app metadata, build hooks, bundle targets, window config, icon list |
| `vite.config.ts` | Frontend dev server port, strict-port behavior, optional Tauri host/HMR wiring, and SvelteKit config (static adapter with `index.html` fallback) |
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

### Content Security Policy

`app.security.csp` (release) and `app.security.devCsp` (debug, adds the Vite HMR websocket) allow only the app's own scripts, styles, fonts and images (plus `data:` images for the inline SVG icons in CSS) and only Tauri IPC connections (`ipc:`, `http://ipc.localhost`). Tauri hashes the inline scripts in `index.html` (the theme script in `src/app.html` and SvelteKit's bootstrap) into `script-src` at build time. `dangerousDisableAssetCspModification: ["style-src"]` stops Tauri adding style nonces, which would make browsers ignore `'unsafe-inline'` and break the inline styles that Svelte and CodeMirror set. Loading anything new (a remote font, image or API) needs a CSP change.

### SvelteKit is configured for a desktop SPA

The `sveltekit()` plugin in `vite.config.ts` uses `@sveltejs/adapter-static` with an `index.html` fallback. Combined with `src/routes/+layout.ts` setting `ssr = false`, this keeps the frontend compatible with the Tauri WebView runtime.

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

### Semantic search dependencies

`src-tauri/Cargo.toml` adds, for semantic search:

- `ort` (pinned with `=`, default features off: `std`, `download-binaries`, `tls-rustls`, `api-22`) — ONNX Runtime bindings. **The `ort` build script downloads a prebuilt ONNX Runtime from `cdn.pyke.io` at build time**, so the first build, or any build with an empty `target/`, needs network access. The archive is checked against a SHA-256 shipped in `ort-sys` and unpacked to `~/Library/Caches/ort.pyke.io` (macOS) or `~/.cache/ort.pyke.io` (Linux); later builds reuse it, and release CI caches that directory. On the Linux dev host it is linked statically (no dylib to sign). On `aarch64-apple-darwin` (the release target) `ort` resolves its CoreML prebuilt (the session still runs on the CPU only); a local macOS build was checked by hand on 2026-11-07. That prebuilt targets macOS 13.4, so `tauri.conf.json` sets `minimumSystemVersion` to `13.4`, and its CoreML objects need `__isPlatformVersionAtLeast` from clang's runtime (`libclang_rt.osx.a`), which rustc does not link by default; `src-tauri/build.rs` links it on macOS targets (the release build failed without it once release LTO was on). The `=` pin is deliberate: `ort` 2.0 is still a release candidate, and Cargo's semver rules don't protect pre-releases, so `^2.0.0-rc.15` could pull a later rc with breaking API changes; bump it by hand
- `tokenizers` (default features off, `fancy-regex`) — the model's tokenizer, pure Rust
- `reqwest` (default features off, `rustls`) — the opt-in model download; GET only
- `sha2` — verifies the downloaded files

The workspace `Cargo.toml` sets `[profile.dev.package.tokenizers] opt-level = 3`, because the tokenizer is far too slow unoptimised for debug builds and `cargo test`. The release profile (`panic = "abort"`) is unchanged, which is why the tokenizer, inference and download paths must not panic (see `docs/architecture.md`).

The model itself is not bundled: it is downloaded at runtime from HuggingFace at a pinned commit, with SHA-256s hard-coded in `src-tauri/src/semantic/model_files.rs`. Bumping the model means updating the commit, hashes and `MODEL_ID` there, and recalibrating the similarity floor constants in `src-tauri/src/semantic/fusion.rs` (`MARGIN`, `ABSOLUTE_FLOOR`, `FIXED_FLOOR`).

### Ad-hoc macOS signing is enabled

`src-tauri/tauri.conf.json` sets:

```json
"macOS": {
  "signingIdentity": "-",
  "minimumSystemVersion": "13.4"
}
```

That tells Tauri to build with ad-hoc signing rather than expecting a configured signing identity.
