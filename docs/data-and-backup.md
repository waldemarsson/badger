# Data and backup

## Data model

TimeTracker stores its working data in a local SQLite database. The schema is intentionally small and centered on the time-entry workflow:

- `customers` — top-level client records, including a single default marker
- `projects` — customer-owned projects, also with a default marker
- `issues` — project-owned work items, identified by title
- `time_entries` — dated hour records linked to issues
- `todos` — standalone kanban todos (`Status`: not started / in progress / done, ordered by `Position`) with an optional link to an issue
- `travel` — standalone business trips: date, from/to place, and optional km and amount (kr), stored as NULL when not entered
- `notes` — markdown notes: unique `Title` (case-insensitive, via a `COLLATE NOCASE` unique index), `Body`, `Pinned`, `CreatedAt`/`UpdatedAt`, and an optional `FolderId`
- `folders` — nestable note folders (`ParentId`, root = NULL, `Name`, `Position`). `Position` is unused: the tree sorts folders by name in the frontend. Names are trimmed, 1–100 characters, may not contain `/`, and are unique among siblings ignoring case (Rust `to_lowercase()`, like note titles)
- `note_refs` — link index: one row per note and distinct `#tag` or `[[wikilink]]` target (`Kind` `tag`/`link`, `Target` lowercased, tags without the `#`); `NoteId` cascades on note delete. Derived data like `notes_fts`: Rust writes it in the same transaction as every body write, it is not in backups, and it is rebuilt by restore, by the same Settings › Preferences › Search index › Rebuild button (which rebuilds both indexes), and at startup when the `note_refs_version` value in `app_settings` differs from the code's `NOTE_REFS_VERSION` (first start after upgrade, or after the extraction rules change; bump the constant when they do). The startup rebuild is version-only, so a drifted index is repaired by Rebuild, not at startup. Entities are resolved at read time, so deleting a customer, project, todo or note needs no index cleanup
- `notes_fts` — FTS5 search index (`trigram` tokenizer) over note titles and plain-text bodies; rowid = note id. Derived data: Rust writes it in the same transaction as every note write, it is not in backups, and it is rebuilt by restore, by Settings › Preferences › Search index › Rebuild, and at startup when its row count differs from `notes` (`backfill_notes_fts`: the first start after an upgrade, or notes written by an older build)

Deletes cascade downward from customers to projects, issues, and time entries, so the hierarchy stays consistent without manual cleanup across tables. Todos are never deleted by a cascade: deleting a linked issue only clears the todo's link. Likewise, deleting a folder never deletes notes: `delete_folder` first moves its notes and child folders to the folder's parent (the root for a top-level folder) in one transaction, then deletes it. `ON DELETE SET NULL` on `FolderId` and `ParentId` remains only as a backstop.

Folder moves and deletes never fail on a name clash: the moved folder gets the first free ` 2`, ` 3`, … suffix among its new siblings (`Work` becomes `Work 2`). Moving a folder into itself or a descendant is refused, and moving to the current parent is a no-op. Notes never clash because titles are globally unique. `move_note` does not bump `UpdatedAt`: moving is not an edit and would reorder the list. Commands that take a folder id (`create_note`, `create_note_titled`, `move_note`, `move_folder`, `create_folder`) answer "Folder not found" for a missing one.

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
.timetracker-dev/timetracker.db
```

Installed and other non-debug builds:

```text
~/.timetracker/timetracker.db
```

## Automatic startup backups

TimeTracker automatically creates a backup on each application startup. This behavior is controlled by the startup backup retention setting, which defaults to `3`:

- **Retention > 0**: A backup is written to `backups/YYYY-MM-DD/backup.json` beside the active database, overwriting any existing backup for today. After creation, old backups are automatically trimmed to keep only the most recent `N` backups.
- **Retention = 0**: Automatic startup backups are disabled entirely.

The retention setting can be adjusted in the Settings > Backup & Restore tab.

### Startup backup locations

The backup root directory is determined by the database location:

Development (`npm run dev` / debug builds):

```text
.timetracker-dev/backups/YYYY-MM-DD/backup.json
```

Installed and other non-debug builds:

```text
~/.timetracker/backups/YYYY-MM-DD/backup.json
```

Startup backup creation, trimming, and listing are handled entirely in Rust and do not involve the frontend beyond displaying the available backups and retention setting in the UI.

## Backup format

Backup export writes a JSON document containing eight arrays:

- `customers`
- `projects`
- `issues`
- `time_entries`
- `todos` (optional on import, so backups made before todos existed still restore)
- `travel` (optional on import, so backups made before travel existed still restore)
- `folders` (optional on import, so backups made before notes existed still restore)
- `notes` (optional on import, same reason)

The backup commands preserve record identifiers so a restore can rebuild the same relationships between records.

## Restore

Restore always overwrites. In one transaction it:

- deletes all customers, which cascades to projects, issues, and time entries
- clears todos, travel, notes and folders explicitly (the cascade does not reach them)
- re-inserts the full backup, folders before notes
- rebuilds the notes search and link indexes (`notes_fts`, `note_refs`) from the restored notes, in the same transaction

Data not in the backup is lost, so restoring a backup made before notes existed leaves zero notes. If the backup file is invalid, nothing is changed.

Folders and notes are inserted with `PRAGMA defer_foreign_keys = ON` inside the transaction, so a child folder may appear before its parent (a lower id) in the file. Foreign keys are checked at commit. A dangling folder reference (for example in a hand-edited file) fails with "Backup is invalid: a folder or note refers to a missing folder", and two notes whose titles differ only in case fail with "Backup is invalid: duplicate note title". Both roll back everything. Before inserting, restore also rejects folders that form a cycle (including a folder that is its own parent) with "Backup is invalid: folders form a cycle", checked in Rust code rather than by the database, and renames duplicate sibling folder names (case-insensitive) to the first free ` 2`, ` 3`, … suffix in id order, so such a backup still restores. Names over 100 characters or containing `/` are restored as they are (tolerance for existing data, as with note titles); a `/` in a name makes the displayed folder path ambiguous. Folder ids are preserved, so a stored expanded-folders set still applies where the folders match.

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
