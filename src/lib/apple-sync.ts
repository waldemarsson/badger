// Pure helpers for Apple Reminders sync: the event name, the Settings status pill and
// skipped-items summary, and the "Keep off phone" (Mac-only) rules for tasks. Sync's own
// task changes arrive as `tasks-changed` (see tasks.ts).
import type { AppleSyncSkipped, AppleSyncStatus, TaskRepeat } from '#lib/types.js';

/** Tauri event sent whenever the sync status changes (no payload). */
export const APPLE_SYNC_STATUS_EVENT = 'apple-sync-status';

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

/** How the details panel offers "Keep off phone": hidden while sync is off, explained for
 *  repeating tasks (they never sync), else a switch. */
export type MacOnlyControl = 'hidden' | 'repeating' | 'toggle';

export function taskMacOnlyControl(syncOn: boolean, repeat: TaskRepeat): MacOnlyControl {
	if (!syncOn) return 'hidden';
	return repeat === 'none' ? 'toggle' : 'repeating';
}
