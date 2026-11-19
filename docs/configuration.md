# Configuration

## Runtime locations

Badger uses different runtime roots depending on how the app is started:

Development (`npm run dev` / debug builds):

```text
.badger-dev/badger.db
.badger-dev/logs/
.badger-dev/backups/
.badger-dev/models/
```

Non-debug builds (installed app / release bundle):

```text
~/.badger/badger.db
~/.badger/logs/
~/.badger/backups/
~/.badger/models/
```

In development, `.badger-dev/` lives at the repo root. The database file is created on startup if it does not exist, and log files are written with daily rotation. Automatic backups are stored in the `backups/` subdirectory unless another folder is chosen in Settings. `models/` holds the semantic search model once the user enables it (see `docs/architecture.md`, Semantic search); it is created on first download.

## Key configuration files

| File | Purpose |
| --- | --- |
| `src-tauri/tauri.conf.json` | Tauri app metadata, build hooks, bundle targets, window config, icon list |
| `vite.config.ts` | Frontend dev server port, strict-port behavior, optional Tauri host/HMR wiring, and SvelteKit config (static adapter with `index.html` fallback) |
| `src-tauri/Info.plist` | Merged by Tauri into the app bundle: the Reminders usage strings that Apple Reminders sync needs (see `docs/architecture.md`, Apple Reminders sync) |
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

### Automatic backups

Folder, encryption and retention are set in Settings › Backup & Restore and stored in `app_settings`: `backup_dir` (unset = `backups/` beside the database), `backup_public_key` (unset = unencrypted) and `startup_backup_retention` (default 3, 0 = off). See `docs/data-and-backup.md` (Automatic backups).

### Semantic search dependencies

`src-tauri/Cargo.toml` adds, for semantic search:

- `ort` (pinned with `=`, default features off: `std`, `download-binaries`, `tls-rustls`, `api-22`) — ONNX Runtime bindings. **The `ort` build script downloads a prebuilt ONNX Runtime from `cdn.pyke.io` at build time**, so the first build, or any build with an empty `target/`, needs network access. The archive is checked against a SHA-256 shipped in `ort-sys` and unpacked to `~/Library/Caches/ort.pyke.io` (macOS) or `~/.cache/ort.pyke.io` (Linux); later builds reuse it, and release CI caches that directory. On the Linux dev host it is linked statically (no dylib to sign). On `aarch64-apple-darwin` (the release target) `ort` resolves its CoreML prebuilt (the session still runs on the CPU only); a local macOS build was checked by hand on 2026-11-07. That prebuilt targets macOS 13.4, so `tauri.conf.json` sets `minimumSystemVersion` to `13.4`, and its CoreML objects need `__isPlatformVersionAtLeast` from clang's runtime (`libclang_rt.osx.a`), which rustc does not link by default; `src-tauri/build.rs` links it on macOS targets (the release build failed without it once release LTO was on). The `=` pin is deliberate: `ort` 2.0 is still a release candidate, and Cargo's semver rules don't protect pre-releases, so `^2.0.0-rc.15` could pull a later rc with breaking API changes; bump it by hand
- `tokenizers` (default features off, `fancy-regex`) — the model's tokenizer, pure Rust
- `reqwest` (default features off, `rustls`) — the opt-in model download; GET only
- `sha2` — verifies the downloaded files

The workspace `Cargo.toml` sets `[profile.dev.package.tokenizers] opt-level = 3`, because the tokenizer is far too slow unoptimised for debug builds and `cargo test`. The release profile (`panic = "abort"`) is unchanged, which is why the tokenizer, inference and download paths must not panic (see `docs/architecture.md`).

The model itself is not bundled: it is downloaded at runtime from HuggingFace at a pinned commit, with SHA-256s hard-coded in `src-tauri/src/semantic/model_files.rs`. Bumping the model means updating the commit, hashes and `MODEL_ID` there, and recalibrating the similarity floor constants in `src-tauri/src/semantic/fusion.rs` (`MARGIN`, `ABSOLUTE_FLOOR`, `FIXED_FLOOR`).

### macOS code signing

`src-tauri/tauri.conf.json` sets:

```json
"macOS": {
  "signingIdentity": "-",
  "minimumSystemVersion": "13.4"
}
```

so local builds are signed ad-hoc and need no certificate. Release builds are signed with a self-signed certificate instead: `release.yml` imports it into a temporary keychain, marks it trusted for code signing, and passes its SHA-1 identity to `tauri build` through a `--config` override (`$RUNNER_TEMP/signing.conf.json`).

Why: macOS keys privacy permissions (notifications, Reminders) to the app's code-signing requirement. An ad-hoc requirement is the hash of that exact build, so every update looks like a new app and permissions reset. A certificate-based requirement stays the same across builds signed with the same certificate. It is not notarized, so a first manual `.dmg` install still needs the quarantine flag removed (see README); updates never get quarantined.

One-time setup (maintainer, on a Mac):

1. Keychain Access → Certificate Assistant → Create a Certificate…: name `Badger Code Signing`, Identity Type `Self-Signed Root`, Certificate Type `Code Signing`. Optionally tick "Let me override defaults" and set a long validity (the default is one year; an expired certificate fails the release).
2. In the login keychain, select the certificate together with its private key and export them as `badger-signing.p12` with a strong password. Back both up in a password manager.
3. Add the GitHub Actions secrets `MACOS_CERTIFICATE` (`base64 -i badger-signing.p12 | pbcopy`) and `MACOS_CERTIFICATE_PASSWORD`. Never commit them. The release fails if either is missing.

Keep using the same certificate. A new certificate changes the signing requirement once, so macOS asks for permissions again after that update. The switch from ad-hoc is such a change too. To sign a local bundled build the same way, pass `--config '{"bundle":{"macOS":{"signingIdentity":"Badger Code Signing"}}}'` to `npm run tauri build`.

### Updater and release signing

In-app updates use `tauri-plugin-updater` (Rust-only: no JS package, no capability entry). `tauri.conf.json` sets `bundle.createUpdaterArtifacts: true` and `plugins.updater` with the public key and one endpoint, `https://github.com/waldemarsson/badger/releases/latest/download/latest.json`, so the repo must stay public. The update signing key is separate from Apple code signing:

- The private key and its password live with the maintainer (generated with `npm run tauri signer generate -- -w ~/.tauri/badger.key`, backed up in a password manager) and in the GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit them. Losing either strands every install: they only accept updates signed by this key.
- The public key (`badger.key.pub`, minisign) is `plugins.updater.pubkey`. Rotating it needs one release signed with the old key that ships the new public key.

`release.yml` runs a `gates` job (the AGENTS.md gates plus a check that `pubkey` is not the `REPLACE_…` placeholder) before `release-macos`. With the secrets set, `tauri-action` also uploads `Badger.app.tar.gz`, its `.sig` and `latest.json`; `latest.json` takes its version from `tauri.conf.json`, not from the tag, so suffixed tags (`v1.3.0-1`) still work. `/releases/latest` skips drafts and pre-releases. Third-party actions are pinned to commit SHAs (version in a trailing comment).

Updates are fetched and verified in Rust, so they never get `com.apple.quarantine`; only the first manual `.dmg` install needs it removed. See `docs/architecture.md` (Updater) for the install and restart path.
