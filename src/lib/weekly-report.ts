const MONTH_ABBR = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

export function isoWeekToMonday(year: number, week: number): Date {
	const jan4 = new Date(year, 0, 4);
	const jan4Weekday = (jan4.getDay() + 6) % 7;
	const week1Monday = new Date(jan4);
	week1Monday.setDate(jan4.getDate() - jan4Weekday);
	const monday = new Date(week1Monday);
	monday.setDate(week1Monday.getDate() + (week - 1) * 7);
	return monday;
}

export function getMondayIsoWeek(monday: Date): { year: number; week: number } {
	const thursday = new Date(monday);
	thursday.setDate(monday.getDate() + 3);
	const isoYear = thursday.getFullYear();
	const jan4 = new Date(isoYear, 0, 4);
	const jan4Weekday = (jan4.getDay() + 6) % 7;
	const week1Monday = new Date(jan4);
	week1Monday.setDate(jan4.getDate() - jan4Weekday);
	const diffMs = monday.getTime() - week1Monday.getTime();
	const week = Math.round(diffMs / (7 * 86400000)) + 1;
	return { year: isoYear, week };
}

export function getCurrentIsoWeek(): { year: number; week: number } {
	const today = new Date();
	const weekday = (today.getDay() + 6) % 7;
	const monday = new Date(today);
	monday.setDate(today.getDate() - weekday);
	return getMondayIsoWeek(monday);
}

export function weekLabel(year: number, week: number): string {
	const monday = isoWeekToMonday(year, week);
	const sunday = new Date(monday);
	sunday.setDate(monday.getDate() + 6);
	const mondayMonth = MONTH_ABBR[monday.getMonth()];
	const sundayMonth = MONTH_ABBR[sunday.getMonth()];
	const range =
		monday.getMonth() === sunday.getMonth()
			? `${mondayMonth} ${monday.getDate()}-${sunday.getDate()}`
			: `${mondayMonth} ${monday.getDate()} - ${sundayMonth} ${sunday.getDate()}`;
	return `W${week} ${year} · ${range}`;
}

export function dayLabelsForWeek(year: number, week: number, dayNames: string[]) {
	const monday = isoWeekToMonday(year, week);
	return dayNames.map((name, index) => {
		const date = new Date(monday);
		date.setDate(monday.getDate() + index);
		return { name, date: date.getDate(), month: MONTH_ABBR[date.getMonth()] };
	});
}
