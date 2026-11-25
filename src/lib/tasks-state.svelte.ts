// Shared tasks list, kept current from Rust's `tasks-changed` / `reminders-changed` events.
// The root layout starts the watch; /tasks, the Today card, the nav count, the reminder
// toasts and the Undo toast all read it. Completing goes through `completeTask`, which
// plays the row animation (tick, then slide out) wherever the row is shown and offers Undo.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Task, TaskCompletion, TaskInput, TaskReminderInput } from '#lib/types.js';
import {
	LEAVE_MS,
	REMINDERS_CHANGED_EVENT,
	TASKS_CHANGED_EVENT,
	TASKS_OPEN_EVENT,
	TICK_MS,
	undoActive,
	undoEntry,
	type PendingUndo
} from '#lib/tasks.js';

const CLOCK_MS = 30_000;

export const tasks = $state<{ list: Task[]; loaded: boolean; error: string }>({
	list: [],
	loaded: false,
	error: ''
});

/** Epoch ms, ticking every 30 s while watched: overdue counts, groups and labels follow it. */
export const clock = $state({ ms: Date.now() });

/** Height (px) of the fixed reminder toast stack; 0 when none shows. Pages pad their bottom by it. */
export const toastArea = $state({ height: 0 });

/** A task to open on /tasks (notification click, ⌘K); the page handles it once (`pending` → false). */
export const openRequest = $state<{ id: number | null; pending: boolean }>({ id: null, pending: false });

/**
 * Rows mid-completion. `tick`: the row stays in place (as it was) while the check pops and the
 * title strikes through; `leave`: it slides out of its group. Then the real row shows where it
 * now belongs (Completed, or its next due for a repeating task).
 */
export const completing = $state<{ rows: Record<number, { phase: 'tick' | 'leave'; task: Task }>; bump: number }>({
	rows: {},
	bump: 0
});

/** The last completion, while Undo is offered. */
export const undo = $state<{ pending: PendingUndo | null; error: string }>({ pending: null, error: '' });

let inFlight = false;
let again = false;

/** Re-reads the list; bursts of events coalesce into one trailing read. */
export async function refreshTasks(): Promise<void> {
	if (inFlight) {
		again = true;
		return;
	}
	inFlight = true;
	try {
		do {
			again = false;
			tasks.list = await invoke<Task[]>('list_tasks');
			tasks.error = '';
			tasks.loaded = true;
			clock.ms = Date.now();
		} while (again);
	} catch (e) {
		tasks.error = String(e);
	} finally {
		inFlight = false;
	}
}

/**
 * Loads the list and follows the change events; `onopen` gets each notification click (`id`
 * null for a batch). Window focus re-reads, so reminders sent while the app was in the
 * background show as toasts. Returns the cleanup.
 */
export function watchTasks(onopen: (id: number | null) => void): () => void {
	let disposed = false;
	const stops: (() => void)[] = [];
	const keep = (stop: () => void) => (disposed ? stop() : stops.push(stop));
	void refreshTasks();
	for (const event of [TASKS_CHANGED_EVENT, REMINDERS_CHANGED_EVENT]) {
		listen(event, () => void refreshTasks())
			.then(keep)
			.catch(() => {});
	}
	listen<{ task_id: number | null } | null>(TASKS_OPEN_EVENT, (e) => onopen(e.payload?.task_id ?? null))
		.then(keep)
		.catch(() => {});
	const onFocus = () => void refreshTasks();
	window.addEventListener('focus', onFocus);
	const timer = setInterval(() => (clock.ms = Date.now()), CLOCK_MS);
	return () => {
		disposed = true;
		stops.forEach((stop) => stop());
		window.removeEventListener('focus', onFocus);
		clearInterval(timer);
	};
}

/** Asks /tasks to select and scroll to a task (null: just the page). */
export function requestOpen(id: number | null): void {
	openRequest.id = id;
	openRequest.pending = true;
}

/** Replaces (or adds) a task returned by a command, ahead of the event-driven refresh. */
function upsert(t: Task): void {
	const i = tasks.list.findIndex((x) => x.id === t.id);
	if (i === -1) tasks.list = [...tasks.list, t];
	else tasks.list[i] = t;
}

/** The list as rows show it: tasks mid-tick keep their pre-completion look; leaving ones are gone. */
export function shownTasks(list: readonly Task[], rows: typeof completing.rows): Task[] {
	return list.flatMap((t) => {
		const c = rows[t.id];
		if (!c) return [t];
		return c.phase === 'tick' ? [c.task] : [];
	});
}

/** True when the user asked for less motion: completion and row transitions are instant. */
export function reducedMotion(): boolean {
	return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

// The actions below throw (or resolve to) the user-facing error message, so callers show it
// inline next to the control that ran it.

async function call(cmd: string, args: Record<string, unknown>): Promise<Task> {
	try {
		const t = await invoke<Task>(cmd, args);
		upsert(t);
		void refreshTasks();
		return t;
	} catch (e) {
		void refreshTasks(); // e.g. "Task not found": drop the stale row
		throw String(e);
	}
}

export function createTask(input: TaskInput, reminders: TaskReminderInput[] = []): Promise<Task> {
	return call('create_task', { input, reminders });
}

export function updateTask(id: number, input: TaskInput): Promise<Task> {
	return call('update_task', { id, input });
}

export function reopenTask(id: number): Promise<Task> {
	return call('reopen_task', { id });
}

export function createIssueForTask(taskId: number, projectId: number, issueTitle: string): Promise<Task> {
	return call('create_issue_for_task', { taskId, projectId, issueTitle });
}

export function addTaskReminder(taskId: number, reminder: TaskReminderInput): Promise<Task> {
	return call('add_task_reminder', { taskId, reminder });
}

export function updateTaskReminder(id: number, reminder: TaskReminderInput): Promise<Task> {
	return call('update_task_reminder', { id, reminder });
}

export function deleteTaskReminder(id: number): Promise<Task> {
	return call('delete_task_reminder', { id });
}

/** `until`: a local timestamp from `snoozeUntil`. Moves only this reminder; the due stays. */
export function snoozeTaskReminder(id: number, until: string): Promise<Task> {
	return call('snooze_task_reminder', { id, until });
}

/** Toast ×: the reminder stops counting as overdue. */
export function dismissTaskReminder(id: number): Promise<Task> {
	return call('dismiss_task_reminder', { id });
}

export async function deleteTask(id: number): Promise<void> {
	try {
		await invoke('delete_task', { id });
		tasks.list = tasks.list.filter((t) => t.id !== id);
		void refreshTasks();
	} catch (e) {
		void refreshTasks();
		throw String(e);
	}
}

const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));

/**
 * Completes a task: the row ticks (450 ms), slides out (250 ms), and Undo is offered for 5 s.
 * `onleave` runs just before the row goes (so focus can move to the next one). Instant under
 * reduced motion. Resolves to '' or the error message.
 */
export async function completeTask(task: Task, onleave?: () => void): Promise<string> {
	if (completing.rows[task.id]) return '';
	const animate = !reducedMotion();
	// eslint-disable-next-line svelte/prefer-svelte-reactivity -- a timestamp, never mutated or rendered
	const started = new Date();
	const startedMs = started.getTime();
	if (animate) completing.rows[task.id] = { phase: 'tick', task: $state.snapshot(task) as Task };
	let done: TaskCompletion;
	try {
		done = await invoke<TaskCompletion>('complete_task', { id: task.id });
	} catch (e) {
		delete completing.rows[task.id];
		void refreshTasks();
		return String(e);
	}
	upsert(done.task);
	undo.pending = undoEntry(done, task, started);
	undo.error = '';
	if (animate) {
		await wait(Math.max(0, TICK_MS - (Date.now() - startedMs)));
		onleave?.();
		if (completing.rows[task.id]) completing.rows[task.id].phase = 'leave';
		await wait(LEAVE_MS);
		delete completing.rows[task.id];
	} else {
		onleave?.();
	}
	completing.bump += 1;
	void refreshTasks();
	return '';
}

let undoing = false;

/**
 * Puts the last completed task back exactly as it was (due, reminder states, snooze). A
 * repeated ⌘Z while one is on its way is ignored, so the same undo is never sent twice.
 */
export async function undoCompletion(): Promise<void> {
	if (undoing) return;
	const u = undo.pending;
	if (!undoActive(u, Date.now())) {
		undo.pending = null;
		return;
	}
	undoing = true;
	try {
		upsert(await invoke<Task>('undo_complete_task', { undo: u.undo }));
		if (undo.pending === u) undo.pending = null;
		undo.error = '';
	} catch (e) {
		undo.error = String(e);
	} finally {
		undoing = false;
		void refreshTasks();
	}
}
