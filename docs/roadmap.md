# Roadmap

Planned work, in priority order.

## 1. Agent instructions

Instructions for coding agents covering Tauri, Svelte, UI/UX, and similar.

## 2. Edit customers and projects

## 3. Dark mode

## 4. Travel

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

## 5. Rewrite restore

Rewrite backup restore, possibly removing merge mode for simplicity.

Context: merge mode uses `INSERT OR REPLACE`, which deletes and re-inserts each issue from the backup. That cascades to local time entries on those issues and clears links on local todos.

## 6. Today / overview / start page

## 7. Due reminders on todos

## 8. Quick add / command palette

Quick add for todos and time entries.
