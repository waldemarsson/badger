// Shared reminders list, kept current from Rust's `reminders-changed` events. The root layout
// starts the watch; the reminders page, the Today card, the todo board, TodoModal and the
// toast stack all read `reminders.list`. Toasts are derived from fired rows, so launch
// catch-up and reloads aren't lost; × hides one for this session only.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { SvelteSet } from 'svelte/reactivity';
import type { Reminder, ReminderInput } from '#lib/types.js';
import { toastKey } from '#lib/reminders.js';

export const REMINDERS_CHANGED_EVENT = 'reminders-changed';
export const REMINDERS_OPEN_EVENT = 'reminders-open';

const CLOCK_MS = 30_000;

export const reminders = $state<{ list: Reminder[]; loaded: boolean; error: string }>({
	list: [],
	loaded: false,
	error: ''
});

/** Epoch ms, ticking every 30 s while watched, so relative labels ("14:52", "Tomorrow") and grouping stay current. */
export const clock = $state({ ms: Date.now() });

/** Height (px) of the fixed toast stack; 0 when no toast shows. Pages pad their bottom by it. */
export const toastArea = $state({ height: 0 });

/** Toast keys (`id@remind_at`) hidden this session. */
export const dismissedToasts = new SvelteSet<string>();

/** A row to scroll to and flash on /reminders; the page handles it once (`pending` → false). */
export const highlightRequest = $state<{ id: number | null; pending: boolean }>({ id: null, pending: false });

let inFlight = false;
let again = false;

/** Re-reads the list; bursts of events coalesce into one trailing read. */
export async function refreshReminders(): Promise<void> {
	if (inFlight) {
		again = true;
		return;
	}
	inFlight = true;
	try {
		do {
			again = false;
			reminders.list = await invoke<Reminder[]>('list_reminders');
			reminders.error = '';
			reminders.loaded = true;
			clock.ms = Date.now();
		} while (again);
	} catch (e) {
		reminders.error = String(e);
	} finally {
		inFlight = false;
	}
}

/**
 * Loads the list and follows `reminders-changed`; `onopen` gets each notification click
 * (`id` null for a batch). Window focus re-reads the list, so fired rows the native
 * notification announced while the app was in the background show as toasts. Returns the cleanup.
 */
export function watchReminders(onopen: (id: number | null) => void): () => void {
	let disposed = false;
	const stops: (() => void)[] = [];
	const keep = (stop: () => void) => (disposed ? stop() : stops.push(stop));
	void refreshReminders();
	listen(REMINDERS_CHANGED_EVENT, () => void refreshReminders())
		.then(keep)
		.catch(() => {});
	listen<{ id: number | null } | null>(REMINDERS_OPEN_EVENT, (e) => onopen(e.payload?.id ?? null))
		.then(keep)
		.catch(() => {});
	const onFocus = () => void refreshReminders();
	window.addEventListener('focus', onFocus);
	const timer = setInterval(() => (clock.ms = Date.now()), CLOCK_MS);
	return () => {
		disposed = true;
		stops.forEach((stop) => stop());
		window.removeEventListener('focus', onFocus);
		clearInterval(timer);
	};
}

/** Asks /reminders to scroll to and flash a row (null: just the page). */
export function requestHighlight(id: number | null): void {
	highlightRequest.id = id;
	highlightRequest.pending = true;
}

/** Replaces (or adds) a row returned by a command, ahead of the event-driven refresh. */
function upsert(r: Reminder): void {
	const i = reminders.list.findIndex((x) => x.id === r.id);
	if (i === -1) reminders.list = [...reminders.list, r];
	else reminders.list[i] = r;
}

// The actions below resolve to the user-facing error message ('' on success), so callers
// show it inline next to the control that ran it.

async function run(action: () => Promise<void>): Promise<string> {
	try {
		await action();
		void refreshReminders();
		return '';
	} catch (e) {
		void refreshReminders(); // e.g. "Reminder not found": drop the stale row
		return String(e);
	}
}

export function completeReminder(id: number): Promise<string> {
	return run(async () => upsert(await invoke<Reminder>('complete_reminder', { id })));
}

/** `until`: a local timestamp from `snoozeUntil`. */
export function snoozeReminder(id: number, until: string): Promise<string> {
	return run(async () => upsert(await invoke<Reminder>('snooze_reminder', { id, until })));
}

export function deleteReminder(id: number): Promise<string> {
	return run(async () => {
		await invoke('delete_reminder', { id });
		reminders.list = reminders.list.filter((r) => r.id !== id);
	});
}

/** Creates or updates; resolves to the saved row, or throws the user-facing message. */
export async function saveReminder(id: number | null, input: ReminderInput): Promise<Reminder> {
	try {
		const r =
			id === null
				? await invoke<Reminder>('create_reminder', { input })
				: await invoke<Reminder>('update_reminder', { id, input });
		upsert(r);
		void refreshReminders();
		return r;
	} catch (e) {
		throw String(e);
	}
}

export function dismissToast(r: Reminder): void {
	dismissedToasts.add(toastKey(r));
}
