# Getting started

## Prerequisites

- Node.js for the SvelteKit and Tauri frontend toolchain
- Rust stable with Cargo for the Tauri backend
- macOS if you want to run the packaged desktop app outside a container

## First-time setup

Install JavaScript dependencies from the repo root:

```bash
npm install
```

Rust dependencies are resolved automatically by Cargo when you run backend commands.

## Run the app

Start the Vite dev server and open the Tauri shell:

```bash
npm run dev
```

This uses:

- `npm run vite:dev` for the frontend dev server on port `1420`
- Tauri development mode pointing at that dev server

Development runs started with `npm run dev` keep runtime data under repo-local `.badger-dev/` (including the SQLite database and logs). Release and other non-debug builds continue to use `~/.badger/`.

## Verification commands

Run the checks contributors are expected to use:

```bash
npm run check
npm run lint
cargo test
cargo clippy --all -- -D warnings
cargo fmt --check
```

Use `npm run format` when you want Prettier to rewrite frontend files.

## Build the app

Create the production bundle:

```bash
npm run build
```

For the macOS helper targets in the repo:

```bash
make build
make install
```

`make install` expects the app bundle to exist already and copies it into `/Applications` after removing the previous installed copy.
