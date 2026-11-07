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

## 2. Due reminders on todos

## 3. Encrypted backups

## Done

- Notes semantic search — opt-in in Settings › Preferences (downloads a ~300 MB multilingual e5 model on click, then indexes your notes in the background); the notes list and ⌘K merge meaning-based hits into the keyword results (Reciprocal Rank Fusion), marked "related" when the note has no keyword match, so a Swedish query finds English notes and vice versa; search stays keyword-only until the model is ready; the index is derived, not in backups, and rebuilt after restore
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
