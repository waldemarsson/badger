// Pure helpers for the Today page. Dates are local YYYY-MM-DD strings; callers pass
// "today" in so these stay deterministic and testable.
import type { TimeEntryRow, TodoRow } from '#lib/types.js';

export interface DayTotal {
	date: string; // YYYY-MM-DD
	hours: number;
}

function parseLocal(ymd: string): Date {
	const [y, m, d] = ymd.split('-').map(Number);
	return new Date(y, m - 1, d);
}

function formatLocal(date: Date): string {
	const mm = String(date.getMonth() + 1).padStart(2, '0');
	const dd = String(date.getDate()).padStart(2, '0');
	return `${date.getFullYear()}-${mm}-${dd}`;
}

/** Monday..Sunday of the week containing `todayIso` (Monday start, like getWeekBounds). */
export function weekDays(todayIso: string): string[] {
	const date = parseLocal(todayIso);
	const offset = (date.getDay() + 6) % 7; // 0 = Monday
	return Array.from({ length: 7 }, (_, i) =>
		formatLocal(new Date(date.getFullYear(), date.getMonth(), date.getDate() - offset + i))
	);
}

export function isWeekend(ymd: string): boolean {
	const day = parseLocal(ymd).getDay();
	return day === 0 || day === 6;
}

export function sumHours(entries: TimeEntryRow[]): number {
	return entries.reduce((sum, e) => sum + e.hours, 0);
}

/** Hours per day for `days`, in the same order; dates outside `days` are ignored. */
export function dailyTotals(entries: TimeEntryRow[], days: string[]): DayTotal[] {
	const totals = new Map(days.map((d) => [d, 0]));
	for (const e of entries) {
		const current = totals.get(e.date);
		if (current !== undefined) totals.set(e.date, current + e.hours);
	}
	return days.map((date) => ({ date, hours: totals.get(date) ?? 0 }));
}

/** Entries dated `todayIso`, in input order. */
export function todayEntries(entries: TimeEntryRow[], todayIso: string): TimeEntryRow[] {
	return entries.filter((e) => e.date === todayIso);
}

function isDueByToday(t: TodoRow, todayIso: string): boolean {
	return t.status !== 'done' && t.due_date !== null && t.due_date <= todayIso;
}

/** Non-done todos due on or before today: by due date (overdue first), then board position. */
export function needsAttention(todos: TodoRow[], todayIso: string): TodoRow[] {
	return todos
		.filter((t) => isDueByToday(t, todayIso))
		.sort(
			(a, b) =>
				(a.due_date as string).localeCompare(b.due_date as string) ||
				a.position - b.position ||
				a.id - b.id
		);
}

/** In-progress todos not already in needsAttention, in board order. */
export function inProgress(todos: TodoRow[], todayIso: string): TodoRow[] {
	return todos
		.filter((t) => t.status === 'in_progress' && !isDueByToday(t, todayIso))
		.sort((a, b) => a.position - b.position || a.id - b.id);
}

/** Fraction of `target` reached, clamped to 0..1; 0 when the target is off (<= 0). */
export function progress(hours: number, target: number): number {
	if (!(target > 0)) return 0;
	return Math.min(1, Math.max(0, hours / target));
}

/** The value when it is a real YYYY-MM-DD date, else null. */
export function parseDayParam(value: string | null | undefined): string | null {
	if (!value || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
	return formatLocal(parseLocal(value)) === value ? value : null;
}

export const DAILY_TARGET_ERROR = 'Daily target must be between 0 and 24 hours';

/** Parses the Settings target field ("7.5" or "7,5"); 0 turns the target off. */
export function parseDailyTarget(input: string): { target: number } | { error: string } {
	const value = input.trim().replace(',', '.');
	if (!value) return { error: 'Daily target is required' };
	if (!/^-?(\d+(\.\d+)?|\.\d+)$/.test(value)) return { error: 'Daily target must be a number, e.g. 7.5' };
	const target = Number(value);
	if (target < 0 || target > 24) return { error: DAILY_TARGET_ERROR };
	return { target };
}

/** "Fri, Nov 6" for a YYYY-MM-DD date, in the user's locale. */
export function formatDayLabel(ymd: string): string {
	return parseLocal(ymd).toLocaleDateString(undefined, {
		weekday: 'short',
		month: 'short',
		day: 'numeric'
	});
}

/** Hours without float noise or trailing zeros ("3.5", "8"). */
export function formatHours(h: number): string {
	const r = Math.round(h * 10000) / 10000;
	return Number.isInteger(r) ? String(Math.round(r)) : String(r);
}
