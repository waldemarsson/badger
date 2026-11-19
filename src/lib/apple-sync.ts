// Pure helpers for Apple Reminders sync: event names, the Settings status pill and
// skipped-items summary, and the "Keep off phone" (Mac-only) rules for todos and reminders.
import type { AppleSyncSkipped, AppleSyncStatus, Reminder, ReminderRepeat } from '#lib/types.js';

/** Tauri event sent whenever the sync status changes (no payload). */
export const APPLE_SYNC_STATUS_EVENT = 'apple-sync-status';
/** Tauri event sent when sync changed todos (no payload); todo views re-read. */
export const TODOS_CHANGED_EVENT = 'todos-changed';

export type AppleSyncTone = 'off' | 'ok' | 'warn' | 'error';

/** Short status text and its tone for the Settings pill. */
export function appleSyncPill(status: AppleSyncStatus): { label: string; tone: AppleSyncTone } {
	if (!status.available) return { label: 'Not available', tone: 'off' };
	if (!status.enabled) return { label: 'Off', tone: 'off' };
	if (status.access !== 'granted') return { label: 'No access', tone: 'warn' };
	if (status.paused) return { label: 'Paused', tone: 'warn' };
	if (status.last_error) return { label: 'Error', tone: 'error' };
	if (!status.last_sync_at) return { label: 'Starting…', tone: 'off' };
	return { label: 'On', tone: 'ok' };
}

/** "Not yet", "Just now", "5 min ago", "Today 14:05" or a date and time. */
export function formatLastSync(stamp: string | null, now: Date): string {
	if (!stamp) return 'Not yet';
	const at = new Date(stamp);
	if (Number.isNaN(at.getTime())) return stamp;
	const minutes = Math.floor((now.getTime() - at.getTime()) / 60_000);
	if (minutes < 1) return 'Just now';
	if (minutes < 60) return `${minutes} min ago`;
	const time = `${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')}`;
	if (at.toDateString() === now.toDateString()) return `Today ${time}`;
	return `${at.getFullYear()}-${String(at.getMonth() + 1).padStart(2, '0')}-${String(at.getDate()).padStart(2, '0')} ${time}`;
}

/** "1 needs a date, 2 need a title" — counts per reason, in first-seen order. */
export function skippedSummary(skipped: readonly AppleSyncSkipped[]): string {
	const counts = new Map<string, number>();
	for (const s of skipped) counts.set(s.reason, (counts.get(s.reason) ?? 0) + 1);
	return [...counts]
		.map(([reason, n]) => `${n} ${n === 1 ? reason : reason.replace(/^needs /, 'need ')}`)
		.join(', ');
}

/** Whether sync is on, so the "Keep off phone" controls and "Mac only" marks show. */
export function syncShowsMacOnly(status: Pick<AppleSyncStatus, 'available' | 'enabled'> | null): boolean {
	return !!status && status.available && status.enabled;
}

/** A reminder stays on the Mac when it or its linked todo is kept off the phone. */
export function reminderMacOnly(r: Pick<Reminder, 'mac_only' | 'todo_mac_only'>): boolean {
	return r.mac_only || r.todo_mac_only;
}

/** How the reminder form offers "Keep off phone": hidden when sync is off or the reminder
 *  repeats (recurring ones never sync), locked on when its todo is Mac-only, else a toggle. */
export type MacOnlyControl = 'hidden' | 'follows-todo' | 'toggle';

export function reminderMacOnlyControl(syncOn: boolean, repeat: ReminderRepeat, todoMacOnly: boolean): MacOnlyControl {
	if (!syncOn || repeat !== 'none') return 'hidden';
	return todoMacOnly ? 'follows-todo' : 'toggle';
}
