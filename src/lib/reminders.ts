// Pure reminder helpers: the `remind …` phrase parser (English + Swedish), snooze
// targets, list grouping and short labels. No runtime `#lib/…` imports: node:test
// loads this file directly. Times are naive local wall-clock strings
// (`YYYY-MM-DDTHH:MM:SS.mmm`), matching the Rust side.

export type Repeat = 'none' | 'daily' | 'weekdays' | 'weekly' | 'monthly';
export type ReminderState = 'pending' | 'fired' | 'done';

export interface ParsedReminder {
	text: string;
	remindAt: string;
	repeat: Repeat;
	/** ISO weekdays 1 = Mon … 7 = Sun; only set for `weekly`. */
	weekdays: number[];
	/** 1–31; only set for `monthly`. */
	monthDay: number | null;
}

export interface ParseError {
	error: 'no_time' | 'no_text';
	text: string;
}

export type ParseResult = ParsedReminder | ParseError;

/** The fields of a `Reminder` row these helpers read. */
export interface ReminderLike {
	id: number;
	remind_at: string;
	state: ReminderState;
	updated_at: string;
}

export type SnoozeChoice = '20m' | '1h' | '3h' | 'tomorrow';

export const SNOOZE_CHOICES: { choice: SnoozeChoice; label: string }[] = [
	{ choice: '20m', label: '20 min' },
	{ choice: '1h', label: '1 hour' },
	{ choice: '3h', label: '3 hours' },
	{ choice: 'tomorrow', label: 'Tomorrow 09:00' }
];

// ---------------------------------------------------------------------------
// Timestamps

const pad = (n: number, width = 2) => String(n).padStart(width, '0');

/** `YYYY-MM-DDTHH:MM:SS.mmm` in local time. */
export function toLocalTimestamp(d: Date): string {
	return (
		`${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
		`T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`
	);
}

/** Parses a local timestamp (`T` or space separator, seconds/fraction optional). Invalid input → Invalid Date. */
export function parseLocalTimestamp(s: string): Date {
	const m = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?$/.exec(s.trim());
	if (!m) return new Date(NaN);
	const ms = m[7] ? Number(m[7].slice(0, 3).padEnd(3, '0')) : 0;
	return new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], m[6] ? +m[6] : 0, ms);
}

// ---------------------------------------------------------------------------
// Calendar helpers (local time)

interface Ymd {
	y: number;
	m: number; // 0-based
	d: number;
}

const ymdOf = (d: Date): Ymd => ({ y: d.getFullYear(), m: d.getMonth(), d: d.getDate() });
const addDays = (date: Ymd, days: number): Ymd => ymdOf(new Date(date.y, date.m, date.d + days));
const at = (date: Ymd, h: number, min: number) => new Date(date.y, date.m, date.d, h, min, 0, 0);
const isoWeekday = (date: Ymd) => ((new Date(date.y, date.m, date.d).getDay() + 6) % 7) + 1;
const daysInMonth = (y: number, m: number) => new Date(y, m + 1, 0).getDate();
const sameYmd = (a: Ymd, b: Ymd) => a.y === b.y && a.m === b.m && a.d === b.d;
const ymdKey = (date: Ymd) => date.y * 10000 + date.m * 100 + date.d;

/** Whole-minute copy of `now` (seconds and milliseconds dropped). */
const floorMinute = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getMinutes(), 0, 0);

// ---------------------------------------------------------------------------
// Lexicon

const FULL_DAYS: Record<string, number> = {
	monday: 1,
	tuesday: 2,
	wednesday: 3,
	thursday: 4,
	friday: 5,
	saturday: 6,
	sunday: 7,
	måndag: 1,
	tisdag: 2,
	onsdag: 3,
	torsdag: 4,
	fredag: 5,
	lördag: 6,
	söndag: 7
};

const ABBR_DAYS: Record<string, number> = {
	mon: 1,
	tue: 2,
	tues: 2,
	wed: 3,
	thu: 4,
	thur: 4,
	thurs: 4,
	fri: 5,
	sat: 6,
	sun: 7,
	mån: 1,
	tis: 2,
	ons: 3,
	tor: 4,
	tors: 4,
	fre: 5,
	lör: 6,
	sön: 7
};

const UNITS: Record<string, 'min' | 'hour' | 'day'> = {
	m: 'min',
	min: 'min',
	mins: 'min',
	minute: 'min',
	minutes: 'min',
	minut: 'min',
	minuter: 'min',
	h: 'hour',
	hr: 'hour',
	hrs: 'hour',
	hour: 'hour',
	hours: 'hour',
	t: 'hour',
	tim: 'hour',
	timme: 'hour',
	timmar: 'hour',
	d: 'day',
	day: 'day',
	days: 'day',
	dag: 'day',
	dagar: 'day'
};

const ONE = new Set(['a', 'an', 'one', 'en', 'ett']);
const AT_WORDS = new Set(['at', 'kl', 'klockan', '@']);
const ON_WORDS = new Set(['on', 'på']);
const NEXT_WORDS = new Set(['next', 'nästa']);
const EVERY_WORDS = new Set(['every', 'varje']);
const AND_WORDS = new Set(['and', 'och', '&', '+']);

// ---------------------------------------------------------------------------
// Token matchers

interface Token {
	raw: string;
	/** Lowercased, trailing punctuation removed. */
	w: string;
}

interface Hm {
	h: number;
	m: number;
	/** A bare hour 1–12 (`at 3`, `mon 3`): no minutes, am/pm or leading zero, so it may mean AM or PM. */
	bare?: boolean;
}

function tokenize(input: string): Token[] {
	return input
		.split(/\s+/)
		.filter(Boolean)
		.map((raw) => ({ raw, w: raw.toLowerCase().replace(/[.,;:!?]+$/, '') }));
}

function weekdayOf(w: string | undefined, allowAbbr: boolean): number | null {
	if (w === undefined) return null;
	if (w in FULL_DAYS) return FULL_DAYS[w];
	if (allowAbbr && w in ABBR_DAYS) return ABBR_DAYS[w];
	return null;
}

function hm(h: number, m: number, ampm?: string): Hm | null {
	if (m < 0 || m > 59) return null;
	if (ampm) {
		if (h < 1 || h > 12) return null;
		const pm = ampm.startsWith('p');
		return { h: (h % 12) + (pm ? 12 : 0), m };
	}
	return h >= 0 && h <= 23 ? { h, m } : null;
}

const AMPM = /^(am|pm|a\.m|p\.m)$/;

class Parser {
	readonly toks: Token[];
	readonly used: boolean[];
	/** Indices that end a day/date/repeat phrase; a bare number right after one is an hour. */
	readonly anchors = new Set<number>();

	constructor(input: string) {
		this.toks = tokenize(input);
		this.used = this.toks.map(() => false);
	}

	free(i: number): boolean {
		return i >= 0 && i < this.toks.length && !this.used[i];
	}

	w(i: number): string | undefined {
		return this.free(i) ? this.toks[i].w : undefined;
	}

	take(start: number, len: number, anchor = false) {
		for (let k = start; k < start + len; k++) this.used[k] = true;
		if (anchor) this.anchors.add(start + len - 1);
	}

	/** Time at token `i` → `{h, m, len}`; bare hours only when `allowBare`. */
	timeAt(i: number, allowBare: boolean): (Hm & { len: number }) | null {
		const w = this.w(i);
		if (w === undefined) return null;
		const next = this.w(i + 1);
		const nextAmpm = next !== undefined && AMPM.test(next) ? next : undefined;
		let m = /^(\d{1,2})[:.](\d{2})(am|pm)?$/.exec(w);
		if (m) {
			const ampm = m[3] ?? nextAmpm;
			const t = hm(+m[1], +m[2], ampm);
			return t && { ...t, len: !m[3] && nextAmpm ? 2 : 1 };
		}
		m = /^(\d{1,2})(am|pm)$/.exec(w);
		if (m) {
			const t = hm(+m[1], 0, m[2]);
			return t && { ...t, len: 1 };
		}
		if (/^\d{1,2}$/.test(w)) {
			if (nextAmpm) {
				const t = hm(+w, 0, nextAmpm);
				return t && { ...t, len: 2 };
			}
			if (allowBare) {
				const t = hm(+w, 0);
				return t && { ...t, len: 1, bare: !w.startsWith('0') && t.h >= 1 && t.h <= 12 };
			}
		}
		return null;
	}

	/** Time phrase at `i`, including a leading `at` / `kl` / `klockan`. */
	timePhraseAt(i: number, allowBare: boolean): (Hm & { len: number }) | null {
		const w = this.w(i);
		if (w !== undefined && AT_WORDS.has(w)) {
			const t = this.timeAt(i + 1, true);
			return t && { ...t, len: t.len + 1 };
		}
		return this.timeAt(i, allowBare);
	}

	/** Day of month at `i`: `15th`, `15:e`, or a plain `15` when `allowPlain`. */
	monthDayAt(i: number, allowPlain: boolean): number | null {
		const raw = this.free(i) ? this.toks[i].raw.toLowerCase().replace(/[.,;!?]+$/, '') : undefined;
		if (raw === undefined) return null;
		const m = /^(\d{1,2})(st|nd|rd|th|:e|e)?$/.exec(raw);
		if (!m || (!m[2] && !allowPlain)) return null;
		const day = +m[1];
		return day >= 1 && day <= 31 ? day : null;
	}

	/** `[on] [the|den] 15th` → `{day, len}`. */
	onTheDayAt(i: number): { day: number; len: number } | null {
		let k = i;
		if (ON_WORDS.has(this.w(k) ?? '')) k++;
		const article = this.w(k) === 'the' || this.w(k) === 'den';
		if (article) k++;
		const day = this.monthDayAt(k, article);
		return day === null ? null : { day, len: k + 1 - i };
	}

	/** Weekday list `mon and thu, fri` starting at `i` (abbreviations allowed). */
	weekdayListAt(i: number): { days: number[]; len: number } | null {
		const days: number[] = [];
		let k = i;
		let end = i;
		for (;;) {
			const wd = weekdayOf(this.w(k), true);
			if (wd === null) break;
			days.push(wd);
			end = k + 1;
			k = end;
			if (AND_WORDS.has(this.w(k) ?? '') && weekdayOf(this.w(k + 1), true) !== null) k++;
		}
		return days.length ? { days, len: end - i } : null;
	}
}

interface RepeatSpec {
	repeat: Exclude<Repeat, 'none'>;
	weekdays: number[];
	/** Monthly day; `null` = today's day of month. */
	monthDay: number | null;
}

function matchRepeat(p: Parser): RepeatSpec | null {
	for (let i = 0; i < p.toks.length; i++) {
		const w = p.w(i);
		if (w === undefined) continue;

		if (w === 'daily' || w === 'dagligen') {
			p.take(i, 1, true);
			return { repeat: 'daily', weekdays: [], monthDay: null };
		}

		if (EVERY_WORDS.has(w)) {
			const n = p.w(i + 1);
			if (n === 'day' || n === 'dag') {
				p.take(i, 2, true);
				return { repeat: 'daily', weekdays: [], monthDay: null };
			}
			if (n === 'weekday' || n === 'vardag') {
				p.take(i, 2, true);
				return { repeat: 'weekdays', weekdays: [], monthDay: null };
			}
			if (n === 'month' || n === 'månad') {
				const on = p.onTheDayAt(i + 2);
				p.take(i, 2 + (on?.len ?? 0), true);
				return { repeat: 'monthly', weekdays: [], monthDay: on?.day ?? null };
			}
			if (n === 'week' || n === 'vecka') {
				const hasOn = ON_WORDS.has(p.w(i + 2) ?? '');
				const list = hasOn ? p.weekdayListAt(i + 3) : null;
				p.take(i, 2 + (list ? list.len + 1 : 0), true);
				return { repeat: 'weekly', weekdays: list ? list.days : [], monthDay: null };
			}
			const day = p.monthDayAt(i + 1, false);
			if (day !== null) {
				p.take(i, 2, true);
				return { repeat: 'monthly', weekdays: [], monthDay: day };
			}
			const list = p.weekdayListAt(i + 1);
			if (list) {
				p.take(i, 1 + list.len, true);
				return { repeat: 'weekly', weekdays: list.days, monthDay: null };
			}
		}

		// `den 15 varje månad`, `the 15th of every month`, `on the 15th every month`
		const on = p.onTheDayAt(i);
		if (on) {
			let k = i + on.len;
			if (p.w(k) === 'of') k++;
			if (EVERY_WORDS.has(p.w(k) ?? '') && (p.w(k + 1) === 'month' || p.w(k + 1) === 'månad')) {
				p.take(i, k + 2 - i, true);
				return { repeat: 'monthly', weekdays: [], monthDay: on.day };
			}
		}
	}
	return null;
}

type Relative = { unit: 'min' | 'hour' | 'day'; n: number };

function matchRelative(p: Parser): Relative | null {
	for (let i = 0; i < p.toks.length; i++) {
		const w = p.w(i);
		if (w !== 'in' && w !== 'om') continue;
		const a = p.w(i + 1);
		if (a === undefined) continue;
		const joined = /^(\d+)([a-zåäö]+)$/.exec(a);
		if (joined && joined[2] in UNITS && +joined[1] > 0) {
			p.take(i, 2, UNITS[joined[2]] === 'day');
			return { unit: UNITS[joined[2]], n: +joined[1] };
		}
		const n = /^\d+$/.test(a) ? +a : ONE.has(a) ? 1 : 0;
		const unit = UNITS[p.w(i + 2) ?? ''];
		if (n > 0 && unit) {
			p.take(i, 3, unit === 'day');
			return { unit, n };
		}
	}
	return null;
}

function matchDate(p: Parser, now: Date): Ymd | null {
	const today = ymdOf(now);
	for (let i = 0; i < p.toks.length; i++) {
		const w = p.w(i);
		if (w === undefined) continue;
		let date: Ymd | null = null;
		let m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(w);
		if (m) date = { y: +m[1], m: +m[2] - 1, d: +m[3] };
		m = date ? null : /^(\d{1,2})\/(\d{1,2})(?:\/(\d{4}|\d{2}))?$/.exec(w);
		if (m) {
			const y = m[3] ? (m[3].length === 2 ? 2000 + +m[3] : +m[3]) : today.y;
			date = { y, m: +m[2] - 1, d: +m[1] };
			// Day/month without a year: the next such date.
			if (!m[3] && ymdKey(date) < ymdKey(today)) date = { ...date, y: y + 1 };
		}
		if (!date || date.m < 0 || date.m > 11 || date.d < 1 || date.d > daysInMonth(date.y, date.m)) continue;
		const prev = p.w(i - 1);
		const lead = prev !== undefined && (ON_WORDS.has(prev) || prev === 'den') ? 1 : 0;
		p.take(i - lead, 1 + lead, true);
		return date;
	}
	return null;
}

type DayWord = { kind: 'date'; date: Ymd } | { kind: 'weekday'; wd: number; next: boolean };

function matchDayWord(p: Parser, now: Date): DayWord | null {
	const today = ymdOf(now);
	for (let i = 0; i < p.toks.length; i++) {
		const w = p.w(i);
		if (w === undefined) continue;
		const n = p.w(i + 1);
		if (w === 'today' || w === 'idag') {
			p.take(i, 1, true);
			return { kind: 'date', date: today };
		}
		if (w === 'tomorrow' || w === 'imorgon' || w === 'tmrw') {
			p.take(i, 1, true);
			return { kind: 'date', date: addDays(today, 1) };
		}
		if (w === 'i' && (n === 'dag' || n === 'morgon')) {
			p.take(i, 2, true);
			return { kind: 'date', date: n === 'dag' ? today : addDays(today, 1) };
		}
		if (NEXT_WORDS.has(w) || ON_WORDS.has(w)) {
			const wd = weekdayOf(n, true);
			if (wd !== null) {
				p.take(i, 2, true);
				return { kind: 'weekday', wd, next: NEXT_WORDS.has(w) };
			}
		}
		const full = weekdayOf(w, false);
		if (full !== null) {
			p.take(i, 1, true);
			return { kind: 'weekday', wd: full, next: false };
		}
		// Abbreviations stand alone only directly before a time (`fri 16:00`, `ons kl 9`).
		const abbr = weekdayOf(w, true);
		if (abbr !== null && p.timePhraseAt(i + 1, true)) {
			p.take(i, 1, true);
			return { kind: 'weekday', wd: abbr, next: false };
		}
	}
	return null;
}

function matchTime(p: Parser): Hm | null {
	for (let i = 0; i < p.toks.length; i++) {
		const t = p.timePhraseAt(i, p.anchors.has(i - 1) && p.used[i - 1]);
		if (t) {
			p.take(i, t.len);
			return t;
		}
	}
	return null;
}

type BareRule = 'asTyped' | 'dayPm' | 'next';

/**
 * Hour for a bare `1`–`12`. A bare `12` is always noon. After a repeat phrase (`every weekday
 * at 7`, `varje dag kl 3`) it is taken as typed. With a day (`mon 3`, `tomorrow 3`, a date,
 * `in 2 days`) 1–7 mean PM and 8–11 stay as typed. Without one (an explicit `today` counts as
 * none) it is the next occurrence of that hour (`at 3` at 14:30 → 15:00, `at 9` at 14:30 →
 * 21:00, `at 9` at 08:00 → 09:00).
 */
function bareHour(h: number, rule: BareRule, now: Date): number {
	if (h === 12 || rule === 'asTyped') return h;
	if (rule === 'dayPm') return h <= 7 ? h + 12 : h;
	const today = ymdOf(now);
	const candidates = [h % 12, (h % 12) + 12];
	return candidates.find((c) => at(today, c, 0) > now) ?? candidates[0];
}

function extractText(p: Parser): string {
	const words = p.toks.filter((_, i) => !p.used[i]).map((t) => t.raw);
	const lead = (k: number) => words[k]?.toLowerCase().replace(/[.,;:!?]+$/, '');
	let k = 0;
	if (lead(0) === 'remind' || lead(0) === 'påminn') {
		k = 1;
		if (lead(1) === 'me' || lead(1) === 'mig') k = 2;
	}
	if (lead(k) === 'to' || lead(k) === 'att') k++;
	return words
		.slice(k)
		.join(' ')
		.replace(/^[\s,;:–-]+|[\s,;:–-]+$/g, '');
}

function matchesRule(date: Ymd, spec: RepeatSpec): boolean {
	switch (spec.repeat) {
		case 'daily':
			return true;
		case 'weekdays':
			return isoWeekday(date) <= 5;
		case 'weekly':
			return spec.weekdays.includes(isoWeekday(date));
		case 'monthly':
			return date.d === Math.min(spec.monthDay ?? 1, daysInMonth(date.y, date.m));
	}
}

/**
 * Parses `remind …` input into a reminder. Recognised phrases are removed from the
 * text; a leading `remind [me]` / `påminn [mig]` and `to` / `att` are dropped too.
 */
export function parseReminder(input: string, now: Date): ParseResult {
	const p = new Parser(input);
	const today = ymdOf(now);

	const repeat = matchRepeat(p);
	const relative = matchRelative(p);

	let date: Ymd | null;
	let time: Hm | null;
	let weekday: { wd: number; next: boolean } | null = null;
	// `today` / `idag` names a day but keeps the next-occurrence rule for bare hours.
	let todayWord = false;

	if (relative && relative.unit !== 'day') {
		// `in 20 min` is complete on its own; nothing else is parsed as a time.
		const minutes = relative.unit === 'min' ? relative.n : relative.n * 60;
		const target = floorMinute(new Date(now.getTime() + minutes * 60_000));
		date = ymdOf(target);
		time = { h: target.getHours(), m: target.getMinutes() };
	} else {
		if (relative) date = addDays(today, relative.n);
		else {
			date = matchDate(p, now);
			if (!date) {
				const dw = matchDayWord(p, now);
				if (dw?.kind === 'date') {
					date = dw.date;
					todayWord = ymdKey(date) === ymdKey(today);
				}
				else if (dw) weekday = dw;
			}
		}
		time = matchTime(p);
		if (time?.bare) {
			const rule: BareRule = repeat ? 'asTyped' : relative || (date && !todayWord) || weekday ? 'dayPm' : 'next';
			time = { h: bareHour(time.h, rule, now), m: time.m };
		}
		// `in N days` keeps the current time of day.
		if (!time && relative) time = { h: now.getHours(), m: now.getMinutes() };
	}

	const text = extractText(p);
	if (!repeat && !date && !weekday && !time) return { error: 'no_time', text };
	if (!text) return { error: 'no_text', text };

	const tod: Hm = time ?? { h: 9, m: 0 };

	if (weekday) {
		let days = (weekday.wd - isoWeekday(today) + 7) % 7;
		if (days === 0 && (weekday.next || at(today, tod.h, tod.m) <= now)) days = 7;
		date = addDays(today, days);
	}

	if (repeat) {
		const spec: RepeatSpec = {
			...repeat,
			weekdays: repeat.repeat === 'weekly' && !repeat.weekdays.length ? [isoWeekday(date ?? today)] : repeat.weekdays,
			monthDay: repeat.repeat === 'monthly' ? (repeat.monthDay ?? (date ?? today).d) : null
		};
		// First occurrence: the first matching day on/after the start whose time is still ahead.
		let day = date ?? today;
		for (let k = 0; k < 800 && !(matchesRule(day, spec) && at(day, tod.h, tod.m) > now); k++) day = addDays(day, 1);
		return {
			text,
			remindAt: toLocalTimestamp(at(day, tod.h, tod.m)),
			repeat: spec.repeat,
			weekdays: spec.repeat === 'weekly' ? [...new Set(spec.weekdays)].sort((a, b) => a - b) : [],
			monthDay: spec.monthDay
		};
	}

	// A time without a date: today, or tomorrow once it has passed.
	if (!date) date = at(today, tod.h, tod.m) > now ? today : addDays(today, 1);

	return { text, remindAt: toLocalTimestamp(at(date, tod.h, tod.m)), repeat: 'none', weekdays: [], monthDay: null };
}

// ---------------------------------------------------------------------------
// Snooze

/** Local timestamp a snooze choice moves the reminder to. */
export function snoozeUntil(choice: SnoozeChoice, now: Date): string {
	if (choice === 'tomorrow') return toLocalTimestamp(at(addDays(ymdOf(now), 1), 9, 0));
	const minutes = choice === '20m' ? 20 : choice === '1h' ? 60 : 180;
	return toLocalTimestamp(floorMinute(new Date(now.getTime() + minutes * 60_000)));
}

// ---------------------------------------------------------------------------
// Grouping and labels

export interface ReminderGroups<T> {
	/** Fired, not yet done or snoozed. */
	overdue: T[];
	today: T[];
	tomorrow: T[];
	later: T[];
	done: T[];
}

/**
 * Buckets reminders for the list. Pending rows are bucketed by the day of
 * `remind_at`; a pending row already past (the scheduler hasn't ticked yet) stays in
 * `today`. Active buckets sort by time, done by `updated_at` (latest first).
 */
export function groupReminders<T extends ReminderLike>(list: readonly T[], now: Date): ReminderGroups<T> {
	const groups: ReminderGroups<T> = { overdue: [], today: [], tomorrow: [], later: [], done: [] };
	const todayKey = toLocalTimestamp(at(ymdOf(now), 0, 0)).slice(0, 10);
	const tomorrowKey = toLocalTimestamp(at(addDays(ymdOf(now), 1), 0, 0)).slice(0, 10);
	for (const r of list) {
		if (r.state === 'done') groups.done.push(r);
		else if (r.state === 'fired') groups.overdue.push(r);
		else {
			const day = r.remind_at.slice(0, 10);
			if (day <= todayKey) groups.today.push(r);
			else if (day === tomorrowKey) groups.tomorrow.push(r);
			else groups.later.push(r);
		}
	}
	const byTime = (a: T, b: T) => a.remind_at.localeCompare(b.remind_at) || a.id - b.id;
	groups.overdue.sort(byTime);
	groups.today.sort(byTime);
	groups.tomorrow.sort(byTime);
	groups.later.sort(byTime);
	groups.done.sort((a, b) => b.updated_at.localeCompare(a.updated_at) || b.id - a.id);
	return groups;
}

const DAY_NAMES = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/**
 * Short label relative to `now`: `14:52`, `Tomorrow 09:00`, `Yesterday 09:00`,
 * `Fri 16:00` (within the coming week), `20 Nov 09:00`, `20 Nov 2027 09:00`.
 */
export function formatRemindAt(remindAt: string, now: Date): string {
	const d = parseLocalTimestamp(remindAt);
	if (Number.isNaN(d.getTime())) return remindAt;
	const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
	const day = ymdOf(d);
	const today = ymdOf(now);
	if (sameYmd(day, today)) return time;
	if (sameYmd(day, addDays(today, 1))) return `Tomorrow ${time}`;
	if (sameYmd(day, addDays(today, -1))) return `Yesterday ${time}`;
	const ahead = Math.round((at(day, 12, 0).getTime() - at(today, 12, 0).getTime()) / 86_400_000);
	if (ahead > 1 && ahead < 7) return `${DAY_NAMES[isoWeekday(day) - 1]} ${time}`;
	const year = day.y === today.y ? '' : ` ${day.y}`;
	return `${day.d} ${MONTH_NAMES[day.m]}${year} ${time}`;
}

/** `1st`, `2nd`, `3rd`, `11th`, `21st` … */
export function ordinal(n: number): string {
	const teen = n % 100 >= 11 && n % 100 <= 13;
	const suffix = teen ? 'th' : ({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[n % 10] ?? 'th';
	return `${n}${suffix}`;
}

/** `daily`, `weekdays`, `Mon, Thu`, `monthly on the 15th`; `''` for one-offs. Accepts rows and parse results. */
export function formatRepeat(r: {
	repeat: Repeat;
	weekdays: readonly number[];
	month_day?: number | null;
	monthDay?: number | null;
}): string {
	switch (r.repeat) {
		case 'none':
			return '';
		case 'daily':
			return 'daily';
		case 'weekdays':
			return 'weekdays';
		case 'weekly':
			return [...new Set(r.weekdays)]
				.filter((d) => d >= 1 && d <= 7)
				.sort((a, b) => a - b)
				.map((d) => DAY_NAMES[d - 1])
				.join(', ');
		case 'monthly': {
			const day = r.month_day ?? r.monthDay;
			return day ? `monthly on the ${ordinal(day)}` : 'monthly';
		}
	}
}

// ---------------------------------------------------------------------------
// Form and UI helpers

export type QuickChoice = '20m' | '1h' | 'tomorrow' | 'monday';

/** Quick-pick chips in the reminder form. */
export const QUICK_TIMES: { choice: QuickChoice; label: string }[] = [
	{ choice: '20m', label: '20 min' },
	{ choice: '1h', label: '1 h' },
	{ choice: 'tomorrow', label: 'Tomorrow 09:00' },
	{ choice: 'monday', label: 'Mon 09:00' }
];

/** Local timestamp for a quick-pick chip; `monday` is the coming Monday (never today) at 09:00. */
export function quickTime(choice: QuickChoice, now: Date): string {
	if (choice !== 'monday') return snoozeUntil(choice, now);
	const today = ymdOf(now);
	return toLocalTimestamp(at(addDays(today, 8 - isoWeekday(today)), 9, 0));
}

/** `{date: 'YYYY-MM-DD', time: 'HH:MM'}` of a local timestamp (for date/time inputs). */
export function splitTimestamp(ts: string): { date: string; time: string } {
	return { date: ts.slice(0, 10), time: ts.slice(11, 16) };
}

/**
 * Date/time the edit form starts from. One-off rows show `remind_at`; recurring rows show
 * their `time_of_day` anchor (not a snoozed occurrence's time) on the next matching day whose
 * anchor time is still ahead, so a rule-only change doesn't adopt the snoozed time.
 */
export function formStart(
	r: { remind_at: string; time_of_day: string; repeat: Repeat; weekdays: readonly number[]; month_day: number | null },
	now: Date
): { date: string; time: string } {
	const m = /^(\d{2}):(\d{2})/.exec(r.time_of_day);
	if (r.repeat === 'none' || !m || +m[1] > 23 || +m[2] > 59 || (r.repeat === 'weekly' && !r.weekdays.length)) return splitTimestamp(r.remind_at);
	const spec: RepeatSpec = { repeat: r.repeat, weekdays: [...r.weekdays], monthDay: r.month_day };
	let day = ymdOf(now);
	for (let k = 0; k < 800 && !(matchesRule(day, spec) && at(day, +m[1], +m[2]) > now); k++) day = addDays(day, 1);
	return splitTimestamp(toLocalTimestamp(at(day, +m[1], +m[2])));
}

/** True when the local timestamp is not after `now`. */
export function isPast(remindAt: string, now: Date): boolean {
	const d = parseLocalTimestamp(remindAt);
	return !Number.isNaN(d.getTime()) && d <= now;
}

/** Fired and pending reminders linked to a todo, fired first, then by time. */
export function activeForTodo<T extends ReminderLike & { todo_id: number | null }>(list: readonly T[], todoId: number): T[] {
	return list
		.filter((r) => r.todo_id === todoId && r.state !== 'done')
		.sort((a, b) => Number(b.state === 'fired') - Number(a.state === 'fired') || a.remind_at.localeCompare(b.remind_at) || a.id - b.id);
}

/** Per todo, the reminder its card bell shows: a fired one, else the next pending one. */
export function todoBells<T extends ReminderLike & { todo_id: number | null }>(list: readonly T[]): Map<number, T> {
	const bells = new Map<number, T>();
	for (const r of list) {
		if (r.todo_id === null || r.state === 'done') continue;
		const cur = bells.get(r.todo_id);
		const better =
			!cur ||
			(r.state === 'fired' && cur.state !== 'fired') ||
			(r.state === cur.state && r.remind_at.localeCompare(cur.remind_at) < 0);
		if (better) bells.set(r.todo_id, r);
	}
	return bells;
}

/** Identity of a toast: one per fired occurrence, so a re-fired recurring reminder shows again. */
export function toastKey(r: { id: number; remind_at: string }): string {
	return `${r.id}@${r.remind_at}`;
}

/** Fired reminders not dismissed this session, oldest first: the first `max` show, the rest count as "+N more". */
export function toastStack<T extends ReminderLike>(list: readonly T[], dismissed: ReadonlySet<string>, max = 3): { shown: T[]; more: number } {
	const fired = list
		.filter((r) => r.state === 'fired' && !dismissed.has(toastKey(r)))
		.sort((a, b) => a.remind_at.localeCompare(b.remind_at) || a.id - b.id);
	return { shown: fired.slice(0, max), more: Math.max(0, fired.length - max) };
}
