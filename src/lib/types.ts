// src/lib/types.ts
// TypeScript mirrors of Rust structs — field names match serde_json snake_case output.

export interface Customer {
	id: number;
	name: string;
	use_as_default: boolean;
}

export interface Project {
	id: number;
	customer_id: number;
	name: string;
	use_as_default: boolean;
}

export interface Issue {
	id: number;
	project_id: number;
	title: string;
}

/**
 * Fully-joined time entry row returned by list_time_entries, create_time_entry,
 * update_time_entry. Fields match the SQL aliases in TimeEntryRow (time_entries.rs).
 */
export interface TimeEntryRow {
	id: number;
	date: string; // YYYY-MM-DD
	hours: number; // f64 — use formatHours() for display
	notes: string | null;
	issue_id: number;
	issue_title: string;
	project_id: number;
	project_name: string;
	customer_id: number;
	customer_name: string;
}

/** Weekly report — one entry per issue.
 *  `adjacent_days` lists weekday names ("mon".."sun") that fall outside Monday's
 *  calendar month; frontend renders "—" for those cells instead of hours. */
export interface WeeklyReportRow {
	customer_id: number;
	customer_name: string;
	project_id: number;
	project_name: string;
	issue_id: number;
	issue_title: string;
	mon: number;
	tue: number;
	wed: number;
	thu: number;
	fri: number;
	sat: number;
	sun: number;
	total: number;
	adjacent_days: string[]; // e.g. ["mon","tue"] — render "—" for these
}

/** Monthly report — one entry per issue; frontend derives Customer→Project→Issue tree
 *  using $derived by grouping on customer_id then project_id. */
export interface MonthlyReportRow {
	customer_id: number;
	customer_name: string;
	project_id: number;
	project_name: string;
	issue_id: number;
	issue_title: string;
	hours: number;
}

export type TaskRepeat = 'none' | 'daily' | 'weekdays' | 'weekly' | 'monthly';
/** pending → sent (notified; counts as overdue) → dismissed (toast ×, or silently past). */
export type TaskReminderState = 'pending' | 'sent' | 'dismissed';

/** A task reminder: relative to the due (`offset_minutes`, 0 = at due time) or custom (`at`).
 *  Times are naive local `YYYY-MM-DDTHH:MM:SS.mmm`. */
export interface TaskReminder {
	id: number;
	task_id: number;
	offset_minutes: number | null;
	at: string | null;
	state: TaskReminderState;
	snoozed_until: string | null;
	/** Effective time (snooze, else due-relative or custom); null = relative without a due date. */
	fire_at: string | null;
	created_at: string;
}

/** A task (list_tasks and the task commands). Link fields are null when unlinked. */
export interface Task {
	id: number;
	title: string;
	notes: string | null;
	done: boolean;
	completed_at: string | null;
	due_date: string | null; // YYYY-MM-DD
	due_time: string | null; // HH:MM, only with a due date
	repeat: TaskRepeat;
	weekdays: number[]; // ISO 1=Mon … 7=Sun; weekly only
	month_day: number | null;
	issue_id: number | null;
	issue_title: string | null;
	project_id: number | null;
	project_name: string | null;
	customer_id: number | null;
	customer_name: string | null;
	note_id: number | null;
	note_title: string | null;
	/** "Keep off phone": never synced to Apple Reminders (repeating tasks never sync either). */
	mac_only: boolean;
	created_at: string;
	updated_at: string;
	/** By fire_at (null last), then id. */
	reminders: TaskReminder[];
}

/** create_task / update_task `input` (camelCase). Update replaces every field (null clears). */
export interface TaskInput {
	title: string;
	notes?: string | null;
	dueDate?: string | null;
	dueTime?: string | null;
	repeat?: TaskRepeat;
	/** Weekly: empty → the due date's weekday. */
	weekdays?: number[];
	/** Monthly: missing → the due date's day. */
	monthDay?: number | null;
	issueId?: number | null;
	noteId?: number | null;
	/** Create: default false. Update: omitted keeps the current value. */
	macOnly?: boolean;
}

/** Exactly one of `offsetMinutes` (0..40320) or `at` (a future local timestamp). */
export type TaskReminderInput = { offsetMinutes: number; at?: undefined } | { at: string; offsetMinutes?: undefined };

/** Opaque pre-completion snapshot: hand it back to undo_complete_task unchanged. */
export type TaskUndo = { readonly task_id: number } & Record<string, unknown>;

/** complete_task result; `undo` is null when the task was already done. */
export interface TaskCompletion {
	task: Task;
	undo: TaskUndo | null;
}

/** restore_data result. A pre-tasks backup keeps the current tasks and reports the legacy rows it skipped. */
export interface RestoreReport {
	kept_tasks: boolean;
	skipped_todos: number;
	skipped_reminders: number;
}

/** A trip; km and amount (kr) are null when not entered. */
export interface TravelRow {
	id: number;
	date: string; // YYYY-MM-DD
	from_place: string;
	to_place: string;
	km: number | null;
	amount: number | null;
}

/** Command palette preferences (get_palette_settings). Ids may reference deleted rows. */
export interface PaletteSettings {
	shortcut: string; // e.g. "Meta+K" — see src/lib/palette.ts
	last_project_id: number | null;
	last_issue_id: number | null;
}

/** Note folder (list_folders, create_folder, rename_folder, move_folder). Root = null parent. */
export interface Folder {
	id: number;
	parent_id: number | null;
	name: string; // unique among siblings, case-insensitively
	position: number; // manual order among siblings (then id)
}

/** A note's (or the scratchpad's) editor mode. JSON notes are title-only in search and links. */
export type NoteKind = 'markdown' | 'json';

/** Note list row (list_notes, update_note_body, set_note_pinned, move_note). */
export interface NoteSummary {
	id: number;
	folder_id: number | null;
	title: string;
	excerpt: string; // first meaningful body line, markers stripped; '' for an empty body
	pinned: boolean;
	created_at: string; // local YYYY-MM-DDTHH:MM:SS.mmm
	updated_at: string;
	position: number; // manual order within the folder (shown pinned first, then position, then id)
	kind: NoteKind; // JSON notes have an empty excerpt
}

/** search_notes result: a list row plus title/snippet with hits wrapped in \u0002…\u0003. */
export interface NoteHit extends NoteSummary {
	title_marked: string;
	snippet: string; // '' when the body has no highlighted hit
	/** Semantic-only hit (hybrid search): `snippet` is the best-matching passage, unmarked. */
	related: boolean;
}

/** get_semantic_status and the semantic search commands. */
export interface SemanticStatus {
	state: 'off' | 'downloading' | 'loading' | 'indexing' | 'ready' | 'error';
	enabled: boolean;
	downloaded_bytes: number;
	download_bytes: number;
	indexing_done: number;
	indexing_total: number;
	error: string | null;
	model_name: string;
	model_bytes: number;
	indexed_notes: number;
	notes: number;
	index_bytes: number;
}

/** Reminders access as EventKit reports it (`unsupported` off macOS). */
export type AppleSyncAccess = 'not_determined' | 'granted' | 'denied' | 'restricted' | 'unsupported';

/** A phone item the last sync pass left alone, and why ("needs a date", "needs a title"). */
export interface AppleSyncSkipped {
	title: string;
	list: string;
	reason: string;
}

/** get_apple_sync_status and the other Apple Reminders sync commands. */
export interface AppleSyncStatus {
	available: boolean;
	enabled: boolean;
	access: AppleSyncAccess;
	last_sync_at: string | null;
	last_error: string | null;
	/** Why the mass-delete guard holds sync, while it does. */
	paused: 'access' | 'list_missing' | 'empty_fetch' | null;
	skipped: AppleSyncSkipped[];
	/** The synced list's name ("Badger"; "Badger Dev" in debug builds). */
	list: string;
	/** Show the one-time note that the old lists (`old_lists`) can be deleted on the phone. */
	old_lists_note: boolean;
	old_lists: string[];
}

/** search_index_stats / rebuild_search_index. */
export interface SearchIndexStats {
	notes: number;
	bytes: number;
}

/** rename_note: the renamed note and the notes whose [[links]] to the old title were
 *  rewritten (the renamed note itself included when it links to itself). */
export interface RenamedNote {
	note: NoteSummary;
	rewritten_ids: number[];
}

/** list_tag_suggestions: one `#` autocomplete entry; `tag` is what follows the `#`. */
export interface TagSuggestion {
	kind: 'tag' | 'customer' | 'project' | 'issue' | 'task';
	tag: string; // e.g. "meeting", "acme", "acme/web", "issue/12", "task/3"
	label: string; // tag / customer or project name / "title — customer / project" / task title
	detail: string | null; // projects: the customer name
	status: 'open' | null; // tasks only (open tasks are suggested)
}

/** list_backlinks: a note linking to the open one, with the line that links (raw markdown). */
export interface Backlink {
	id: number;
	title: string;
	snippet: string;
}

/** get_link_targets: what wikilinks and #issue/#task chips resolve to. */
export interface LinkTargets {
	notes: { id: number; title: string }[];
	issues: { id: number; title: string; context: string }[]; // context: "Customer / Project"
	tasks: { id: number; title: string; status: 'open' | 'done' }[];
}

/** render_markdown mode: the read view, or "Copy as HTML" (links and tags as plain text). */
export type RenderMode = 'read' | 'copy';

/** The scratchpad (get_scratchpad, set_scratchpad_body). updated_at is null until the first save. */
export interface Scratchpad {
	body: string;
	updated_at: string | null;
	kind: NoteKind; // kept when the pad is cleared or saved as a note
}

/** One day's end-of-day handover (get_handover, list_handovers, save_handover). `date` is
 *  the local YYYY-MM-DD it was written; `dismissed_at` is set once its card was dismissed. */
export interface Handover {
	date: string;
	body: string;
	dismissed_at: string | null;
	created_at: string;
	updated_at: string;
}

/** Full note (get_note, create_note, create_note_titled, save_scratchpad_as_note, save_handover_as_note). */
export interface Note {
	id: number;
	folder_id: number | null;
	title: string;
	body: string;
	pinned: boolean;
	created_at: string;
	updated_at: string;
	position: number;
	kind: NoteKind;
}

/** Backup folder and encryption settings (`get_backup_settings`). */
export interface BackupSettings {
  dir: string;
  is_default_dir: boolean;
  /** `age1…` key automatic backups are encrypted to; null when encryption is off. */
  public_key: string | null;
}

/** One automatic backup file in the backup folder (`list_startup_backups`). */
export interface StoredBackup {
  date: string;
  size_bytes: number;
  encrypted: boolean;
}

/** Outcome of backups in this run of the app (`get_backup_status`). */
export interface BackupStatus {
  /** Local `YYYY-MM-DDTHH:MM:SS`. */
  last_success: string | null;
  last_error: string | null;
}

/** A key pair from `generate_backup_key`; the secret is shown once and never stored. */
export interface GeneratedBackupKey {
  public_key: string;
  secret_key: string;
}

// ── Playground ────────────────────────────────────────────────────────────────

/** A saved playground script (`get_script`, `create_script`). */
export interface Script {
	id: number;
	title: string;
	body: string;
	created_at: string;
	updated_at: string;
}

/** A script in the playground list (`list_scripts`, `update_script_title`, `update_script_body`). */
export interface ScriptSummary {
	id: number;
	title: string;
	updated_at: string;
}

export type OutputLevel = 'log' | 'info' | 'warn' | 'error' | 'debug';

/** One captured `console.*` call. */
export interface OutputLine {
	level: OutputLevel;
	text: string;
}

export type RunErrorKind = 'user' | 'timeout' | 'stopped' | 'oom' | 'stack' | 'syntax';

/** Why a run failed; `line`/`col` (1-based) point into the script when known. */
export interface RunError {
	kind: RunErrorKind;
	message: string;
	line: number | null;
	col: number | null;
}

/** What a run produced. `value` is the formatted last expression (absent when undefined). */
export interface RunOutcome {
	lines: OutputLine[];
	value: string | null;
	error: RunError | null;
	/** Output hit the cap; later console output was dropped. */
	truncated: boolean;
	duration_ms: number;
}

/** `run_script` result. */
export interface RunResult {
	run_id: number;
	outcome: RunOutcome;
}

/** `check_for_update` result: a newer release than the running one. */
export interface UpdateInfo {
	version: string;
	current_version: string;
	notes: string | null;
}
