# Architecture

## Overview

TimeTracker is a local-first desktop application built as a Tauri shell around a SvelteKit single-page app. The frontend renders the user interface and calls Rust commands through Tauri's `invoke()` bridge. The Rust backend owns persistence, validation, reporting queries, backup/restore logic, and file-system access.

## Main parts

### Frontend

- Located under `src/`
- Uses SvelteKit in SPA mode with `ssr = false`
- Uses Tauri's WebView runtime instead of a server-rendered deployment model
- Calls backend commands with `@tauri-apps/api/core`

The main user-facing areas are:

- `/` — Today: agenda of todos to act on (due today or overdue, in progress), today's logged time, and a Mon–Sun week strip with progress against the daily hours target
- `/timesheet` — timesheet entry and editing; date filter includes Single Day, and `?day=YYYY-MM-DD` opens it on that day (an invalid `day` is ignored)
- `/todos` — kanban board of standalone todos (drag cards between status columns), optionally linked to an issue
- `/reports/weekly` — ISO-week report view
- `/reports/monthly` — month report view with expandable grouping, the month's travel, and a copy-month text export
- `/notes` — a folder tree of notes on the left (folders first A–Z, then notes pinned first, then most recently updated; see Notes folders) that becomes a flat list of ranked hits with highlighted title, snippet and folder path while searching via the `search_notes` command, and a markdown live-preview editor on the right; the selected note lives in the URL (`?id=12`)
- `/travel` — month-scoped list of business trips (date, from/to, optional km and kr) with inline add, edit, repeat, and delete
- `/settings` — customer and project management (cards with inline add, rename, default, and delete), preferences (System/Light/Dark appearance), backup/restore, and dev-only seed tooling

### Backend

- Located under `src-tauri/src/`
- Starts from `main.rs` and delegates to `time_tracker_lib::run()`
- Registers Tauri commands in `lib.rs`
- Stores a shared SQLite pool and logging guard in Tauri managed state

The backend is organized around command modules:

- `customers` and `projects` for reference data (names are trimmed and unique case-insensitively: customers globally, projects per customer)
- `issues` and `time_entries` for core timesheet operations
- `todos` for standalone todos and their optional issue link
- `travel` for business trips, listed per month
- `folders` for note folders (`list_folders`, `create_folder`, `rename_folder`, `move_folder`, `delete_folder`) and `move_note`; see Notes folders
- `notes` for markdown notes (create, rename, body save, pin, delete), the list excerpt, which is computed in Rust, full-text search (`search_notes`, `search_index_stats`, `rebuild_search_index`), and links (`create_note_titled`, `list_tag_suggestions`, `get_link_targets`, `list_backlinks`)
- `semantic` for semantic note search: the lifecycle commands (`get_semantic_status`, `enable_semantic_search`, `cancel_semantic_download`, `remove_semantic_model`, `rebuild_semantic_index`) live in `commands/semantic.rs`; the engine is the top-level `semantic/` module (not a command module); see Semantic search
- `markdown` for `render_markdown`, which turns note markdown into safe HTML (read view, copy as HTML)
- `note_links` (a top-level module, not a command module) for the `#tag` / `[[wikilink]]` extractor and entity slugs; see Notes links
- `reports` for weekly and monthly rollups
- `backup` for JSON export and import
- `settings` for app preferences (theme, daily hours target, command palette shortcut and last-used project/issue, expanded note folders) stored in `app_settings`
- `startup_backup` for automatic daily backups on application startup
- `flush_guard` for the quit/close guard (see Notes editor)
- `dev_seed` for debug-only sample data

## Data flow

The core request path is:

```text
Svelte component
  -> invoke("command_name", payload)
  -> Tauri command in Rust
  -> sqlx query against SQLite
  -> serialized result back to the frontend
```

### Theme

Dark mode uses Tailwind's `darkMode: "class"`: a `dark` class on `<html>` activates the `dark:` variants in `app.css` and markup. Dark surfaces use Tailwind's `neutral` grays; light mode uses `slate`. The theme mode (`system`, `light`, `dark`) is stored in `app_settings` via `get_theme`/`set_theme`; `src/lib/theme-state.svelte.ts` applies it (class, CSS `color-scheme`, native window theme) and mirrors it to `localStorage` so the inline script in `src/app.html` paints the right theme before the app loads. The setting is not part of backups.

### Command palette

`src/lib/components/CommandPalette.svelte` is mounted in the root layout and listens for its shortcut on `window` in the capture phase, so it opens over text fields and modals and Esc closes only the palette. The shortcut (default `Meta+K`, configurable in Settings › Preferences) and the last project/issue used for Log time live in `app_settings` (`get_palette_settings`, `set_palette_shortcut`, `set_palette_last_used`) and are not part of backups. Shortcut rules (needs ⌘/⌃/⌥, macOS-reserved combos refused), filtering and hours/date parsing are pure helpers in `src/lib/palette.ts`. After creating data the palette bumps a counter in `src/lib/palette-state.svelte.ts` (`changes.todos` / `changes.timeEntries`); the Todos and Timesheet pages reload when it changes. The same module drives the layout's `Toast`. Pages can open the Log time flow with `requestLogTime(prefill?)` in the same module: the palette consumes the request, and a prefill with a project and issue (Today's todo [Log] button) jumps straight to the hours step. If the prefill's project no longer exists, the normal project step is shown.

### Today

Today (`src/routes/+page.svelte`) aggregates in the frontend: it loads the current week's `list_time_entries`, `list_todos` and the daily target, and the pure selection, ordering and totals helpers live in `src/lib/today.ts` (tested in `tests/today.test.mjs`). There are no dedicated list/report commands. Needs attention is non-done todos due today or earlier; In progress is the remaining `in_progress` todos, so no todo appears twice. The week runs Monday–Sunday and the weekly target is 5 × the daily target. The daily target is stored in `app_settings` via `get_daily_target`/`set_daily_target` (default 8, range 0–24, 0 turns the progress UI off, an invalid stored value falls back to 8), edited in Settings › Preferences, and is not part of backups. The page reloads when the palette bumps `changes`.

### Notes editor

The editor is CodeMirror 6 (`@codemirror/*` packages only: no `codemirror` meta-package or basicSetup), hosted by `src/lib/components/NoteEditor.svelte`. The live preview is written in this repo under `src/lib/editor/` rather than taken from a library, because the existing ones are alpha or stale. It hides markdown markers except on the lines touched by the selection and renders GFM tables as a block widget (raw source shows while the cursor is inside the table). Markdown links are styled but not clickable in the editor (they are in the read view); tags, chips and wikilinks follow on ⌘-click (see Notes links). Images stay as raw markdown. Editor colors are CSS variables in `src/app.css` for light and `.dark`, so a theme change needs no editor reconfigure. Each note keeps its own undo history while the page is open.

Fenced code is highlighted per language via `@codemirror/language-data` (each language is a lazy chunk, loaded on first use; unknown languages stay plain), with code colors scoped to code so they never restyle markdown. Typing `/` at a line start or after whitespace (not in words, URLs or code) opens a snippet menu (`@codemirror/autocomplete`): table, code block, task list, callout and today's date; the pure rules are in `src/lib/slash-menu.ts`. The `#` and `[[` link menus are two more sources in the same `autocompletion` config (`override`, so no word completion).

The title is a separate field above the editor, not derived from a heading, because wikilinks need a stable, unique, user-controlled name. Rename commits on Enter or blur; Esc reverts.

Autosave is a pure queue in `src/lib/note-save-queue.ts` (tested in `tests/note-save-queue.test.mjs`): 800 ms after the last edit, plus a flush on note switch, window blur, the tab becoming hidden and leaving `/notes`. One save per note is in flight at a time and the latest text wins. Unchanged text is never sent. A failed save keeps the note in a page-level dirty set and is retried on every edit and flush; navigating away from `/notes` is blocked until it succeeds. ⌘N on the page creates a note; the palette's "New note" navigates to `/notes?new=1`, which runs the same create path (title focused).

#### Read view and copy as HTML

⌘E (or the book/pencil button in the title bar) swaps the editor for a read view; the mode survives note switches and resets on page load, and creating a note always returns to edit. The editor stays mounted but hidden, so each note keeps its state. The read view renders the editor's current text, unsaved edits included, through the `render_markdown` command (`src-tauri/src/commands/markdown.rs`, `pulldown-cmark` with tables, strikethrough and task lists). It takes a `mode` (`read` or `copy`) and the database pool, because tags, chips and wikilinks are resolved against current data (see Notes links), so the same text can render differently after a rename.

The WebView has a strict CSP (`app.security.csp` in `src-tauri/tauri.conf.json`: no inline or remote scripts, no remote connections or images), but Rust also locks the HTML down itself: raw HTML is escaped and shown as text, only `http:`, `https:` and `mailto:` links are kept (others render as their text), and images never load (they become links labelled with their alt text). `src/lib/components/NoteReader.svelte` inserts the result, highlights code blocks with the editor's parsers and colors (`src/lib/editor/highlight-code.ts`), and opens links in the default browser through `tauri-plugin-opener`, checking the scheme again (`src/lib/links.ts`); a link never navigates the WebView. The capability grants only `opener:allow-open-url` with `opener:allow-default-urls` (http, https, mailto, tel).

"Copy as HTML" writes the same rendered HTML (`copy` mode: tags, wikilinks and chips become plain text) as `text/html` and the markdown source as `text/plain`, using a `ClipboardItem` whose HTML is a promise so WebKit's user-gesture requirement holds across the `invoke`.

#### Quit/close flush guard

An edit within the debounce window must survive ⌘Q. `src-tauri/src/flush_guard.rs` holds window close and app exit (`WindowEvent::CloseRequested` and `RunEvent::ExitRequested`), emits the `app://flush-request` event, and waits up to 2 s for the frontend to call the `flush_done` command, then closes or exits for real. A flag lets the re-entrant close/exit that Rust triggers itself pass through, so it does not loop. On the frontend, the root layout always listens for the event and runs the handlers pages registered with `registerFlushHandler` (`src/lib/flush-guard.ts`; `/notes` registers one that flushes all dirty notes), then calls `flush_done`. With no handler it replies immediately. No capability change was needed.

On macOS the native app-menu Quit item sends `terminate:`, which tao turns into an unpreventable exit that skips `ExitRequested`. At startup `install_quit_menu` therefore replaces it with a custom "Quit" item (id `app-quit`, accelerator `CmdOrCtrl+Q`) that calls `AppHandle::exit`, which does go through the guard. It only applies when the default menu's app submenu ends in a Quit item; otherwise it logs a warning and keeps the native menu. The guard is unit-tested for its state machine only; it has not been verified in the real Tauri shell.

The frontend keeps TypeScript interfaces in `src/lib/types.ts` aligned with the Rust response shapes so route components can work with typed command results.

### Notes search

`search_notes` (in `commands/notes.rs`) queries the `notes_fts` FTS5 table (after splitting off `#tag` terms, see Notes links), which holds each note's title and a plain-text body from `markdown::to_plain_text` (markers and markdown link/image destinations dropped; raw HTML and autolinks are indexed as written). The `trigram` tokenizer matches case-insensitive substrings, which suits Swedish compounds; it can't match terms under 3 characters, so those filter the rows in Rust (Unicode case folding) without highlights. Every whitespace-separated term must match and is quoted, so FTS5 operators are plain text. Results are ranked with `bm25` (title weighted 10×), and hits come back wrapped in `\u0002`/`\u0003`, which are stripped from indexed text. The frontend splits them into `<mark>` segments (`splitHighlights` in `src/lib/notes.ts`) and never uses `{@html}`. The list re-searches only when the query changes, and rows are drawn from the current summaries, so the note you are editing never drops out mid-edit. The palette shows the top 5 hits under its commands once the query has 3 characters; shorter queries list commands only.

`search_notes` takes an optional `hybrid` argument (default false, which is exactly the keyword search above). With it, and semantic search ready, semantic hits are fused in; see Semantic search.

### Semantic search

Opt-in hybrid search: an embedding model finds notes by meaning (Swedish and English, cross-language) and its hits are fused with the FTS5 keyword hits. Nothing downloads until the user clicks Enable in Settings › Preferences › Semantic search; until then, search is keyword-only and the notes search shows a hint linking to Settings.

**Model and runtime.** `ibm-granite/granite-embedding-311m-multilingual-r2` (Apache 2.0, ModernBERT, Swedish among its enhanced-support languages) as IBM's own int8 ONNX export (`onnx/model_quint8_avx2.onnx`, about 313 MB plus a 33 MB tokenizer, 768 dimensions; the model accepts 32k tokens, inputs are truncated to 512), run in-process on the CPU only (CoreML, which `ort` would otherwise register on macOS, is disabled so vectors are deterministic and match the calibration), by `ort` (ONNX Runtime, a prebuilt downloaded at build time; see `docs/configuration.md`) with the `tokenizers` crate. One fixed model, no picker; vectors store the model id, so a different model re-embeds everything. Granite takes no query or passage prompt and uses CLS pooling (the first token's vector, L2-normalised); `query_text` / `passage_text` in `embedder.rs` are where a model's prompts would go. It replaced `multilingual-e5-base` before release: on the calibration corpus it finds a relevant note for 95% of topic queries against 82.5% for e5 at similar precision, and matches fp32 more closely (top-10 overlap 0.90 against 0.83). `tract-onnx` (pure Rust) was tried first and dropped: about 3.1 s per 200-token chunk against about 0.35 s for `ort` on the dev host (2 vCPU x86_64), with e5. Granite takes about 0.6 s per chunk there (onnx-community's int8 export: 1.0 s), above the 200 ms-per-chunk target; on Apple Silicon e5 rebuilt the dev seed's notes in about one second and the app builds and runs with the prebuilt `ort` selects for aarch64-apple-darwin (checked by hand, 2026-11-07, with e5).

**Module layout** (`src-tauri/src/semantic/`): `model_files.rs` (pinned HuggingFace commit and SHA-256 per file; download, verify, remove), `embedder.rs` (`Embedder` trait, `OrtEmbedder`, a deterministic `FakeEmbedder` for tests), `chunker.rs` (heading-aware chunks), `index.rs` (`note_embeddings` rows, vector cache, worker), `fusion.rs` (ranking constants), `real_model_tests.rs` (opt-in, see `docs/development-notes.md`).

**Download.** Only GET requests, to huggingface.co (and its CDN redirects). Each file streams to `<file>.part`, is hashed, and is renamed only if the SHA-256 matches; HTTP, hash and disk errors delete the partial file and show an inline error with Retry. There is no resume. Cancel (and Remove) poll the cancel flag every 100 ms, so they return promptly on a stalled connection. Files live in `<data dir>/models/<model-id>/` (see Storage model); quitting mid-download leaves semantic search off and the stale `.part` is deleted at the next start.

**State and lifecycle.** `SemanticState` is managed Tauri state (an `Arc`). `semantic_search_enabled` (in `app_settings`) turns true only after download and verify succeed. One mutex (`lifecycle`) serialises enable, startup activation, Remove and Rebuild, so a download that finishes during a Remove cannot leave semantic search on without files. At startup: enabled and files present loads the model and catches up on missing or stale notes; enabled with files missing shows an error state with Download, never a crash. Status (`off`, `downloading`, `loading`, `indexing`, `ready`, `error`) is queryable with `get_semantic_status` and pushed as a payload-less `semantic-status` event (`semantic::STATUS_EVENT`); the frontend re-reads the status on each event (`src/lib/semantic-state.svelte.ts`) and on opening Settings.

**Worker and epoch.** Note writes (create, rename, body save, link rewrites, and customer/project renames, which re-queue all notes) enqueue the note id after their transaction commits through the `*_and_queue` helpers in `commands/notes.rs`; the queue dedupes by id. One background worker embeds queued notes one chunk per inference call, releasing the inference lock between chunks, so a query waits for at most about one chunk. Every input (chunk or query) runs alone: the int8 model quantises activations per run, so texts batched together would change each other's vectors. It skips notes whose content hash and model id are unchanged. Restore, Remove and Rebuild bump an in-memory epoch and clear the vector cache; the worker re-checks epoch and hash inside its write transaction and drops stale results, and a missing note (deleted meanwhile) is a skip, not an error. A failed inference is logged and the note skipped. Release builds use `panic = "abort"`, so tokenizer, inference and download paths must not panic: inputs are truncated to 512 tokens before inference.

**Chunking.** Notes split on pulldown-cmark heading events into chunks of about 200 tokens with no overlap. Each chunk is embedded with the note title and heading path as a prefix, but `ChunkText` stores the body only; that is the snippet shown for semantic hits. Token counting is injected (the real tokenizer in the app, a word-count fake in tests).

**Hybrid search.** The vectors are an in-memory cache loaded from `note_embeddings`, searched by brute-force dot product (no `sqlite-vec`, no ANN). A note scores its best chunk. `search_notes` with `hybrid: true` runs the semantic leg only when ready and the query with `#tag` terms removed has at least 4 characters (tag-only queries stay keyword-only); `#tag` terms filter both legs. Both legs are cut to depth 50 and fused with Reciprocal Rank Fusion (k = 60). Granite scores sit in a narrow band that shifts with the query, so the similarity cut is relative (`similarity_floor` in `fusion.rs`): a semantic hit's best-chunk score must reach the median best-chunk score of all indexed notes for that query plus 0.05 (`MARGIN`), and never less than 0.76 (`ABSOLUTE_FLOOR`). The median is taken over every indexed note, before the tag filter and the depth cut. With fewer than 8 indexed notes the median says little, so a fixed 0.8375 (`FIXED_FLOOR`) applies instead. The values were calibrated at note level with the real model on a synthetic Swedish/English corpus plus off-topic queries: no rule tried separates relevant notes from noise cleanly, and this one favours not missing real matches (about 70% of off-topic queries still return a related note; a margin of 0.06 would cut that to 55% but drops relevant notes on the dev seed's sample notes, which sit 0.052–0.14 above the median). `real_model_sample_notes` checks it on those sample notes. Re-calibrate with `real_model_spike` if the model, chunking or prefix changes. Keyword hits past depth 50 are appended in keyword order, so a large `limit` never drops hits that plain search returns. Semantic-only hits come back with `related: true` (`NoteHit`) and their best-chunk snippet; the UI marks them "related".

The notes list and ⌘K search in two phases: keyword results immediately, then `hybrid: true` results after a 250 ms debounce, with stale responses dropped by the existing sequence guard. Pure helpers (merge, stale guard, status labels) are in `src/lib/semantic.ts`, tested in `tests/semantic.test.mjs`. The Search index Rebuild stays FTS-only; the Semantic search section has its own Rebuild.

### Notes links

`#tags` and `[[wikilinks]]` (`[[Title]]`, `[[Title|alias]]`; no heading links or embeds) follow Obsidian's rules, and Rust and the editor each implement them. `src-tauri/src/note_links.rs` walks pulldown-cmark events with byte offsets (skipping code, links, autolinks and raw HTML, and joining the text runs pulldown-cmark splits at `[`, `]` and table pipes) and returns tags and wikilinks with ranges. The one extractor feeds indexing (`note_refs`), `render_markdown` and rename rewriting. `src/lib/note-links.ts` is its TypeScript twin for the editor (scanning, `#`/`[[` triggers, slugs); both are tested against the shared case table `tests/fixtures/note-links-cases.json`, so change a rule in both places and add a case there.

Tags match case-insensitively and nest by prefix: `#acme` matches `#acme` and `#acme/website`. Customers and projects get tags from their names (lowercase, runs of characters not allowed in tags become `-`: customer `#<cslug>`, project `#<cslug>/<pslug>`); issues and todos use their ids (`#issue/<id>`, `#todo/<id>`, inserted by autocomplete and drawn as chips with the live title; an unknown id stays a plain tag). Entities are resolved at read time, so deleting one just leaves a plain tag, and deleting a note leaves its incoming links unresolved. To keep a slug owned by one entity, creating or renaming a customer (or a project within its customer) whose slug equals another's is refused ("Name clashes with tag #…"); a name whose slug is empty or all digits gets no tag, and so does a customer whose slug is `issue` or `todo` (the chip namespaces), along with its projects. Data that already collides gives the slug to the lowest id.

Renames rewrite bodies in the same transaction: a note rename retargets `[[Old]]`/`[[Old|alias]]` (case-insensitive) in other notes, and a customer or project rename rewrites its slug tags (nested ones too), only when it owned the old slug and the new slug is valid. Rewrites edit at the extractor's byte ranges, so code is untouched, re-index FTS and refs, and do not bump `UpdatedAt`. `rename_note` returns the ids of the notes it rewrote; the notes page then drops their cached editor state and queued saves and reloads the open note if affected (undo history of rewritten notes resets), so no stale body can restore the old title. Customer and project renames happen in Settings, where the notes page is unmounted.

`search_notes` treats each `#term` in the query as a tag filter (AND, prefix-nested) and the rest as the text search; a tag-only query lists every tagged note (the note list and ⌘K both). In the editor, resolution data (note titles, entity names, todo status) sits in a CodeMirror `StateField` the page fills from `get_link_targets` and `list_tag_suggestions` (`src/lib/editor/links.ts`); ⌘-click follows (a wikilink opens its note, creating it first through `create_note_titled` when unresolved; a tag or chip filters the list). The read view follows on a plain click through `data-tag` / `data-note` attributes on the rendered anchors, checked before the external-URL path. A collapsible "Linked mentions" strip under the note lists the notes linking to it (`list_backlinks`, one-line snippet, no unlinked mentions).

### Notes folders

Rules and data semantics (name validation, suffixing on clashes, delete-to-parent, restore) are in `docs/data-and-backup.md`. Rust owns them in `src-tauri/src/commands/folders.rs`; the frontend only builds the view.

Pure tree logic lives in `src/lib/folders.ts` (tested in `tests/folders.test.mjs`): building the tree from the flat `list_folders` and note summaries, ordering, flattening to the visible rows, folder paths, descendant sets, valid move targets, the keyboard action table and drag-and-drop helpers. `src/lib/components/NoteTree.svelte` renders one flat keyed `{#each}` over the visible rows (keys `f:<id>`/`n:<id>`, indentation per row, so `animate:flip` works across levels), with ARIA `tree` roles and keyboard navigation (↑/↓, →/←, Enter/Space, F2 to rename). Drag and drop uses pointer events with a 5 px threshold, like the todos board, not HTML5 DnD; drop targets are found with `elementFromPoint` and `[data-drop-folder]`. A collapsed folder under the pointer auto-expands after 600 ms (reverted if the drag is cancelled), the list auto-scrolls near its edges, Esc cancels, and dragging is off while searching. `src/lib/components/MoveToDialog.svelte` is the keyboard alternative: a folder picker (every valid destination by full path, excluding the item's own subtree) opened from the editor header or from a row's context menu. The tree's context menu (`src/lib/components/ContextMenu.svelte`, one instance rendered by `NoteTree`, positioned by the pure `placeMenu` in `src/lib/context-menu.ts`) opens on right-click, Ctrl+click, or Shift+F10 / the ContextMenu key (anchored under the focused row), with folder, note and root ("Notes") item sets; the native WebView menu is suppressed on tree rows only. The hover buttons stay. Note delete from the menu uses an inline confirm on the row, like folders. The notes page owns the current folder (the last clicked folder or the open note's folder): ⌘N, `+` and `create_note_titled` create there, and the list reloads from the backend after every move or delete.

The expanded set is persisted through `get_expanded_folders`/`set_expanded_folders` (debounced; ancestors of the open note always expand).

## Storage model

TimeTracker keeps runtime files in different locations by build type:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/timetracker.db
.timetracker-dev/logs/
.timetracker-dev/backups/
.timetracker-dev/models/
```

Non-debug builds:

```text
~/.timetracker/timetracker.db
~/.timetracker/logs/
~/.timetracker/backups/
~/.timetracker/models/
```

The schema is created on startup if it does not exist. The main tables are:

- `customers`
- `projects`
- `issues`
- `time_entries`
- `todos`
- `travel`
- `notes`
- `folders` (note folders, see Notes folders)
- `note_refs` (derived link index, see Notes links; not in backups)
- `note_embeddings` (derived semantic index, see Semantic search; not in backups)

Foreign keys are enabled, and cascading deletes are used to keep related records consistent.

## Runtime behavior with weight

### SPA mode is required

The root layout disables SSR because Tauri APIs only exist inside the WebView at runtime. Frontend code is therefore written as a client-only application.

### Rust owns persistence and file access

The frontend does not talk to SQLite directly and does not read or write backup files on its own. File-system work stays in Rust commands, which keeps native access behind a single boundary.

### Development-only seed data is gated by debug builds

`dev_seed` is compiled only with `debug_assertions`, and the seed command is registered only in debug builds. Production bundles therefore do not ship the seed workflow.

### Logging is file-based

The backend initializes daily-rotating logs under repo-local `.timetracker-dev/logs/` in debug builds and `~/.timetracker/logs/` in non-debug builds. It keeps the logging guard alive in managed Tauri state so writes do not stop early.

### Automatic startup backups

On each application startup, TimeTracker creates a backup if the retention setting is greater than zero. The backup is written to `backups/YYYY-MM-DD/backup.json` beside the active database, overwriting any existing backup for today. After backup creation, old backups are automatically trimmed to keep only the most recent backups based on the retention setting (default: 3). Setting retention to 0 disables automatic startup backups entirely. Backup creation, trimming, and listing logic stays in Rust; the frontend only displays the available backups and retention setting.
