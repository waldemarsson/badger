// YAML frontmatter at the top of a markdown note: the editor's twin of
// src-tauri/src/frontmatter.rs (same rules; both pass tests/fixtures/frontmatter-cases.json).
// Only a block at the very start counts: a `---` line, then lines up to the first `---` or
// `...` line (trailing spaces allowed). Values use a small YAML subset; anything else is ignored.
// Lezer still parses the block as markdown (a rule, a heading): callers skip that span.

export interface Frontmatter {
	/** The lines between the delimiters: [innerFrom, innerTo). */
	innerFrom: number;
	innerTo: number;
	/** The closing delimiter line, without its line ending. */
	closeFrom: number;
	closeTo: number;
	/** Where the body starts (after the closing line's ending). */
	body: number;
}

/** A span of the source: `from` inclusive, `to` exclusive. */
export interface Span {
	from: number;
	to: number;
}

/** The line at `at` without its ending, and where the next line starts. */
function nextLine(src: string, at: number): { text: string; next: number } {
	const nl = src.indexOf('\n', at);
	const end = nl < 0 ? src.length : nl;
	const text = src.slice(at, end);
	return { text: text.endsWith('\r') ? text.slice(0, -1) : text, next: nl < 0 ? src.length : nl + 1 };
}

const isDelimiter = (line: string, closing: boolean) => {
	const t = line.replace(/ +$/, '');
	return t === '---' || (closing && t === '...');
};

/** The frontmatter block at the start of `src`, if it has one. */
export function parseFrontmatter(src: string): Frontmatter | null {
	const first = nextLine(src, 0);
	if (!isDelimiter(first.text, false) || (first.next === src.length && !src.endsWith('\n'))) return null;
	let at = first.next;
	while (at < src.length) {
		const line = nextLine(src, at);
		if (isDelimiter(line.text, true)) {
			return { innerFrom: first.next, innerTo: at, closeFrom: at, closeTo: at + line.text.length, body: line.next };
		}
		at = line.next;
	}
	return null;
}

/** Where the body starts: after the frontmatter block, or 0. */
export function bodyStart(src: string): number {
	return parseFrontmatter(src)?.body ?? 0;
}

const isSpace = (c: string) => /^\s$/u.test(c);

/** Trims whitespace and one pair of matching quotes; keeps the value if non-empty. */
function pushValue(src: string, from: number, to: number, out: Span[]) {
	const raw = src.slice(from, to);
	const t = raw.trim();
	let start = from + (raw.length - raw.trimStart().length);
	let end = start + t.length;
	if (t.length >= 2 && ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'")))) {
		const inner = src.slice(start + 1, end - 1);
		start = start + 1 + (inner.length - inner.trimStart().length);
		end = start + inner.trim().length;
	}
	if (start < end) out.push({ from: start, to: end });
}

function splitInto(src: string, from: number, to: number, split: (c: string) => boolean, out: Span[]) {
	let start = from;
	for (let i = from; i < to; i++) {
		if (split(src[i])) {
			pushValue(src, start, i, out);
			start = i + 1;
		}
	}
	pushValue(src, start, to, out);
}

function values(src: string, key: string, split: (c: string) => boolean): Span[] {
	const fm = parseFrontmatter(src);
	if (!fm) return [];
	const out: Span[] = [];
	let at = fm.innerFrom;
	let inList = false;
	while (at < fm.innerTo) {
		const { text: line, next } = nextLine(src, at);
		const start = at;
		at = next;
		if (inList) {
			const trimmed = line.trimStart();
			if (trimmed.startsWith('-')) {
				const item = trimmed.slice(1);
				if (item === '' || item.startsWith(' ')) {
					const from = start + (line.length - item.length);
					pushValue(src, from, from + item.length, out);
					continue;
				}
			}
			if (trimmed === '' || trimmed.startsWith('#')) continue;
			inList = false;
		}
		const colon = line.indexOf(':');
		if (colon < 0) continue;
		const k = line.slice(0, colon);
		// A key starts in column 0.
		if (k.trimEnd().toLowerCase() !== key || /^[ \t]/.test(k)) continue;
		const valueFrom = start + colon + 1;
		const valueTo = start + line.length;
		const value = src.slice(valueFrom, valueTo).trim();
		if (value === '') inList = true;
		else if (value.startsWith('[') && value.endsWith(']')) {
			const open = src.indexOf('[', valueFrom) + 1;
			const close = src.lastIndexOf(']', valueTo - 1);
			splitInto(src, open, close, (c) => c === ',', out);
		} else if (!value.startsWith('#')) splitInto(src, valueFrom, valueTo, split, out);
	}
	return out;
}

/** Spans of the `tags:` values, a leading `#` excluded. Plain values split on commas and whitespace. */
export function frontmatterTagSpans(src: string): Span[] {
	return values(src, 'tags', (c) => c === ',' || isSpace(c))
		.map((s) => (src[s.from] === '#' ? { from: s.from + 1, to: s.to } : s))
		.filter((s) => s.from < s.to);
}

/** The `aliases:` values, as written. Plain values split on commas. */
export function frontmatterAliases(src: string): string[] {
	return values(src, 'aliases', (c) => c === ',').map((s) => src.slice(s.from, s.to));
}
