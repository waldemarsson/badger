# Roadmap

Planned work, in priority order.

## 1. Migrate to SvelteKit 3

Upgrade `@sveltejs/kit` 2 → 3. Its peer dependencies require Vite 8, `@sveltejs/vite-plugin-svelte` 7, `@sveltejs/adapter-static` 4, and Svelte ^5.57.1. Include the `$app/stores` → `$app/state` migration.

## 2. Rewrite restore

Rewrite backup restore, possibly removing merge mode for simplicity.

Context: merge mode uses `INSERT OR REPLACE`, which deletes and re-inserts each issue from the backup. That cascades to local time entries on those issues and clears links on local todos.

## 3. Today / overview / start page

## 4. Due reminders on todos

## 5. Quick add / command palette

Quick add for todos and time entries.

## Done

- Travel — `/travel` page (date, from/to, optional km and kr; add, edit, repeat, delete) and the month's travel plus a copy-as-text export on the monthly report
- Dark mode — System/Light/Dark in Settings › Preferences, persisted in `app_settings`, no flash on startup
- Edit customers and projects — inline rename with name validation, and a grouped-card layout in Settings
