# Roadmap

Planned work, in priority order.

## 1. Notes ("micro obsidian")

Markdown notes stored in SQLite, written in Obsidian-compatible markdown so notes export as plain `.md`. Built in chunks, each its own branch and PR:

1. **Notes core** — done, see Done below
2. **Read view and copy as HTML** — done, see Done below
3. **Search** — done, see Done below
4. **Linking** — done, see Done below
5. **Folders** — done, see Done below
6. **Semantic search** — done, see Done below
7. **Chat that edits notes** — deferred; local model with tool calls proposing diffs to accept or reject

## 2. Encrypted backups

## 3. Release pipeline hardening

`release.yml` publishes a release on every push to `main` without running the gates. Add a gate job (`cargo test`, `cargo clippy --all -- -D warnings`, `cargo fmt --check`, `npm run check`, `npm run lint`, `node --test tests/*.mjs`) that the release job `needs:`; pin third-party actions (`actions/*`, `dtolnay/rust-toolchain`, `swatinem/rust-cache`, `tauri-apps/tauri-action`) to commit SHAs; default the workflow to `permissions: contents: read` and grant `contents: write` only to the release job.

## 4. Dependency monitoring

Dependabot for npm, cargo and GitHub Actions (grouped weekly updates), plus a `cargo deny check advisories` (or `cargo audit`) step and `npm audit --omit=dev` in the gate job.

## 5. Major dependency upgrades

Each on its own branch: `sqlx` 0.9, `sha2` 0.11, `tokenizers` 0.23 first; then `typescript` 7, `prettier-plugin-svelte` 4, `@types/node` 26 and `tailwind-merge` 3; Tailwind 4 last (touches all styling and `app.css`).

## Done

- Badger — the app is now called Badger (bundle id `io.github.narvallabs.badger`, data in `~/.badger/`, dev data in `.badger-dev/`) with a badger app icon generated from `src-tauri/icons/icon-source.png`
- Reminders — Slack-style one-off or recurring pings (daily, weekdays, weekly on chosen days, monthly) at an exact local time, created by typing `remind call Anna in 20 min` (English or Swedish) in ⌘K with a live preview, from the `/reminders` page, from a todo, or from a note's context menu; due ones become native macOS notifications (batched when several fire at once, skipped while the window is focused) and actionable in-app toasts with Done and Snooze, with the fired count on the dock badge; managed on `/reminders` (Overdue / Today / Tomorrow / Later / Done) and a Today card; stored in `reminders` and included in backup/restore. Fires only while the app runs; the missed ones fire as a catch-up on launch
- Notes semantic search — opt-in in Settings › Preferences (downloads a ~350 MB multilingual Granite embedding model on click, then indexes your notes in the background); the notes list and ⌘K merge meaning-based hits into the keyword results (Reciprocal Rank Fusion), marked "related" when the note has no keyword match, so a Swedish query finds English notes and vice versa; search stays keyword-only until the model is ready; the index is derived, not in backups, and rebuilt after restore
- Notes folders — the notes list is a folder tree (folders first A–Z, then notes pinned first, then most recently updated); create, rename (pencil or F2) and delete folders inline, deleting moves the contents to the parent; move notes and folders by pointer drag and drop (collapsed folders auto-expand under the pointer, Esc cancels) or with a keyboard "Move to…" picker; right-click (Ctrl+click, Shift+F10) a row for a context menu with its actions; ⌘N, `+` and followed unresolved wikilinks create in the current folder; search hits show their folder path; arrow-key navigation (↑/↓, ←/→, Enter/Space, F2), and expanded folders are remembered
- Notes linking — Obsidian-style `#tags` and `[[wikilinks]]` (`[[Title]]`, `[[Title|alias]]`) with `#` and `[[` autocomplete; `#acme`, `#acme/website`, `#issue/<id>` and `#todo/<id>` link to customers, projects, issues and todos, the last two shown as chips with the live title (todo chips coloured by status); ⌘-click in the editor or a plain click in the read view follows a link, a tag filters the note list, and following an unresolved wikilink creates the note; `#term` in the note search or ⌘K filters by tag (nested: `#acme` also matches `#acme/website`); a "Linked mentions" strip lists backlinks; renaming a note, customer or project rewrites matching links and tags in all notes, and deleting leaves a plain tag or an unresolved link
- Notes search — SQLite FTS5 (`trigram`: case-insensitive substrings, so `rapport` finds `tidrapport`; å/ä/ö stay distinct) over titles and plain-text bodies; the note list ranks by relevance (title hits first) with highlighted titles and snippets, ⌘K lists the top 5 notes (from 3 typed characters), and Settings › Preferences shows the index size with a Rebuild button
- Notes read view — ⌘E toggles a read view rendered in Rust (`pulldown-cmark`; raw HTML escaped, only http/https/mailto links, images never load) with highlighted code and links that open in the browser; "Copy as HTML" puts rendered HTML and the markdown source on the clipboard
- Notes core — `/notes` page with a filtered note list and a CodeMirror 6 editor with Obsidian-style live preview (headings, emphasis, code, quotes, lists, task checkboxes, tables), autosave, pinning, ⌘N and a palette "New note" action; `notes` and nestable `folders` tables included in backup/restore; a quit/close guard flushes unsaved edits
- Today — start page at `/` with todos needing attention, in-progress todos, today's logged time and a Mon–Sun week strip with progress against a daily hours target (Settings › Preferences); the Timesheet moved to `/timesheet` and gained a Single Day filter
- Command palette — ⌘K (configurable in Settings) for navigation, New todo and a keyboard-only Log time flow (project → issue → hours/date → notes) that remembers the last project and issue
- Restore overwrites only — merge mode removed; restore always replaces all data with the backup
- SvelteKit 3 — Kit 3, Vite 8, vite-plugin-svelte 7, adapter-static 4; config moved into `vite.config.ts`, `$lib` → `#lib`, `$app/stores` → `$app/state`
- Travel — `/travel` page (date, from/to, optional km and kr; add, edit, repeat, delete) and the month's travel plus a copy-as-text export on the monthly report
- Dark mode — System/Light/Dark in Settings › Preferences, persisted in `app_settings`, no flash on startup
- Edit customers and projects — inline rename with name validation, and a grouped-card layout in Settings
