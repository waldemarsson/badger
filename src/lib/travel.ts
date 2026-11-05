// Pure helpers for travel display and the monthly copy-as-text report.
import type { TravelRow } from './types';

export const MONTH_NAMES = [
	'January', 'February', 'March', 'April', 'May', 'June',
	'July', 'August', 'September', 'October', 'November', 'December',
];

/** Plain number: no thousands separator, decimals only when not whole (max 4). */
export function formatNumber(n: number): string {
	const r = Math.round(n * 10000) / 10000;
	return Number.isInteger(r) ? String(Math.round(r)) : String(r);
}

/** "2026-10-05" → "5/10". */
export function tripDateLabel(date: string): string {
	const [, m, d] = date.split('-').map(Number);
	return `${d}/${m}`;
}

/** "500 km - 1300 kr", "500 km", "1300 kr", or "" when neither is entered. */
export function tripAmountLine(trip: Pick<TravelRow, 'km' | 'amount'>): string {
	const parts: string[] = [];
	if (trip.km !== null) parts.push(`${formatNumber(trip.km)} km`);
	if (trip.amount !== null) parts.push(`${formatNumber(trip.amount)} kr`);
	return parts.join(' - ');
}

/** Sums of entered km and amount; missing values are skipped. */
export function tripTotals(trips: Pick<TravelRow, 'km' | 'amount'>[]): { km: number; amount: number } {
	return trips.reduce<{ km: number; amount: number }>(
		(t, trip) => ({ km: t.km + (trip.km ?? 0), amount: t.amount + (trip.amount ?? 0) }),
		{ km: 0, amount: 0 },
	);
}

export interface CopyCustomer {
	name: string;
	projects: { name: string; hours: number }[];
}

/**
 * Month report text: month name, hours per customer/project with a total, then trips.
 * Returns null when there is neither time nor travel.
 */
export function monthReportText(month: number, customers: CopyCustomer[], trips: TravelRow[]): string | null {
	if (customers.length === 0 && trips.length === 0) return null;

	const blocks: string[] = [MONTH_NAMES[month - 1]];
	if (customers.length > 0) {
		for (const c of customers) {
			blocks.push([c.name, ...c.projects.map((p) => `${p.name}: ${formatNumber(p.hours)} h`)].join('\n'));
		}
		const total = customers.reduce((sum, c) => sum + c.projects.reduce((s, p) => s + p.hours, 0), 0);
		blocks.push(`Total: ${formatNumber(total)} h`);
	}
	if (trips.length > 0) {
		blocks.push('Travel:');
		for (const t of trips) {
			const amount = tripAmountLine(t);
			blocks.push(`${tripDateLabel(t.date)} ${t.from_place} - ${t.to_place}${amount ? `\n${amount}` : ''}`);
		}
	}
	return blocks.join('\n\n');
}
