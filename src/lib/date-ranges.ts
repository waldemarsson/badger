export function today(): string {
	return new Date().toISOString().slice(0, 10);
}

export function getWeekBounds(weekOffset = 0): { start: string; end: string } {
	const date = new Date();
	date.setDate(date.getDate() + weekOffset * 7);
	const day = date.getDay();
	const diff = day === 0 ? -6 : 1 - day;
	const monday = new Date(date);
	monday.setDate(date.getDate() + diff);
	const sunday = new Date(monday);
	sunday.setDate(monday.getDate() + 6);
	const format = (value: Date) => value.toISOString().slice(0, 10);
	return { start: format(monday), end: format(sunday) };
}

export function getMonthBounds(monthOffset = 0): { start: string; end: string } {
	const now = new Date();
	const first = new Date(now.getFullYear(), now.getMonth() + monthOffset, 1);
	const last = new Date(now.getFullYear(), now.getMonth() + monthOffset + 1, 0);
	const format = (value: Date) => value.toISOString().slice(0, 10);
	return { start: format(first), end: format(last) };
}
