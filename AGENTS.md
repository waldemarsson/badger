# Agent instructions

Badger is a local-first macOS desktop app: a SvelteKit SPA (Svelte 5, Tailwind 3) inside a Tauri 2 WebView, with a Rust backend that owns a SQLite database. Read `docs/architecture.md` for the big picture; this file holds the rules and gotchas.

## Gates — all must pass before handing off

```bash
cargo test                          # Rust unit tests (in-memory SQLite)
cargo clippy --all -- -D warnings
cargo fmt --check
npm run check                       # svelte-check + TypeScript
npm run lint
node --test tests/*.mjs             # frontend unit tests
```

Prettier is not enforced and many files predate it. Do not reformat files you did not otherwise change.

CI only builds releases: every push to `main` publishes a new macOS release. Nothing runs these gates for you.

## Ground rules

- **Rust owns persistence and the file system.** The frontend only calls `invoke()`. It never touches SQLite or files directly.
- **SPA only.** `ssr = false`; Tauri APIs exist only in the WebView at runtime.
- **Dev data is isolated.** Debug builds use repo-local `.badger-dev/`. Never read, write, or delete `~/.badger/` — that is the user's real data.
- **No new dependencies** (npm or cargo) without asking. Prefer the stdlib, Svelte built-ins (`svelte/transition`, `svelte/animate`), and what is already in `package.json` / `Cargo.toml`.
- Match the surrounding file's style and indentation; files differ.

## Backend (Rust / Tauri)

Pattern for every command module in `src-tauri/src/commands/` (see `todos.rs` as the reference):

1. `pub async fn inner_<name>(…, pool: &Pool<Sqlite>) -> Result<T, AppError>` holds the logic and validation.
2. A thin `#[tauri::command] pub async fn <name>(…, pool: State<'_, Pool<Sqlite>>)` wrapper calls it.
3. Unit tests in the same file call `inner_*` against `crate::test_db()`.

Gotchas:

- **Register new commands in both `generate_handler!` lists** in `src-tauri/src/lib.rs` (debug and release). Missing the release list breaks only the shipped app.
- **Argument names:** the frontend passes camelCase (`invoke('x', { projectId })`), and Tauri maps it to snake_case Rust parameters (`project_id`).
- **Columns are PascalCase** (`ProjectId`); structs are snake_case. Alias every selected column: `ProjectId AS project_id`.
- **Use runtime queries** (`sqlx::query`, `sqlx::query_as`), not the `query!` macros. `SQLX_OFFLINE` is set and there is no `.sqlx/` metadata.
- **Errors** are `AppError` and reach the user as their display string, so write messages for the user (`"Title is required"`). Return `AppError::Custom` for validation and not-found errors; never panic in commands.
- **Multi-step writes** go in a transaction (`pool.begin()`).
- **Note write paths must also feed the semantic index.** Call the `*_and_queue` / `delete_note_and_evict` helpers in `commands/notes.rs` (not bare `inner_*`) from wrappers so the note is re-embedded; see `docs/architecture.md` (Semantic search).
- **Reminder-affecting write paths must call `reminders_scheduler::reminders_changed(&app, pool)`** after committing (reminder commands, and todo/note wrappers that cascade-delete, complete or retitle linked reminders; restore). It emits `reminders-changed` and recomputes the dock badge; without it the UI and badge go stale until the next 30 s tick. Todo-done completion of reminders belongs inside `inner_move_todo`'s transaction. See `docs/architecture.md` (Reminders).
- **Semantic tests never download or load the model.** Use `FakeEmbedder` and `semantic::index::tests::ready_state`. Real-model tests are `#[ignore]`d; run them with `BADGER_MODEL_DIR=<dir> cargo test -p badger real_model -- --ignored --test-threads=1`.
- **`ort` downloads ONNX Runtime from `cdn.pyke.io` at build time**, so the first build (and any clean `target/`) needs network. Never add `unwrap`/`expect` on tokenizer, inference or download paths: release is `panic = "abort"`.
- **Quit/close must go through `flush_guard.rs`** (flushes unsaved notes). Don't restore the native macOS Quit menu item or exit via `std::process::exit`; both bypass `RunEvent::ExitRequested`.

### Schema changes

- Tables are created by `SCHEMA_SQL` in `src-tauri/src/db.rs` with `CREATE TABLE IF NOT EXISTS`. That does **not** alter existing tables.
- Changing an existing table needs an idempotent migration function in `db.rs`, called from `init_db` after `SCHEMA_SQL`, with a test that builds the old shape and migrates it (see `migrate_todos_status`).
- Foreign keys are on. Choose `ON DELETE CASCADE` vs `SET NULL` on purpose and test it.
- **Every new table must be included in backup/restore** (`commands/backup.rs`), except derived indexes (`notes_fts`, `note_refs`, `note_embeddings`), which restore rebuilds: export, restore (overwrite-only: add an explicit `DELETE`, since cascades won't reach a new table), and `#[serde(default)]` on the new `BackupData` field so older backups still restore. Add a round-trip test.

## Frontend (Svelte 5 / SvelteKit)

- **Runes only:** `$state`, `$derived`, `$props`, `$effect`, `{#snippet}`/`{@render}`. No `export let`, no `$:`, and no stores (use `$app/state`, not `$app/stores`).
- **Types:** keep `src/lib/types.ts` in sync with the Rust structs (snake_case fields).
- **Imports:** SvelteKit 3 removed `$lib`; use `#lib/...` (package.json `imports`) with an explicit extension (`#lib/types.js`, `#lib/x.svelte.js`, `#lib/components/X.svelte`). Extensionless `#lib` imports build but fail `npm run check`.
- **Config:** SvelteKit config lives in the `sveltekit({...})` plugin in `vite.config.ts`; `svelte.config.js` is no longer supported.
- **Pure helpers** go in `src/lib/*.ts`; test them with `node:test` under `tests/`.
- **Styles:** use Tailwind utilities in markup. Shared or `@apply` classes go in `src/app.css` under `@layer components`, not in component `<style>` blocks, because svelte-check warns about `@apply` there.
- **Dynamic classes:** Tailwind only emits `app.css` component classes whose full name appears literally in `src/**` (`content` scan). Write out `cm-link-kind-todo-in_progress` in full; never build it as `` `cm-link-kind-${x}` ``.
- **Icons:** `@lucide/svelte`, imported per icon (`import X from '@lucide/svelte/icons/x'`).
- **Routes:** add new pages to `navLinks` in `src/routes/+layout.svelte` (it feeds ⌘K; `as const` keeps `resolve()` type-checked), and to a group in `navGroups` if the page gets a nav tab. The two reports share one "Reports" tab and switch with `ReportSwitch.svelte`; Settings is the gear.
- **Pickers:** reuse `src/lib/components/Combobox.svelte` for customer, project, and issue selection.
- **Menus:** reuse `src/lib/components/ContextMenu.svelte` (items `{label, icon, hint, danger, run}`) for popup action menus; position with `placeMenu` in `src/lib/context-menu.ts`.
- `svelte-ignore state_referenced_locally` is acceptable only for intentionally snapshotting a prop at mount (see `TodoModal.svelte`).

## UI / UX

The user cares about polish. A plain CRUD table was rejected for todos and replaced by a kanban board, so design the interaction for the workflow, not the data shape.

Design language (follow existing pages before inventing):

- Palette: slate neutrals, with sky for active/drag, emerald for done, red for overdue/errors, amber for due today, and violet for issue links.
- Dark mode: every color utility needs a `dark:` counterpart (class strategy). Dark uses the `neutral` grays (not slate, which reads blue): `neutral-950` page / `neutral-900` card / `neutral-800` ring, text `neutral-100`/`neutral-400`, accents `-400` text on `-500/10` tints. Prefer `app.css` primitives, which already handle it. Screenshot both themes.
- Surfaces: `rounded-xl`/`rounded-2xl`, `ring-1 ring-slate-200`, `shadow-sm`, lifting on hover (`hover:-translate-y-0.5 hover:shadow-md`).
- No page title bands: the active nav tab already names the page. Each page has an `sr-only` `<h1>` and starts directly with content in `table-card` / `panel-card` or purpose-built layouts. Put totals beside the control that scopes them (`toolbar-stat*` in a `table-toolbar`), not in a header.
- Reuse the `app.css` primitives: `btn-primary`, `btn-secondary(-sm)`, `btn-danger(-sm)`, `btn-icon-*`, `form-input`, `form-select`, `form-textarea`, `form-label`, `form-error`, `alert-*`.
- States: show an empty state ("No cards"), show inline errors near the action, and confirm destructive actions inline rather than with `confirm()`.
- Motion: short (150–200 ms) `fade`/`scale`/`flip` from Svelte. No animation libraries.
- Keyboard: Enter submits, Esc cancels or closes, and clickable cards are focusable (`role="button"`, `tabindex="0"`).
- Drag and drop: use pointer events, not HTML5 DnD (Tauri's file-drop handling can block it). See `src/routes/todos/+page.svelte`.

**Check UI work visually before handing off.** The app can't run headless, but the built SPA can:

1. `npm run vite:build`
2. Use Playwright (`playwright-core` installed outside the repo, with a browser from `~/.cache/ms-playwright`) to load `build/` via `page.route`, falling back to `index.html` for SPA routes.
3. Mock the backend with `page.addInitScript`, setting `window.__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: async (cmd, args) => … }` and returning realistic data per command.
4. Screenshot the key states (empty, populated, error, mid-interaction) and look at them. Say in the handoff that this was a mocked render and not the Tauri shell.

## Git and PRs

- Branches: `feature/<short_description>` or `bugfix/<short_description>`. Never commit to `main`.
- Commit messages use conventional prefixes: `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`.
- Open PRs as drafts. Report verification honestly, including checks that could not run.

## Reference

- `docs/architecture.md` — structure, data flow, storage locations
- `docs/data-and-backup.md` — schema, backup format, restore modes
- `docs/development-notes.md` — command pattern, test fixture, dev seed
- `docs/configuration.md` — Tauri, build, and tooling config
- `docs/roadmap.md` — planned work in priority order
