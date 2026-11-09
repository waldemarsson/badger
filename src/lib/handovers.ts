// Pure helpers for handovers (no Svelte, no Tauri). Dates are local YYYY-MM-DD strings;
// callers pass "today" in so these stay deterministic and testable.
import type { Handover } from '#lib/types.js';

function parseLocal(ymd: string): Date {
	const [y, m, d] = ymd.split('-').map(Number);
	return new Date(y, m - 1, d);
}

function formatLocal(date: Date): string {
	const mm = String(date.getMonth() + 1).padStart(2, '0');
	const dd = String(date.getDate()).padStart(2, '0');
	return `${date.getFullYear()}-${mm}-${dd}`;
}

/** The first Monday–Friday after `ymd` (no holidays): Fri, Sat and Sun all give Monday. */
export function nextWorkingDay(ymd: string): string {
	const date = parseLocal(ymd);
	do {
		date.setDate(date.getDate() + 1);
	} while (date.getDay() === 0 || date.getDay() === 6);
	return formatLocal(date);
}

/**
 * The handover to show on Today: the single newest one written before `today`, if it is
 * not dismissed and `today` is on or after its next working day. Otherwise none; older
 * handovers never stand in (they stay in history).
 */
export function selectIncoming<T extends Pick<Handover, 'date' | 'dismissed_at'>>(handovers: readonly T[], today: string): T | null {
	let newest: T | null = null;
	for (const h of handovers) {
		if (h.date < today && (newest === null || h.date > newest.date)) newest = h;
	}
	if (newest === null || newest.dismissed_at !== null) return null;
	return today >= nextWorkingDay(newest.date) ? newest : null;
}

/** The editor and the save queue key bodies by number: one stable number per date. */
export function handoverKey(ymd: string): number {
	return Number(ymd.replaceAll('-', ''));
}

/** The date a `handoverKey` stands for. */
export function keyDate(key: number): string {
	const s = String(key).padStart(8, '0');
	return `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}`;
}

/** Empty or whitespace-only: not stored (saving it deletes the day's handover). */
export function isBlank(text: string): boolean {
	return text.trim() === '';
}
