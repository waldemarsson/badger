function formatLocal(value: Date): string {
	const year = value.getFullYear();
	const month = String(value.getMonth() + 1).padStart(2, '0');
	const day = String(value.getDate()).padStart(2, '0');
	return `${year}-${month}-${day}`;
}

export function today(): string {
	return formatLocal(new Date());
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
	return { start: formatLocal(monday), end: formatLocal(sunday) };
}

export function getMonthBounds(monthOffset = 0): { start: string; end: string } {
	const now = new Date();
	const first = new Date(now.getFullYear(), now.getMonth() + monthOffset, 1);
	const last = new Date(now.getFullYear(), now.getMonth() + monthOffset + 1, 0);
	return { start: formatLocal(first), end: formatLocal(last) };
}
