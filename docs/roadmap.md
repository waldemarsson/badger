# Roadmap

Planned work, in priority order.

## 1. Dark mode

## 2. Travel

### Page to manage travel

- Insert travel: date, from/to, number of km, amount in kr
- Delete and edit

### Monthly

The monthly report should include total hours per customer and project, and the month's travel.

Example:

```text
October

OP
Frånvaro: 50 h

Roxtec
MyRoxtec: 40 h
PublicWeb: 20 h

Total: 110 h

Travel:

5/10 Malmö - Karlskrona
500 km - 1300 kr

10/10 Malmö - Karlskrona
500 km - 1300 kr
```

## 3. Migrate to SvelteKit 3

Upgrade `@sveltejs/kit` 2 → 3. Its peer dependencies require Vite 8, `@sveltejs/vite-plugin-svelte` 7, `@sveltejs/adapter-static` 4, and Svelte ^5.57.1. Include the `$app/stores` → `$app/state` migration.

## 4. Rewrite restore

Rewrite backup restore, possibly removing merge mode for simplicity.

Context: merge mode uses `INSERT OR REPLACE`, which deletes and re-inserts each issue from the backup. That cascades to local time entries on those issues and clears links on local todos.

## 5. Today / overview / start page

## 6. Due reminders on todos

## 7. Quick add / command palette

Quick add for todos and time entries.

## Done

- Edit customers and projects — inline rename with name validation, and a grouped-card layout in Settings
