# Roadmap

Planned work, in priority order.

## 1. Rewrite restore

Rewrite backup restore, possibly removing merge mode for simplicity.

Context: merge mode uses `INSERT OR REPLACE`, which deletes and re-inserts each issue from the backup. That cascades to local time entries on those issues and clears links on local todos.

## 2. Today / overview / start page

## 3. Due reminders on todos

## 4. Quick add / command palette

Quick add for todos and time entries.

## Done

- SvelteKit 3 — Kit 3, Vite 8, vite-plugin-svelte 7, adapter-static 4; config moved into `vite.config.ts`, `$lib` → `#lib`, `$app/stores` → `$app/state`
- Travel — `/travel` page (date, from/to, optional km and kr; add, edit, repeat, delete) and the month's travel plus a copy-as-text export on the monthly report
- Dark mode — System/Light/Dark in Settings › Preferences, persisted in `app_settings`, no flash on startup
- Edit customers and projects — inline rename with name validation, and a grouped-card layout in Settings
