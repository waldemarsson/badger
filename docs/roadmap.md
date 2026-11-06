# Roadmap

Planned work, in priority order.

## 1. Due reminders on todos

## Done

- Today — start page at `/` with todos needing attention, in-progress todos, today's logged time and a Mon–Sun week strip with progress against a daily hours target (Settings › Preferences); the Timesheet moved to `/timesheet` and gained a Single Day filter
- Command palette — ⌘K (configurable in Settings) for navigation, New todo and a keyboard-only Log time flow (project → issue → hours/date → notes) that remembers the last project and issue
- Restore overwrites only — merge mode removed; restore always replaces all data with the backup
- SvelteKit 3 — Kit 3, Vite 8, vite-plugin-svelte 7, adapter-static 4; config moved into `vite.config.ts`, `$lib` → `#lib`, `$app/stores` → `$app/state`
- Travel — `/travel` page (date, from/to, optional km and kr; add, edit, repeat, delete) and the month's travel plus a copy-as-text export on the monthly report
- Dark mode — System/Light/Dark in Settings › Preferences, persisted in `app_settings`, no flash on startup
- Edit customers and projects — inline rename with name validation, and a grouped-card layout in Settings
