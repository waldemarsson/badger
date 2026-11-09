# Data and backup

## Data model

Badger stores its working data in a local SQLite database. The schema is intentionally small and centered on the time-entry workflow:

- `customers` — top-level client records, including a single default marker
- `projects` — customer-owned projects, also with a default marker
- `issues` — project-owned work items, identified by title
- `time_entries` — dated hour records linked to issues
- `todos` — standalone kanban todos (`Status`: not started / in progress / done, ordered by `Position`) with an optional link to an issue
- `reminders` — Slack-style reminders: `Text` (trimmed, 1–500 characters), `RemindAt` (the current occurrence, naive local time `%Y-%m-%dT%H:%M:%S%.3f`), `TimeOfDay` (`HH:MM` recurrence anchor), `Repeat` (`none`/`daily`/`weekdays`/`weekly`/`monthly`), `Weekdays` (ISO numbers as text, e.g. `1,4` for Mon and Thu), `MonthDay` (1–31, monthly only), `State` (`pending`/`fired`/`done`), and an optional `TodoId` or `NoteId`. Both links are `ON DELETE CASCADE`, so deleting a todo or note deletes its reminders (unlike issue links, which only clear), and a `CHECK` allows at most one link. Created by `SCHEMA_SQL` with no migration. Semantics are in `docs/architecture.md` (Reminders)
- `travel` — standalone business trips: date, from/to place, and optional km and amount (kr), stored as NULL when not entered
- `notes` — markdown notes: unique `Title` (case-insensitive, via a `COLLATE NOCASE` unique index), `Body`, `Pinned`, `CreatedAt`/`UpdatedAt`, an optional `FolderId`, and `Position` (manual order within the folder; added to existing databases by the idempotent `migrate_notes_position`, see Manual order below)
- `folders` — nestable note folders (`ParentId`, root = NULL, `Name`, `Position`, manual order among siblings; see Manual order below). Names are trimmed, 1–100 characters, may not contain `/`, and are unique among siblings ignoring case (Rust `to_lowercase()`, like note titles)
- `scratchpad` — the scratchpad's text: a single row (`id` is `CHECK (id = 1)`), `Body` and `UpdatedAt`, created on the first save (no row means an empty pad). Not a note, so it is in no search or link index and has no title rule. An identical body is a no-op that keeps `UpdatedAt`. Created by `SCHEMA_SQL` with no migration. Semantics are in `docs/architecture.md` (Scratchpad)
- `note_refs` — link index: one row per note and distinct `#tag` or `[[wikilink]]` target (`Kind` `tag`/`link`, `Target` lowercased, tags without the `#`); `NoteId` cascades on note delete. Derived data like `notes_fts`: Rust writes it in the same transaction as every body write, it is not in backups, and it is rebuilt by restore, by the same Settings › Preferences › Search index › Rebuild button (which rebuilds both indexes), and at startup when the `note_refs_version` value in `app_settings` differs from the code's `NOTE_REFS_VERSION` (first start after upgrade, or after the extraction rules change; bump the constant when they do). The startup rebuild is version-only, so a drifted index is repaired by Rebuild, not at startup. Entities are resolved at read time, so deleting a customer, project, todo or note needs no index cleanup
- `notes_fts` — FTS5 search index (`trigram` tokenizer) over note titles and plain-text bodies; rowid = note id. Derived data: Rust writes it in the same transaction as every note write, it is not in backups, and it is rebuilt by restore, by Settings › Preferences › Search index › Rebuild, and at startup when its row count differs from `notes` (`backfill_notes_fts`: the first start after an upgrade, or notes written by an older build)
- `note_embeddings` — semantic search index: one row per embedded note chunk (`NoteId` cascades on note delete, `ChunkIndex`, `ContentHash`, `ModelId`, `ChunkText` = the chunk body without its title/heading prefix, `Vector` = L2-normalised little-endian f32 BLOB). Derived data like `notes_fts`, and a deliberate exception to the "every table is backed up" rule: it is not in backups, and restore re-embeds every restored note in the background. It is empty until semantic search is enabled; rows whose `ModelId` differs from the current model are re-embedded. Settings › Preferences › Semantic search › Rebuild re-embeds everything; the Search index Rebuild does not touch it

Deletes cascade downward from customers to projects, issues, and time entries, so the hierarchy stays consistent without manual cleanup across tables. Todos are never deleted by a cascade: deleting a linked issue only clears the todo's link. Likewise, deleting a folder never deletes notes: `delete_folder` first moves its notes and child folders to the folder's parent (the root for a top-level folder) in one transaction, then deletes it. `ON DELETE SET NULL` on `FolderId` and `ParentId` remains only as a backstop.

Folder moves and deletes never fail on a name clash: the moved folder gets the first free ` 2`, ` 3`, … suffix among its new siblings (`Work` becomes `Work 2`). Moving a folder into itself or a descendant is refused, and moving to the current parent without an index is a no-op (with an index it is a reorder). Notes never clash because titles are globally unique. `move_note` does not bump `UpdatedAt`: moving is not an edit. Neither does pin/unpin, and no edit changes `Position`. Commands that take a folder id (`create_note`, `create_note_titled`, `move_note`, `move_folder`, `create_folder`) answer "Folder not found" for a missing one.

### Manual order

The notes tree order is the user's, stored in `Position`; there is no sort toggle. Rust returns it in tree order and the frontend preserves it (no sorting by name or date).

- Notes: `ORDER BY Pinned DESC, Position, id`. One `Position` numbering per folder covers both groups; pinned notes always show above unpinned ones, and order is manual within each group. Folders: `ORDER BY Position, id` per parent, and folders always sit above notes inside a parent, never interleaved. Gaps are tolerated; writes renumber siblings densely 0..n in one transaction.
- Placement: a new note goes to the top of its folder's unpinned group, a new folder to the end of its parent. Pinning puts a note at the bottom of the pinned group, unpinning at the top of the unpinned group (it stays at the boundary). Editing a note or renaming a folder never moves it. `delete_folder` appends the children (folders and notes) to the end of the parent's lists, keeping their relative order.
- `move_note { id, folderId, index? }` and `move_folder { id, parentId, index? }` take an optional `index`. `None` is the default into-placement: top of the note's own pin group, or the end of the parent's folders. `Some(i)` is the position among the target siblings (for notes, within the note's own pin group, so a note cannot be placed into the other group by index), clamped to range. The same parent with an index is a pure reorder. The cycle check and name suffixing are unchanged.
- Upgrade backfill (`migrate_notes_position`, one transaction, no-op once the column exists): notes per folder in the old order (`Pinned DESC, UpdatedAt DESC, id`), folders per parent A–Z approximated in Rust (`compare_folder_names` in `db.rs`: case-insensitive, numbers by value, å < ä < ö after z). The folder order is an approximation of the old frontend sort; the user fixes oddities by dragging. The helper (`backfill_positions`) is shared with restore.

Which folders are expanded in the notes tree is stored in `app_settings` under `notes_expanded_folders` (a JSON array of ids; `get_expanded_folders`/`set_expanded_folders`). Unknown ids are dropped when read or written, `delete_folder` removes its id, and like other settings it is not part of backups.

Note titles are trimmed, 1–200 characters, unique ignoring case (compared in Rust with `to_lowercase()`, so `Åsa` equals `åsa`; no Unicode normalization, so NFC and NFD forms count as different), and may not contain `[`, `]`, `|`, `#` or `` ` ``, which would break `[[wikilinks]]` (a backtick could turn a rewritten link into code). Saving a note's title and body are separate commands (`rename_note`, `update_note_body`), so a title conflict never blocks saving the body. Saving an identical body is a no-op that leaves `UpdatedAt` alone, and pinning does not bump it.

## Database behavior

The database is initialized on startup with:

- WAL journal mode
- foreign keys enabled
- idempotent schema creation

The database location depends on the build type:

Development (`npm run dev` / debug builds):

```text
.badger-dev/badger.db
```

Installed and other non-debug builds:

```text
~/.badger/badger.db
```

## Semantic search model files

Enabling semantic search downloads the embedding model (about 350 MB) into a `models/<model-id>/` directory beside the database:

```text
.badger-dev/models/granite-embedding-311m-multilingual-r2-int8/    (debug builds)
~/.badger/models/granite-embedding-311m-multilingual-r2-int8/      (non-debug builds)
```

The files are not part of backups. The `semantic_search_enabled` setting in `app_settings` is `true` only after the download is verified; like other settings it is not in backups either, so a restored database keeps this machine's value. Downloads stream to `<file>.part` and are renamed only after the SHA-256 matches; stale `.part` files are deleted at startup. Remove model deletes the files and all vectors and turns the setting off. If the setting is on but the files are gone, the Settings section shows an error with a Download button.

Downgrading to a build without semantic search leaves the `note_embeddings` table and the model files behind; both are harmless and can be deleted by hand.

## Automatic startup backups

Badger automatically creates a backup on each application startup, and again whenever the local date changes while the app stays open (checked every 10 minutes; a failed backup is retried at the next check). This behavior is controlled by the startup backup retention setting, which defaults to `3`:

- **Retention > 0**: A backup is written to `backups/YYYY-MM-DD/backup.json` beside the active database, overwriting any existing backup for today. After creation, old backups are automatically trimmed to keep only the most recent `N` backups.
- **Retention = 0**: Automatic startup backups are disabled entirely.

The retention setting can be adjusted in the Settings > Backup & Restore tab.

### Startup backup locations

The backup root directory is determined by the database location:

Development (`npm run dev` / debug builds):

```text
.badger-dev/backups/YYYY-MM-DD/backup.json
```

Installed and other non-debug builds:

```text
~/.badger/backups/YYYY-MM-DD/backup.json
```

Startup backup creation, trimming, and listing are handled entirely in Rust and do not involve the frontend beyond displaying the available backups and retention setting in the UI.

## Backup format

Backup export writes a JSON document containing nine arrays and the scratchpad:

- `customers`
- `projects`
- `issues`
- `time_entries`
- `todos` (optional on import, so backups made before todos existed still restore)
- `travel` (optional on import, so backups made before travel existed still restore)
- `folders` (optional on import, so backups made before notes existed still restore)
- `notes` (optional on import, same reason)
- `reminders` (optional on import, so backups made before reminders existed still restore)
- `scratchpad` (an object with `body` and `updated_at`, or `null` when never written; optional on import, so backups made before the scratchpad existed still restore)

The backup commands preserve record identifiers so a restore can rebuild the same relationships between records.

## Restore

Restore always overwrites. In one transaction it:

- deletes all customers, which cascades to projects, issues, and time entries
- clears reminders, todos, travel, notes, folders and the scratchpad explicitly (the cascade does not reach them)
- re-inserts the full backup, folders before notes, and reminders last (after todos and notes, so their links resolve); backups from before manual sorting get the old tree order backfilled (see below)
- re-inserts the pad when the backup has one
- rebuilds the notes search and link indexes (`notes_fts`, `note_refs`) from the restored notes, in the same transaction
- after the transaction commits, drops in-flight semantic work and queues every restored note for re-embedding (the delete cascaded away the old vectors; `note_embeddings` is not in the backup)

Backups carry folder and note `Position`. `BackupNote.position` is optional (`#[serde(default)]`), so older files still parse. Restore backfills notes when any note lacks a position, and folders when every restored folder has `Position` 0 (older builds never wrote another value, which also covers old backups with no notes). There is no version marker.

Data not in the backup is lost, so restoring a backup made before notes existed leaves zero notes (one made before reminders existed leaves zero reminders, and one made before the scratchpad existed leaves an empty pad). If the backup file is invalid, nothing is changed.

Folders and notes are inserted with `PRAGMA defer_foreign_keys = ON` inside the transaction, so a child folder may appear before its parent (a lower id) in the file. Foreign keys are checked at commit. A dangling folder reference (for example in a hand-edited file) fails with "Backup is invalid: a folder or note refers to a missing folder", and two notes whose titles differ only in case fail with "Backup is invalid: duplicate note title". Both roll back everything. Before inserting, restore also rejects folders that form a cycle (including a folder that is its own parent) with "Backup is invalid: folders form a cycle", checked in Rust code rather than by the database, and renames duplicate sibling folder names (case-insensitive) to the first free ` 2`, ` 3`, … suffix in id order, so such a backup still restores. Names over 100 characters or containing `/` are restored as they are (tolerance for existing data, as with note titles); a `/` in a name makes the displayed folder path ambiguous. Folder ids are preserved, so a stored expanded-folders set still applies where the folders match.

A reminder whose `TodoId` or `NoteId` is not in the backup is rejected before anything changes with "Backup is invalid: a reminder refers to a missing todo or note" (foreign keys are deferred during restore, so the commit error would otherwise blame folders). After a restore the app emits `reminders-changed` and recomputes the dock badge; restored pending reminders that are already past due fire together as one batch on the next scheduler tick.

There is no merge mode. It was removed because `INSERT OR REPLACE` deletes and re-inserts rows, which cascaded to local time entries and cleared todo links.

## Where backup and restore live in the UI

The active backup and restore workflow is currently exposed in the Settings page, where the frontend:

- opens a save dialog for export
- opens a file picker for import
- previews record counts before restore, including notes
- warns when the backup has no notes but the database does ("This backup has no notes; restoring deletes your N notes.")
- warns that restore replaces all current data

## Error handling

Rust command errors are serialized as strings and surfaced back to the frontend. That means backup and restore failures are presented as user-visible text rather than hidden behind silent retries or background recovery.
