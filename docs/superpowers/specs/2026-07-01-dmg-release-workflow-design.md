# Design: macOS `.dmg` Release Workflow

**Date:** 2026-07-01
**Status:** Approved

## Goal

On every push to `main`, build a macOS `.dmg` for TimeTracker (Tauri app) and
publish it as a GitHub Release. Add install instructions to the README.

## Trigger

- `push` to `main`
- `workflow_dispatch` (manual runs)

## Runner & target

- Runner: `macos-14` (native Apple Silicon).
- Architecture: `aarch64-apple-darwin` (arm64 only).

## Workflow steps (`.github/workflows/release.yml`)

1. **Checkout** — `actions/checkout@v4`.
2. **Node** — `actions/setup-node@v4` (Node 20), then `npm ci`.
3. **Rust** — `dtolnay/rust-toolchain@stable` with target `aarch64-apple-darwin`;
   `swatinem/rust-cache@v2` scoped to `src-tauri`.
4. **Build + release** — `tauri-apps/tauri-action@v0`:
   - `args: --target aarch64-apple-darwin`
   - `tagName: v__VERSION__-${{ github.run_number }}`
   - `releaseName: TimeTracker v__VERSION__-${{ github.run_number }}`
   - `releaseDraft: false`, `prerelease: false`
   - Env `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`.

`tauri-action` reads the version from `tauri.conf.json` (`__VERSION__`
placeholder), runs `tauri build`, discovers the `.dmg` artifact, creates the
release, and uploads it.

## Release naming

Tag: `v<version>-<run_number>` (e.g. `v0.1.0-42`). Unique per push, no collisions.

## Permissions

Job needs `contents: write` to create releases. `GITHUB_TOKEN` supplies auth —
no extra secrets.

## Signing

Ad-hoc signing already configured (`tauri.conf.json` -> `macOS.signingIdentity:
"-"`). No Apple Developer secrets. The `.dmg` is unsigned and un-notarized;
first launch requires right-click -> Open.

## README changes

Add an **Install** section:

- Download the latest `.dmg` from the repo Releases page.
- Open the `.dmg`, drag TimeTracker to Applications.
- First launch: right-click the app -> Open -> Open (bypasses Gatekeeper for the
  unsigned build). Note arm64-only (Apple Silicon).

## Approach considered

- **`tauri-action` (chosen)** — official, handles build + bundle discovery +
  release upload in one step.
- Hand-rolled `npm run build` + `gh release create` — more moving parts, manual
  artifact path handling. Rejected.

## Out of scope

- Apple code signing / notarization.
- Intel or universal builds.
- Linux / Windows targets.
