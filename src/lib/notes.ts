// Pure helpers for the Notes page: list order and search rows, relative dates, and the
// markdown table/inline parsing the editor's table widget renders from.
import type { NoteHit, NoteSummary } from '#lib/types.js';

// ── JSON notes ────────────────────────────────────────────────────────────────

/** Largest JSON body Rust accepts, in UTF-8 bytes (markdown has no limit). */
export const JSON_MAX_BYTES = 5 * 1024 * 1024;
/** Rust's error for a JSON body over the limit (a save or a switch to JSON). */
export const JSON_TOO_LARGE = 'JSON notes are limited to 5 MB';

/** UTF-8 length of `text` without encoding it. */
export function utf8Bytes(text: string): number {
	let bytes = 0;
	for (let i = 0; i < text.length; i++) {
		const c = text.charCodeAt(i);
		if (c < 0x80) bytes += 1;
		else if (c < 0x800) bytes += 2;
		else if (c >= 0xd800 && c <= 0xdbff && i + 1 < text.length) {
			const next = text.charCodeAt(i + 1);
			if (next >= 0xdc00 && next <= 0xdfff) {
				bytes += 4;
				i++;
			} else bytes += 3;
		} else bytes += 3;
	}
	return bytes;
}

/** A JSON body Rust won't store (possible after a restore, or typed past the limit). */
export function overJsonLimit(kind: string, body: string): boolean {
	// A UTF-16 unit is at most 3 UTF-8 bytes: short texts need no count.
	return kind === 'json' && body.length * 3 > JSON_MAX_BYTES && utf8Bytes(body) > JSON_MAX_BYTES;
}

// ── List ──────────────────────────────────────────────────────────────────────

/** Pinned first, then manual position, then id (same order as list_notes). */
export function compareNotes(a: NoteSummary, b: NoteSummary): number {
	return Number(b.pinned) - Number(a.pinned) || a.position - b.position || a.id - b.id;
}

export function sortNotes(notes: NoteSummary[]): NoteSummary[] {
	return [...notes].sort(compareNotes);
}

/**
 * Renumbers the notes of one folder 0..n in the given order (pinned ones first), like
 * Rust's dense rewrite, and re-sorts the list.
 */
function withFolderOrder(notes: NoteSummary[], order: NoteSummary[]): NoteSummary[] {
	const positions = new Map(order.map((n, i) => [n.id, i]));
	const others = notes.filter((n) => !positions.has(n.id));
	return sortNotes([...others, ...order.map((n, i) => ({ ...n, position: i }))]);
}

/** The notes in `folderId` other than `id`, in list order, as [pinned, unpinned]. */
function folderGroups(notes: NoteSummary[], folderId: number | null, id: number): [NoteSummary[], NoteSummary[]] {
	const rest = sortNotes(notes.filter((n) => n.folder_id === folderId && n.id !== id));
	return [rest.filter((n) => n.pinned), rest.filter((n) => !n.pinned)];
}

/**
 * Mirrors move_note: puts note `id` into `folderId` at `index` within its pin group there
 * (clamped; null: the top of the group) and renumbers that folder.
 */
export function moveNoteLocally(notes: NoteSummary[], id: number, folderId: number | null, index: number | null): NoteSummary[] {
	const note = notes.find((n) => n.id === id);
	if (!note) return notes;
	const [pinned, unpinned] = folderGroups(notes, folderId, id);
	const group = note.pinned ? pinned : unpinned;
	group.splice(Math.max(0, Math.min(index ?? 0, group.length)), 0, { ...note, folder_id: folderId });
	return withFolderOrder(notes, [...pinned, ...unpinned]);
}

/** A run of text, `hit` when search highlighted it. */
export interface Segment {
	text: string;
	hit: boolean;
}

/** A visible list row: the current summary plus how to draw its title and snippet. */
export interface ListRow {
	note: NoteSummary;
	title: Segment[];
	snippet: Segment[] | null; // null: show the excerpt
	/** A semantic-only hit: found by meaning, no keyword match. */
	related: boolean;
}

const HIT_START = '\u0002';
const HIT_END = '\u0003';

/** Splits search_notes' marked text (hits wrapped in \u0002…\u0003) into segments. */
export function splitHighlights(marked: string): Segment[] {
	const segments: Segment[] = [];
	let hit = false;
	let text = '';
	const push = () => {
		if (text) segments.push({ text, hit });
		text = '';
	};
	for (const ch of marked) {
		if (ch === HIT_START || ch === HIT_END) {
			push();
			hit = ch === HIT_START;
		} else {
			text += ch;
		}
	}
	push();
	return segments;
}

/** Rows for a blank query: every note, nothing highlighted. */
export function plainRows(notes: NoteSummary[]): ListRow[] {
	return notes.map((note) => ({ note, title: [{ text: note.title, hit: false }], snippet: null, related: false }));
}

/**
 * Rows for search hits, in hit (relevance) order, drawn from the current summaries so
 * edits since the search show up. Deleted notes drop out; a renamed note loses its
 * title highlight (the marks belong to the old title).
 */
export function searchRows(notes: NoteSummary[], hits: NoteHit[]): ListRow[] {
	const byId = new Map(notes.map((n) => [n.id, n]));
	return hits.flatMap((hit) => {
		const note = byId.get(hit.id);
		if (!note) return [];
		const title = note.title === hit.title ? splitHighlights(hit.title_marked) : [{ text: note.title, hit: false }];
		return [{ note, title, snippet: hit.snippet ? splitHighlights(hit.snippet) : null, related: hit.related === true }];
	});
}

/**
 * Applies a command's summary to the list, mirroring Rust's order. An edit, rename or move
 * result replaces the row in place (edits never reorder). A new note is added where Rust
 * put it (the top of its folder's unpinned notes). A pin change lands on the boundary
 * between the groups (the bottom of the pinned notes, or the top of the unpinned ones)
 * and renumbers the folder like set_note_pinned.
 */
export function upsertNote(notes: NoteSummary[], summary: NoteSummary): NoteSummary[] {
	const index = notes.findIndex((n) => n.id === summary.id);
	if (index === -1) return sortNotes([...notes, summary]);
	if (notes[index].pinned === summary.pinned) return notes.map((n, i) => (i === index ? summary : n));
	const [pinned, unpinned] = folderGroups(notes, summary.folder_id, summary.id);
	return withFolderOrder(
		notes.filter((n) => n.id !== summary.id),
		[...pinned, summary, ...unpinned]
	);
}

/** The id to select after removing `id` from the visible list: the next one, else the previous. */
export function neighborAfterRemoval(ids: number[], id: number): number | null {
	const index = ids.indexOf(id);
	if (index === -1) return null;
	return ids[index + 1] ?? ids[index - 1] ?? null;
}

/** "512 B", "48 KB", "12.4 MB" (1024-based). */
export function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const units = ['KB', 'MB', 'GB'];
	let value = bytes / 1024;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit++;
	}
	return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
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

/** "Oct 3, 14:07" this year, "Dec 24, 2025" for other years, in the user's locale. */
export function formatNoteDate(value: string, now: Date): string {
	const date = parseTimestamp(value);
	if (Number.isNaN(date.getTime())) return '';
	if (date.getFullYear() !== now.getFullYear()) {
		return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
	}
	const day = date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
	return `${day}, ${date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}`;
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
