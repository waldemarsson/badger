// Pure task helpers: grouping and order, overdue (D10), row chips and reminder labels,
// reminder time math for the details panel, the quick-add mapping, undo entries and
// focus after completion. No runtime `#lib/…` imports: node:test loads this file directly
// (the quick-add phrase parser is passed in). Dates are local `YYYY-MM-DD`, due times
// `HH:MM`, reminder times naive local `YYYY-MM-DDTHH:MM[:SS.mmm]`, matching the Rust side.
import type { ParseResult } from '#lib/time-phrases.js';
import type { Task, TaskCompletion, TaskInput, TaskReminder, TaskReminderInput, TaskRepeat, TaskUndo } from '#lib/types.js';

/** Tauri events (no payload): any task or reminder write, sync import, scheduler send. */
export const TASKS_CHANGED_EVENT = 'tasks-changed';
export const REMINDERS_CHANGED_EVENT = 'reminders-changed';
/** Notification click: `{ task_id }`, null for a batch. */
export const TASKS_OPEN_EVENT = 'tasks-open';

export const MAX_REMINDERS = 10;
export const MAX_TITLE = 500;
/** How long the Undo toast offers to put a completed task back. */
export const UNDO_MS = 5000;
/** Completion animation: tick (pop, draw, sparks, strike), then the row slides out. */
export const TICK_MS = 450;
export const LEAVE_MS = 250;
/** Untimed due dates anchor relative reminders at 09:00 (D5). */
export const DEFAULT_TIME = '09:00';

// ---------------------------------------------------------------------------
// Dates

const pad = (n: number) => String(n).padStart(2, '0');
const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** Local `YYYY-MM-DD`. */
export function ymd(d: Date): string {
	return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Local `HH:MM`. */
export function hhmm(d: Date): string {
	return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Local `YYYY-MM-DDTHH:MM` (the form of `<input type="datetime-local">`). */
export function minuteStamp(d: Date): string {
	return `${ymd(d)}T${hhmm(d)}`;
}

/** Parses `YYYY-MM-DD` or a local timestamp (seconds optional); invalid → Invalid Date. */
export function parseLocal(s: string): Date {
	const m = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?(?:\.\d+)?)?$/.exec(s.trim());
	if (!m) return new Date(NaN);
	return new Date(+m[1], +m[2] - 1, +m[3], m[4] ? +m[4] : 0, m[5] ? +m[5] : 0, m[6] ? +m[6] : 0);
}

/** `YYYY-MM-DD` `n` days after `day`. */
export function addDays(day: string, n: number): string {
	const d = parseLocal(day);
	return ymd(new Date(d.getFullYear(), d.getMonth(), d.getDate() + n));
}

/** Monday of the week containing `day`. */
export function weekStart(day: string): string {
	const d = parseLocal(day);
	return addDays(day, -((d.getDay() + 6) % 7));
}

/** `Wed 25 Nov` (with the year when it isn't `now`'s). */
export function formatDay(day: string, now: Date): string {
	const d = parseLocal(day);
	if (Number.isNaN(d.getTime())) return day;
	const year = d.getFullYear() === now.getFullYear() ? '' : ` ${d.getFullYear()}`;
	return `${DAY_NAMES[d.getDay()]} ${d.getDate()} ${MONTH_NAMES[d.getMonth()]}${year}`;
}

/** `Today 09:00`, `Tomorrow 09:00`, `Yesterday 09:00` or `Fri 27 Nov 10:00`. */
export function formatWhen(stamp: string, now: Date): string {
	const day = stamp.slice(0, 10);
	const time = stamp.slice(11, 16);
	const today = ymd(now);
	if (day === today) return `Today ${time}`;
	if (day === addDays(today, 1)) return `Tomorrow ${time}`;
	if (day === addDays(today, -1)) return `Yesterday ${time}`;
	return `${formatDay(day, now)} ${time}`;
}

/** Shorter, for chips: `14:00` today, else `Fri 27 Nov 10:00`. */
export function chipWhen(stamp: string, now: Date): string {
	return stamp.slice(0, 10) === ymd(now) ? stamp.slice(11, 16) : `${formatDay(stamp.slice(0, 10), now)} ${stamp.slice(11, 16)}`;
}

/** `Completed today`, `Completed yesterday`, `Completed Tue 24 Nov`. */
export function completedLabel(t: Pick<Task, 'completed_at'>, now: Date): string {
	if (!t.completed_at) return 'Completed';
	const day = t.completed_at.slice(0, 10);
	const today = ymd(now);
	if (day === today) return 'Completed today';
	if (day === addDays(today, -1)) return 'Completed yesterday';
	return `Completed ${formatDay(day, now)}`;
}

// ---------------------------------------------------------------------------
// Due and overdue (D10)

/** The due as a minute stamp, untimed dates at 09:00; null without a due date. */
export function dueAnchor(t: Pick<Task, 'due_date' | 'due_time'>): string | null {
	return t.due_date ? `${t.due_date}T${t.due_time ?? DEFAULT_TIME}` : null;
}

/** Open and past due: an earlier day, or today with a time that has come. */
export function isPastDue(t: Pick<Task, 'done' | 'due_date' | 'due_time'>, now: Date): boolean {
	if (t.done || !t.due_date) return false;
	const today = ymd(now);
	return t.due_date < today || (t.due_date === today && t.due_time !== null && t.due_time <= hhmm(now));
}

/** Open with a reminder that was sent and not dismissed. */
export function hasSentReminder(t: Pick<Task, 'done' | 'reminders'>): boolean {
	return !t.done && t.reminders.some((r) => r.state === 'sent');
}

/** Counts toward the nav badge and the dock badge: past due or a sent reminder. */
export function needsAttention(t: Task, now: Date): boolean {
	return !t.done && (isPastDue(t, now) || hasSentReminder(t));
}

export function overdueCount(tasks: readonly Task[], now: Date): number {
	return tasks.filter((t) => needsAttention(t, now)).length;
}

// ---------------------------------------------------------------------------
// Groups and order (D3, D15)

export type GroupKey = 'overdue' | 'today' | 'upcoming' | 'none';

export const GROUPS: { key: GroupKey; label: string }[] = [
	{ key: 'overdue', label: 'Overdue' },
	{ key: 'today', label: 'Today' },
	{ key: 'upcoming', label: 'Upcoming' },
	{ key: 'none', label: 'No date' }
];

/** Which list group an open task sits in; done tasks are `done`. */
export function taskGroup(t: Pick<Task, 'done' | 'due_date'>, now: Date): GroupKey | 'done' {
	if (t.done) return 'done';
	if (!t.due_date) return 'none';
	const today = ymd(now);
	if (t.due_date < today) return 'overdue';
	return t.due_date === today ? 'today' : 'upcoming';
}

/** Open order: due date, due time (untimed last), created, id. */
export function compareOpen(a: Task, b: Task): number {
	return (
		(a.due_date ?? '9999').localeCompare(b.due_date ?? '9999') ||
		(a.due_time ?? '99').localeCompare(b.due_time ?? '99') ||
		a.created_at.localeCompare(b.created_at) ||
		a.id - b.id
	);
}

/** Newest completion first. */
export function compareDone(a: Task, b: Task): number {
	return (b.completed_at ?? '').localeCompare(a.completed_at ?? '') || b.id - a.id;
}

export interface TaskGroup {
	key: GroupKey;
	label: string;
	items: Task[];
}

/** The non-empty open groups in list order. */
export function groupTasks(tasks: readonly Task[], now: Date): TaskGroup[] {
	return GROUPS.map((g) => ({
		...g,
		items: tasks.filter((t) => taskGroup(t, now) === g.key).sort(compareOpen)
	})).filter((g) => g.items.length > 0);
}

/** Done tasks, newest first, split at this week's Monday. */
export function completedTasks(tasks: readonly Task[], now: Date): { thisWeek: Task[]; older: Task[] } {
	const monday = weekStart(ymd(now));
	const done = tasks.filter((t) => t.done).sort(compareDone);
	return {
		thisWeek: done.filter((t) => (t.completed_at ?? '').slice(0, 10) >= monday),
		older: done.filter((t) => (t.completed_at ?? '').slice(0, 10) < monday)
	};
}

/** Today page: overdue and due today (open), in list order. */
export function todayTasks(tasks: readonly Task[], now: Date): Task[] {
	return tasks
		.filter((t) => {
			const g = taskGroup(t, now);
			return g === 'overdue' || g === 'today' || (g !== 'done' && hasSentReminder(t));
		})
		.sort(compareOpen);
}

export function dueTodayCount(tasks: readonly Task[], now: Date): number {
	return tasks.filter((t) => taskGroup(t, now) === 'today').length;
}

// ---------------------------------------------------------------------------
// Labels

/** `At due time`, `15 min before`, `1 hour before`, `2 days before`, `1 week before`. */
export function offsetLabel(minutes: number): string {
	if (minutes === 0) return 'At due time';
	const unit = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'} before`;
	if (minutes % 10080 === 0) return unit(minutes / 10080, 'week');
	if (minutes % 1440 === 0) return unit(minutes / 1440, 'day');
	if (minutes % 60 === 0) return unit(minutes / 60, 'hour');
	return `${minutes} min before`;
}

/** The relative presets of the Add reminder menu. */
export const PRESET_OFFSETS = [0, 15, 60, 1440, 10080];

export function reminderLabel(r: Pick<TaskReminder, 'offset_minutes' | 'at'>): string {
	return r.offset_minutes !== null ? offsetLabel(r.offset_minutes) : 'Custom';
}

const ISO_DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

/** `Daily`, `Weekdays`, `Mon, Thu`, `Monthly on the 15th`; `''` for one-offs. */
export function repeatLabel(t: { repeat: TaskRepeat; weekdays: readonly number[]; month_day: number | null }): string {
	switch (t.repeat) {
		case 'none':
			return '';
		case 'daily':
			return 'Daily';
		case 'weekdays':
			return 'Weekdays';
		case 'weekly': {
			const days = [...new Set(t.weekdays)].filter((d) => d >= 1 && d <= 7).sort((a, b) => a - b);
			return days.length ? `Weekly · ${days.map((d) => ISO_DAYS[d - 1]).join(', ')}` : 'Weekly';
		}
		case 'monthly':
			return t.month_day ? `Monthly on the ${ordinal(t.month_day)}` : 'Monthly';
	}
}

function ordinal(n: number): string {
	const teen = n % 100 >= 11 && n % 100 <= 13;
	const suffix = teen ? 'th' : ({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[n % 10] ?? 'th';
	return `${n}${suffix}`;
}

// ---------------------------------------------------------------------------
// Row chips

export type ChipTone = 'red' | 'amber' | 'sky' | 'violet' | 'slate';
export type ChipIcon = 'cal' | 'bell' | 'ring' | 'repeat' | 'link' | 'note' | 'mac';

export interface Chip {
	label: string;
	tone: ChipTone;
	icon: ChipIcon;
	/** Full text for a truncated chip (issue path). */
	title?: string;
}

/** Live reminders (not dismissed) with a fire time, as minute stamps, oldest first. */
function liveTimes(t: Task): { at: string; sent: boolean }[] {
	return t.reminders
		.filter((r) => r.state !== 'dismissed' && r.fire_at !== null)
		.map((r) => ({ at: (r.fire_at as string).slice(0, 16), sent: r.state === 'sent' }))
		.sort((a, b) => a.at.localeCompare(b.at));
}

/**
 * Chips under a row: due (a reminder at the due time folds in as a bell), the reminder that
 * matters (the latest sent one in red, else the next pending one with `+N`), repeat, issue,
 * note, and `Mac only` while sync is on. Done rows show no reminder chip.
 */
export function taskChips(t: Task, now: Date, syncOn = false): Chip[] {
	const chips: Chip[] = [];
	const group = taskGroup(t, now);
	const due = t.due_date && t.due_time ? `${t.due_date}T${t.due_time}` : null;
	const times = liveTimes(t);
	const atDue = due !== null && times.some((x) => x.at === due);
	const others = times.filter((x) => !(atDue && x.at === due));
	if (t.due_date) {
		const over = isPastDue(t, now);
		const label = group === 'today' ? (t.due_time ?? 'Today') : `${formatDay(t.due_date, now)}${t.due_time ? ` · ${t.due_time}` : ''}`;
		const tone: ChipTone = t.done ? 'slate' : over ? 'red' : group === 'today' ? 'amber' : 'slate';
		chips.push({ label, tone, icon: atDue ? (over || times.some((x) => x.at === due && x.sent) ? 'ring' : 'bell') : 'cal' });
	}
	if (!t.done && others.length) {
		const sent = others.filter((x) => x.sent);
		const next = others.filter((x) => !x.sent);
		if (sent.length) chips.push({ label: chipWhen(sent[sent.length - 1].at, now), tone: 'red', icon: 'ring' });
		else chips.push({ label: chipWhen(next[0].at, now) + (next.length > 1 ? ` +${next.length - 1}` : ''), tone: 'sky', icon: 'bell' });
	}
	const repeat = repeatLabel(t);
	if (repeat) chips.push({ label: repeat, tone: 'slate', icon: 'repeat' });
	if (t.issue_title) {
		chips.push({
			label: t.project_name ? `${t.project_name} · ${t.issue_title}` : t.issue_title,
			tone: 'violet',
			icon: 'link',
			title: [t.customer_name, t.project_name, t.issue_title].filter(Boolean).join(' / ')
		});
	}
	if (t.note_title) chips.push({ label: t.note_title, tone: 'slate', icon: 'note' });
	if (syncOn && t.mac_only) chips.push({ label: 'Mac only', tone: 'slate', icon: 'mac' });
	return chips;
}

// ---------------------------------------------------------------------------
// Reminder time math (details panel)

/** When a relative reminder fires for this due (minute stamp); null without a due date. */
export function relativeFireAt(t: Pick<Task, 'due_date' | 'due_time'>, offsetMinutes: number): string | null {
	const anchor = dueAnchor(t);
	if (!anchor) return null;
	const d = parseLocal(anchor);
	return minuteStamp(new Date(d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getMinutes() - offsetMinutes));
}

export type ReminderTone = 'pending' | 'sent' | 'dismissed' | 'dormant';

/** A reminder row's time text and look: `Today 09:00`, `… · sent`, `… · snoozed`, `Needs a due date`. */
export function reminderStatus(r: TaskReminder, done: boolean, now: Date): { when: string; tone: ReminderTone } {
	if (r.fire_at === null) return { when: 'Needs a due date', tone: 'dormant' };
	const when = formatWhen(r.fire_at, now);
	if (r.state === 'sent' && !done) return { when: `${when} · sent`, tone: 'sent' };
	if (r.state === 'dismissed' || (r.state === 'sent' && done)) return { when: `${when} · done`, tone: 'dismissed' };
	if (r.snoozed_until) return { when: `${when} · snoozed`, tone: 'pending' };
	return { when, tone: 'pending' };
}

export interface ReminderPreset {
	offset: number;
	label: string;
	hint: string;
	disabled: boolean;
}

/** The Add reminder menu's presets: hint = when it would fire, `Added`, or `Needs a due date`. */
export function reminderPresets(t: Task, now: Date): ReminderPreset[] {
	const full = t.reminders.length >= MAX_REMINDERS;
	return PRESET_OFFSETS.map((offset) => {
		const taken = t.reminders.some((r) => r.offset_minutes === offset);
		const at = relativeFireAt(t, offset);
		return {
			offset,
			label: offsetLabel(offset),
			hint: taken ? 'Added' : at ? formatWhen(at, now) : 'Needs a due date',
			disabled: taken || !at || full
		};
	});
}

/** Where a new custom reminder starts: the due (when still ahead), else the next whole hour. */
export function customReminderStart(t: Pick<Task, 'due_date' | 'due_time'>, now: Date): string {
	const anchor = dueAnchor(t);
	if (anchor && parseLocal(anchor) > now) return anchor;
	return minuteStamp(new Date(now.getFullYear(), now.getMonth(), now.getDate(), now.getHours() + 1, 0));
}

// ---------------------------------------------------------------------------
// Inputs

/** The update input for `t` with `patch` applied (update replaces every field). */
export function taskInput(t: Task, patch: Partial<TaskInput> = {}): TaskInput {
	return {
		title: t.title,
		notes: t.notes,
		dueDate: t.due_date,
		dueTime: t.due_time,
		repeat: t.repeat,
		weekdays: [...t.weekdays],
		monthDay: t.month_day,
		issueId: t.issue_id,
		noteId: t.note_id,
		macOnly: t.mac_only,
		...patch
	};
}

/**
 * The due part of an edit (D4, D6): a time without a date sets today; clearing the date
 * clears the time and the repeat (a repeating task needs a due date).
 */
export function dueEdit(date: string | null, time: string | null, now: Date): Partial<TaskInput> {
	const dueDate = date || (time ? ymd(now) : null);
	if (!dueDate) return { dueDate: null, dueTime: null, repeat: 'none', weekdays: [], monthDay: null };
	return { dueDate, dueTime: time || null };
}

// ---------------------------------------------------------------------------
// Quick add (D17)

export interface IssueTarget {
	id: number;
	title: string;
	/** "Customer / Project" */
	context: string;
}

export interface QuickAddPlan {
	title: string;
	dueDate: string | null;
	dueTime: string | null;
	repeat: TaskRepeat;
	weekdays: number[];
	monthDay: number | null;
	issue: IssueTarget | null;
	/** A `#tag` that matched no issue (shown as "No issue matches #tag"). */
	unmatchedTag: string | null;
	/** A typed time adds an "At due time" reminder. */
	reminders: TaskReminderInput[];
}

const fold = (s: string) => s.toLowerCase().replace(/[-_]+/g, ' ').trim();

/** The issue a `#tag` names: `#issue/12`, else the first whose title, then context, contains it. */
export function matchIssue(tag: string, issues: readonly IssueTarget[]): IssueTarget | null {
	const byId = /^issue\/(\d+)$/i.exec(tag);
	if (byId) return issues.find((i) => i.id === +byId[1]) ?? null;
	const q = fold(tag);
	if (!q) return null;
	return (
		issues.find((i) => fold(i.title).includes(q)) ??
		issues.find((i) => fold(i.context).includes(q)) ??
		issues.find((i) => fold(`${i.context} ${i.title}`).replace(/ \/ /g, ' ').includes(q)) ??
		null
	);
}

/**
 * Maps quick-add (and ⌘K "New task") text to a task: the first `#tag` links an issue, the
 * phrase parser finds the due. Parsed date → due date; a typed time → due time plus an "At
 * due time" reminder; nothing parsed → no due. `parse` is `parseReminder`.
 */
export function planQuickAdd(
	input: string,
	now: Date,
	issues: readonly IssueTarget[],
	parse: (input: string, now: Date) => ParseResult
): QuickAddPlan {
	let tag: string | null = null;
	const rest = input.replace(/(^|\s)#(\S+)/, (_, lead: string, t: string) => {
		tag = t;
		return lead;
	});
	const issue = tag ? matchIssue(tag, issues) : null;
	const base = { issue, unmatchedTag: tag && !issue ? tag : null };
	const parsed = parse(rest, now);
	if ('error' in parsed) {
		return { ...base, title: parsed.text.trim(), dueDate: null, dueTime: null, repeat: 'none', weekdays: [], monthDay: null, reminders: [] };
	}
	const dueTime = parsed.hasTime ? parsed.remindAt.slice(11, 16) : null;
	return {
		...base,
		title: parsed.text.trim(),
		dueDate: parsed.remindAt.slice(0, 10),
		dueTime,
		repeat: parsed.repeat,
		weekdays: parsed.weekdays,
		monthDay: parsed.monthDay,
		reminders: dueTime ? [{ offsetMinutes: 0 }] : []
	};
}

/** `create_task` arguments for a plan. */
export function planInput(plan: QuickAddPlan, extra: Partial<TaskInput> = {}): { input: TaskInput; reminders: TaskReminderInput[] } {
	return {
		input: {
			title: plan.title,
			dueDate: plan.dueDate,
			dueTime: plan.dueTime,
			repeat: plan.repeat,
			weekdays: plan.weekdays,
			monthDay: plan.monthDay,
			issueId: plan.issue?.id ?? null,
			...extra
		},
		reminders: plan.reminders
	};
}

/** The preview chips under the quick-add field. */
export function planChips(plan: QuickAddPlan, now: Date): Chip[] {
	const chips: Chip[] = [];
	if (plan.dueDate) chips.push({ label: `${formatDay(plan.dueDate, now)}${plan.dueTime ? ` · ${plan.dueTime}` : ''}`, tone: 'slate', icon: 'cal' });
	if (plan.reminders.length) chips.push({ label: 'Reminder at due time', tone: 'sky', icon: 'bell' });
	const repeat = repeatLabel({ repeat: plan.repeat, weekdays: plan.weekdays, month_day: plan.monthDay });
	if (repeat) chips.push({ label: repeat, tone: 'slate', icon: 'repeat' });
	if (plan.issue) chips.push({ label: plan.issue.title, tone: 'violet', icon: 'link', title: `${plan.issue.context} / ${plan.issue.title}` });
	else if (plan.unmatchedTag) chips.push({ label: `No issue matches #${plan.unmatchedTag}`, tone: 'slate', icon: 'link' });
	return chips;
}

/** Inline problem with a plan before it is sent (Rust validates again). */
export function planError(plan: QuickAddPlan): string {
	if (!plan.title) return 'Type what the task is, not only when';
	if (plan.title.length > MAX_TITLE) return `Title is too long (max ${MAX_TITLE} characters)`;
	return '';
}

// ---------------------------------------------------------------------------
// Completion, undo and focus (D15, D21)

export interface PendingUndo {
	taskId: number;
	/** What the toast and the live region say. */
	message: string;
	undo: TaskUndo;
	/** Epoch ms after which Undo is gone. */
	expires: number;
}

/** What a completion says: `Completed “X”`, or for a repeating task its next due. */
export function completionMessage(before: Pick<Task, 'title'>, after: Task, now: Date): string {
	if (after.done || !after.due_date) return `Completed “${before.title}”`;
	return `“${before.title}” is next due ${formatDay(after.due_date, now)}${after.due_time ? ` ${after.due_time}` : ''}`;
}

/** The Undo entry for a completion; null when there is nothing to undo. `undo` is passed on as is. */
export function undoEntry(c: TaskCompletion, before: Pick<Task, 'title'>, now: Date): PendingUndo | null {
	if (!c.undo) return null;
	return { taskId: c.task.id, message: completionMessage(before, c.task, now), undo: c.undo, expires: now.getTime() + UNDO_MS };
}

export function undoActive(u: PendingUndo | null, nowMs: number): u is PendingUndo {
	return u !== null && nowMs < u.expires;
}

/** Where focus goes when row `id` leaves `ids` (visible order): the next row, else the previous. */
export function nextFocusId(ids: readonly number[], id: number): number | null {
	const i = ids.indexOf(id);
	if (i === -1) return null;
	return ids[i + 1] ?? ids[i - 1] ?? null;
}

// ---------------------------------------------------------------------------
// Reminder toasts (D20)

export interface ReminderToast {
	task: Task;
	reminder: TaskReminder;
}

/** Sent reminders on open tasks, oldest first: the first `max` show, the rest count as "+N more". */
export function toastStack(tasks: readonly Task[], max = 3): { shown: ReminderToast[]; more: number } {
	const sent = tasks
		.filter((t) => !t.done)
		.flatMap((task) => task.reminders.filter((r) => r.state === 'sent').map((reminder) => ({ task, reminder })))
		.sort((a, b) => (a.reminder.fire_at ?? '').localeCompare(b.reminder.fire_at ?? '') || a.reminder.id - b.reminder.id);
	return { shown: sent.slice(0, max), more: Math.max(0, sent.length - max) };
}
