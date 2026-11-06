# Roadmap

Planned work, in priority order.

## 1. Notes ("micro obsidian")

Markdown notes stored in SQLite, written in Obsidian-compatible markdown so notes export as plain `.md`. Built in chunks, each its own branch and PR:

1. **Notes core** — `notes` and nestable `folders` tables (folders UI comes later), backup/restore, `/notes` page with a filtered note list on the left and a CodeMirror 6 editor with Obsidian-style live preview (incl. tables, task checkboxes) on the right, autosave, pinning, ⌘N and a palette "New note" action
2. **Read view and copy as HTML** — markdown rendered in Rust (`pulldown-cmark`), ⌘E toggles a read view, copy as HTML (`text/html` + plain text) from the editor header
3. **Search** — SQLite FTS5 over title and body with highlighted snippets, used by the note list and by ⌘K
4. **Linking** — `#tags` with `#` autocomplete that also suggests customers (`#acme`), projects (`#acme/website`), issues (`#issue/123`) and todos (`#todo/42`, shown as chips); `[[wikilinks]]` between notes with backlinks; renaming a customer, project or note rewrites matching tags and links; deleting an item leaves its tag as a plain tag
5. **Folders** — folder tree, move, drag and drop; deleting a folder moves its contents to the parent
6. **Semantic search** — embedding model embedded in the app (downloaded on first use, configured in Settings), hybrid keyword and semantic results
7. **Chat that edits notes** — deferred; local model with tool calls proposing diffs to accept or reject

## 2. Due reminders on todos

## Done

- Today — start page at `/` with todos needing attention, in-progress todos, today's logged time and a Mon–Sun week strip with progress against a daily hours target (Settings › Preferences); the Timesheet moved to `/timesheet` and gained a Single Day filter
- Command palette — ⌘K (configurable in Settings) for navigation, New todo and a keyboard-only Log time flow (project → issue → hours/date → notes) that remembers the last project and issue
- Restore overwrites only — merge mode removed; restore always replaces all data with the backup
- SvelteKit 3 — Kit 3, Vite 8, vite-plugin-svelte 7, adapter-static 4; config moved into `vite.config.ts`, `$lib` → `#lib`, `$app/stores` → `$app/state`
- Travel — `/travel` page (date, from/to, optional km and kr; add, edit, repeat, delete) and the month's travel plus a copy-as-text export on the monthly report
- Dark mode — System/Light/Dark in Settings › Preferences, persisted in `app_settings`, no flash on startup
- Edit customers and projects — inline rename with name validation, and a grouped-card layout in Settings
