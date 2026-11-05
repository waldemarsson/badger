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
	external_id: string | null;
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

/** Standalone todo; link fields are null when the todo is not linked to an issue. */
export interface TodoRow {
	id: number;
	title: string;
	notes: string | null;
	done: boolean;
	due_date: string | null; // YYYY-MM-DD
	created_at: string;
	issue_id: number | null;
	issue_title: string | null;
	project_id: number | null;
	project_name: string | null;
	customer_id: number | null;
	customer_name: string | null;
}
