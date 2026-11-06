// Pure helpers for the Notes page: list order and filter, relative dates, and the
// markdown table/inline parsing the editor's table widget renders from.
import type { NoteSummary } from '#lib/types.js';

// ── List ──────────────────────────────────────────────────────────────────────

/** Pinned first, then most recently updated, then id (same order as list_notes). */
export function sortNotes(notes: NoteSummary[]): NoteSummary[] {
	return [...notes].sort(
		(a, b) =>
			Number(b.pinned) - Number(a.pinned) ||
			b.updated_at.localeCompare(a.updated_at) ||
			a.id - b.id
	);
}

/** Case-insensitive substring match on title + excerpt; a blank query keeps everything. */
export function filterNotes(notes: NoteSummary[], query: string): NoteSummary[] {
	const q = query.trim().toLowerCase();
	if (!q) return notes;
	return notes.filter((n) => n.title.toLowerCase().includes(q) || n.excerpt.toLowerCase().includes(q));
}

/** Replaces the row with the same id (or adds it) and re-sorts. */
export function upsertNote(notes: NoteSummary[], summary: NoteSummary): NoteSummary[] {
	return sortNotes([...notes.filter((n) => n.id !== summary.id), summary]);
}

/** The id to select after removing `id` from the visible list: the next one, else the previous. */
export function neighborAfterRemoval(ids: number[], id: number): number | null {
	const index = ids.indexOf(id);
	if (index === -1) return null;
	return ids[index + 1] ?? ids[index - 1] ?? null;
}

// ── Dates ─────────────────────────────────────────────────────────────────────

/** Parses the stored local `YYYY-MM-DDTHH:MM:SS.mmm` timestamp as local time. */
export function parseTimestamp(value: string): Date {
	const m = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,3}))?/.exec(value);
	if (!m) return new Date(NaN);
	const [y, mo, d, h, mi, s] = m.slice(1, 7).map(Number);
	const ms = m[7] ? Number(m[7].padEnd(3, '0')) : 0;
	return new Date(y, mo - 1, d, h, mi, s, ms);
}

function startOfDay(date: Date): number {
	return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/** "Just now", "5 min ago", "3 h ago", "Yesterday", "Monday", "Oct 3", "Dec 24, 2025". */
export function relativeDate(value: string, now: Date): string {
	const date = parseTimestamp(value);
	if (Number.isNaN(date.getTime())) return '';
	const seconds = (now.getTime() - date.getTime()) / 1000;
	if (seconds < 60) return 'Just now';
	if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
	const days = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);
	if (days === 0) return `${Math.floor(seconds / 3600)} h ago`;
	if (days === 1) return 'Yesterday';
	if (days < 7) return date.toLocaleDateString(undefined, { weekday: 'long' });
	if (date.getFullYear() === now.getFullYear()) {
		return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
	}
	return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

// ── Tables ────────────────────────────────────────────────────────────────────

/** A table cell's trimmed source text and its [from, to) offsets. */
export interface TableCell {
	text: string;
	from: number;
	to: number;
}

export type TableAlign = 'left' | 'center' | 'right' | null;

export interface TableModel {
	header: TableCell[];
	align: TableAlign[];
	rows: TableCell[][]; // each row has exactly header.length cells
}

/** Splits one table row on unescaped pipes; offsets are relative to the line start. */
export function splitTableRow(line: string): TableCell[] {
	const pipes: number[] = [];
	for (let i = 0; i < line.length; i++) {
		if (line[i] === '\\') i++;
		else if (line[i] === '|') pipes.push(i);
	}
	const edges = [-1, ...pipes, line.length];
	let segments: [number, number][] = [];
	for (let k = 0; k < edges.length - 1; k++) segments.push([edges[k] + 1, edges[k + 1]]);

	const lead = line.length - line.trimStart().length;
	const trail = line.trimEnd().length;
	if (pipes.length && pipes[0] === lead) segments = segments.slice(1);
	if (pipes.length && pipes[pipes.length - 1] === trail - 1 && segments.length) segments = segments.slice(0, -1);

	return segments.map(([start, end]) => {
		const raw = line.slice(start, end);
		const text = raw.trim();
		if (!text) return { text: '', from: start, to: start };
		const from = start + (raw.length - raw.trimStart().length);
		return { text, from, to: from + text.length };
	});
}

/** Column alignment from the delimiter row (`:--`, `:-:`, `--:`). */
export function parseTableAlign(line: string): TableAlign[] {
	return splitTableRow(line).map(({ text }) => {
		const left = text.startsWith(':');
		const right = text.length > 1 && text.endsWith(':');
		return left && right ? 'center' : left ? 'left' : right ? 'right' : null;
	});
}

/** Parses a GFM table's source. Rows are padded or truncated to the header width. */
export function parseTable(source: string): TableModel {
	const lines = source.split('\n');
	const starts: number[] = [];
	let offset = 0;
	for (const line of lines) {
		starts.push(offset);
		offset += line.length + 1;
	}
	const cellsAt = (index: number): TableCell[] =>
		splitTableRow(lines[index]).map((c) => ({ text: c.text, from: c.from + starts[index], to: c.to + starts[index] }));

	const header = cellsAt(0);
	const width = header.length;
	const delimiter = lines.length > 1 ? parseTableAlign(lines[1]) : [];
	const align = Array.from({ length: width }, (_, i) => delimiter[i] ?? null);

	const rows: TableCell[][] = [];
	for (let i = 2; i < lines.length; i++) {
		if (!lines[i].trim()) continue;
		const cells = cellsAt(i).slice(0, width);
		const end = starts[i] + lines[i].length;
		while (cells.length < width) cells.push({ text: '', from: end, to: end });
		rows.push(cells);
	}
	return { header, align, rows };
}

// ── Inline markdown (table cells) ─────────────────────────────────────────────

/** A run of text with the inline styles that apply to it. */
export interface InlineSpan {
	text: string;
	bold?: true;
	italic?: true;
	code?: true;
	strike?: true;
	link?: true;
}

type Marks = Omit<InlineSpan, 'text'>;

const ESCAPABLE = /[!-/:-@[-`{-~]/;
const AUTOLINK = /^<([a-zA-Z][a-zA-Z0-9+.-]{1,31}:[^\s<>]*)>/;

function runLength(s: string, i: number, ch: string): number {
	let n = 0;
	while (s[i + n] === ch) n++;
	return n;
}

/** Index of the next backtick run of exactly `length` at or after `from`, or -1. */
function findBacktickRun(s: string, from: number, length: number): number {
	for (let i = from; i < s.length; ) {
		if (s[i] !== '`') {
			i++;
			continue;
		}
		const run = runLength(s, i, '`');
		if (run === length) return i;
		i += run;
	}
	return -1;
}

/** `[label](dest)` starting at `i`: the label and the index after `)`. */
function matchLink(s: string, i: number): { label: string; end: number } | null {
	let depth = 0;
	let close = -1;
	for (let j = i; j < s.length; j++) {
		if (s[j] === '\\') j++;
		else if (s[j] === '[') depth++;
		else if (s[j] === ']' && --depth === 0) {
			close = j;
			break;
		}
	}
	if (close === -1 || s[close + 1] !== '(') return null;
	let parens = 0;
	for (let j = close + 1; j < s.length; j++) {
		if (s[j] === '\\') j++;
		else if (s[j] === '(') parens++;
		else if (s[j] === ')' && --parens === 0) return { label: s.slice(i + 1, close), end: j + 1 };
	}
	return null;
}

const isSpace = (c: string | undefined) => c === undefined || /\s/.test(c);
const isWordChar = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}]/u.test(c);

/** Emphasis/strike opened by the delimiter run at `i`: inner text, styles and end index. */
function matchDelimited(s: string, i: number, ch: string, run: number): { inner: string; marks: Marks; end: number } | null {
	if (ch === '_' && isWordChar(s[i - 1])) return null;
	const sizes = ch === '~' ? (run >= 2 ? [2] : []) : [3, 2, 1].filter((n) => n <= run);
	for (const size of sizes) {
		const start = i + size;
		if (isSpace(s[start])) continue;
		for (let j = start + 1; j < s.length; ) {
			if (s[j] === '\\') {
				j += 2;
				continue;
			}
			if (s[j] === '`') {
				const len = runLength(s, j, '`');
				const close = findBacktickRun(s, j + len, len);
				j = close === -1 ? j + len : close + len;
				continue;
			}
			if (s[j] !== ch) {
				j++;
				continue;
			}
			const len = runLength(s, j, ch);
			const closeAt = j + len - size; // take the closing delimiter from the end of the run
			if (len >= size && !isSpace(s[j - 1]) && !(ch === '_' && isWordChar(s[j + len]))) {
				const marks: Marks =
					ch === '~' ? { strike: true }
					: size === 3 ? { bold: true, italic: true }
					: size === 2 ? { bold: true }
					: { italic: true };
				return { inner: s.slice(start, closeAt), marks, end: closeAt + size };
			}
			j += len;
		}
	}
	return null;
}

function parseInto(s: string, marks: Marks, out: InlineSpan[]): void {
	let buffer = '';
	const flush = () => {
		if (buffer) out.push({ text: buffer, ...marks });
		buffer = '';
	};
	let i = 0;
	while (i < s.length) {
		const ch = s[i];
		if (ch === '\\' && i + 1 < s.length && ESCAPABLE.test(s[i + 1])) {
			buffer += s[i + 1];
			i += 2;
		} else if (ch === '`') {
			const run = runLength(s, i, '`');
			const close = findBacktickRun(s, i + run, run);
			if (close === -1) {
				buffer += s.slice(i, i + run);
				i += run;
				continue;
			}
			let code = s.slice(i + run, close);
			if (code.length >= 2 && code.startsWith(' ') && code.endsWith(' ') && code.trim()) code = code.slice(1, -1);
			flush();
			out.push({ text: code, ...marks, code: true });
			i = close + run;
		} else if (ch === '!' && s[i + 1] === '[' && matchLink(s, i + 1)) {
			// Images are not rendered: keep the whole thing as text.
			const end = matchLink(s, i + 1)!.end;
			buffer += s.slice(i, end);
			i = end;
		} else if (ch === '[' && matchLink(s, i)) {
			const link = matchLink(s, i)!;
			flush();
			parseInto(link.label, { ...marks, link: true }, out);
			i = link.end;
		} else if (ch === '<' && AUTOLINK.test(s.slice(i))) {
			const m = AUTOLINK.exec(s.slice(i))!;
			flush();
			out.push({ text: m[1], ...marks, link: true });
			i += m[0].length;
		} else if (ch === '*' || ch === '_' || ch === '~') {
			const run = runLength(s, i, ch);
			const span = matchDelimited(s, i, ch, run);
			if (span) {
				flush();
				parseInto(span.inner, { ...marks, ...span.marks }, out);
				i = span.end;
			} else {
				buffer += s.slice(i, i + run);
				i += run;
			}
		} else {
			buffer += ch;
			i++;
		}
	}
	flush();
}

const MARK_KEYS = ['bold', 'italic', 'code', 'strike', 'link'] as const;

/** Parses bold/italic/code/strike/link inside a table cell into styled text runs. */
export function parseInline(source: string): InlineSpan[] {
	const spans: InlineSpan[] = [];
	parseInto(source, {}, spans);
	const merged: InlineSpan[] = [];
	for (const span of spans) {
		const last = merged[merged.length - 1];
		if (last && MARK_KEYS.every((k) => last[k] === span[k])) last.text += span.text;
		else merged.push({ ...span });
	}
	return merged;
}
