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

export type TodoStatus = 'not_started' | 'in_progress' | 'done';

/** Kanban todo; link fields are null when the todo is not linked to an issue. */
export interface TodoRow {
	id: number;
	title: string;
	notes: string | null;
	status: TodoStatus;
	position: number; // order within its status column
	due_date: string | null; // YYYY-MM-DD
	created_at: string;
	issue_id: number | null;
	issue_title: string | null;
	project_id: number | null;
	project_name: string | null;
	customer_id: number | null;
	customer_name: string | null;
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

/** Note list row (list_notes, update_note_body, set_note_pinned). */
export interface NoteSummary {
	id: number;
	folder_id: number | null;
	title: string;
	excerpt: string; // first meaningful body line, markers stripped; '' for an empty body
	pinned: boolean;
	created_at: string; // local YYYY-MM-DDTHH:MM:SS.mmm
	updated_at: string;
}

/** search_notes result: a list row plus title/snippet with hits wrapped in \u0002…\u0003. */
export interface NoteHit extends NoteSummary {
	title_marked: string;
	snippet: string; // '' when the body has no highlighted hit
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
	kind: 'tag' | 'customer' | 'project' | 'issue' | 'todo';
	tag: string; // e.g. "meeting", "acme", "acme/web", "issue/12", "todo/3"
	label: string; // tag / customer or project name / "title — customer / project" / todo title
	detail: string | null; // projects: the customer name
	status: Exclude<TodoStatus, 'done'> | null; // todos only
}

/** list_backlinks: a note linking to the open one, with the line that links (raw markdown). */
export interface Backlink {
	id: number;
	title: string;
	snippet: string;
}

/** get_link_targets: what wikilinks and #issue/#todo chips resolve to. */
export interface LinkTargets {
	notes: { id: number; title: string }[];
	issues: { id: number; title: string; context: string }[]; // context: "Customer / Project"
	todos: { id: number; title: string; status: TodoStatus }[];
}

/** render_markdown mode: the read view, or "Copy as HTML" (links and tags as plain text). */
export type RenderMode = 'read' | 'copy';

/** Full note (get_note, create_note, create_note_titled). */
export interface Note {
	id: number;
	folder_id: number | null;
	title: string;
	body: string;
	pinned: boolean;
	created_at: string;
	updated_at: string;
}
