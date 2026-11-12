// `#tags` and `[[wikilinks]]` in note markdown (Obsidian syntax): the editor's twin of
// src-tauri/src/note_links.rs. Both pass tests/fixtures/note-links-cases.json.
// Needs no DOM (testable under node:test); the editor wiring lives in src/lib/editor/.
import type { SyntaxNode, Tree } from '@lezer/common';
import { GFM, parser } from '@lezer/markdown';

export type RefKind = 'tag' | 'link';

/** A tag or wikilink. Offsets are string indices into the scanned text. */
export interface NoteRef {
	kind: RefKind;
	/** Normalized target: the tag without `#`, or the trimmed title; lowercased. */
	target: string;
	/** The whole `#tag` or `[[…]]`. */
	from: number;
	to: number;
	/** The tag after `#`, or the trimmed title inside the brackets. */
	targetFrom: number;
	targetTo: number;
	/** Trimmed wikilink alias; null for tags and empty aliases. */
	alias: string | null;
}

const tagCharRe = /^[\p{Alphabetic}\p{N}_\-/]$/u;
const whitespaceRe = /^\p{White_Space}$/u;
const numericRe = /^\p{N}+$/u;

/** Letters and digits (any script), `_`, `-` and `/` (nesting). */
export function isTagChar(c: string): boolean {
	return tagCharRe.test(c);
}

const isWhitespace = (c: string) => whitespaceRe.test(c);

const markdownParser = parser.configure(GFM);

/** Never scanned: code, autolinks, raw HTML, reference definitions, markup characters. */
const blockedNodes = new Set([
	'InlineCode',
	'FencedCode',
	'CodeBlock',
	'Autolink',
	'HTMLTag',
	'HTMLBlock',
	'Comment',
	'CommentBlock',
	'ProcessingInstruction',
	'LinkReference',
	'HorizontalRule',
	'HardBreak',
	'HeaderMark',
	'QuoteMark',
	'ListMark',
	'EmphasisMark',
	'StrikethroughMark',
	'TaskMarker',
	'TableDelimiter'
]);

const normalizeLabel = (label: string) => label.replace(/\s+/g, ' ').trim().toLowerCase();

/** Blocks that can hold a reference definition: the parser only adds them to these. */
const labelContainers = new Set(['Document', 'Blockquote', 'BulletList', 'OrderedList', 'ListItem']);

/** Reference labels defined in the document (`[label]: url`). Skips the inside of every other block. */
function definedLabels(src: string, tree: Tree): Set<string> {
	const labels = new Set<string>();
	// A definition always has `]:` (no space allowed between them); most notes have none.
	if (!src.includes(']:')) return labels;
	tree.iterate({
		enter(node) {
			if (labelContainers.has(node.name)) return;
			if (node.name === 'LinkReference') {
				const label = node.node.getChild('LinkLabel');
				if (label) labels.add(normalizeLabel(src.slice(label.from + 1, label.to - 1)));
			}
			return false;
		}
	});
	return labels;
}

/**
 * Is this Link/Image node a real link? Lezer parses every `[text]` as a Link, but
 * markdown (and pulldown-cmark on the Rust side) only links inline links and references
 * to defined labels; anything else is plain text (which is what makes `[[Foo]]` text).
 */
function isRealLink(src: string, node: SyntaxNode, labels: Set<string>): boolean {
	if (node.getChild('URL')) return true;
	const marks = node.getChildren('LinkMark');
	if (marks.some((m) => src.slice(m.from, m.to) === '(')) return true;
	const label = node.getChild('LinkLabel');
	if (label) return labels.has(normalizeLabel(src.slice(label.from + 1, label.to - 1)));
	const open = marks[0];
	const close = marks.find((m) => src.slice(m.from, m.to) === ']');
	if (!open || !close) return false;
	return labels.has(normalizeLabel(src.slice(open.to, close.from)));
}

/** A span of the scanned text: `from` inclusive, `to` exclusive. */
export interface TextRange {
	from: number;
	to: number;
}

/** `ranges` widened to whole lines (refs never cross a line), sorted and merged. */
function wholeLines(src: string, ranges: readonly TextRange[]): TextRange[] {
	const spans = ranges
		.map(({ from, to }) => {
			const at = Math.max(0, Math.min(from, src.length));
			const start = at === 0 ? 0 : src.lastIndexOf('\n', at - 1) + 1;
			const end = src.indexOf('\n', Math.max(start, Math.min(to, src.length)));
			return { from: start, to: end < 0 ? src.length : end };
		})
		.sort((a, b) => a.from - b.from);
	const out: TextRange[] = [];
	for (const span of spans) {
		const last = out[out.length - 1];
		if (last && span.from <= last.to + 1) last.to = Math.max(last.to, span.to);
		else out.push({ ...span });
	}
	return out;
}

/**
 * All tags and wikilinks in `src`, in source order. `tree` defaults to a GFM parse.
 * With `ranges`, only the lines they touch are walked and scanned: the result is the full
 * result's refs on those lines (the editor passes its visible ranges).
 */
export function extractRefs(src: string, tree: Tree = markdownParser.parse(src), ranges?: readonly TextRange[]): NoteRef[] {
	const labels = definedLabels(src, tree);
	const out: NoteRef[] = [];
	const scanSegment = (from: number, to: number) => {
		// Lines are separate runs, as in the Rust extractor.
		let start = from;
		for (let i = from; i <= to; i++) {
			if (i === to || src[i] === '\n' || src[i] === '\r') {
				if (i > start) scanRun(src, start, i, out);
				start = i + 1;
			}
		}
	};
	for (const span of ranges ? wholeLines(src, ranges) : [{ from: 0, to: src.length }]) {
		const blocked: [number, number][] = [];
		tree.iterate({
			from: span.from,
			to: span.to,
			enter(node) {
				if (blockedNodes.has(node.name)) {
					blocked.push([node.from, node.to]);
					return false;
				}
				if ((node.name === 'Link' || node.name === 'Image') && isRealLink(src, node.node, labels)) {
					blocked.push([node.from, node.to]);
					return false;
				}
			}
		});
		blocked.sort((a, b) => a[0] - b[0]);

		let pos = span.from;
		for (const [from, to] of blocked) {
			if (from > pos) scanSegment(pos, Math.min(from, span.to));
			pos = Math.max(pos, to);
			if (pos >= span.to) break;
		}
		if (pos < span.to) scanSegment(pos, span.to);
	}
	return out;
}

/** Is the character before `pos` a backslash (an escaped `#` or `[`)? */
const escaped = (src: string, pos: number) => pos > 0 && src[pos - 1] === '\\';

/** Scans `src[from, to)` for tags and wikilinks (same rules as `scan_run` in Rust). */
export function scanRun(src: string, from: number, to: number, out: NoteRef[] = []): NoteRef[] {
	let i = from;
	let prev: string | null = null;
	while (i < to) {
		const c = String.fromCodePoint(src.codePointAt(i)!);
		if (prev === null || isWhitespace(prev)) {
			// A bare URL (`scheme://…`) is one token: no tags or links inside it.
			let end = i;
			while (end < to) {
				const ch = String.fromCodePoint(src.codePointAt(end)!);
				if (isWhitespace(ch)) break;
				end += ch.length;
			}
			const token = src.slice(i, end);
			if (token.includes('://')) {
				prev = [...token].pop() ?? prev;
				i = end;
				continue;
			}
		}
		if (src.startsWith('[[', i) && prev !== '!' && !escaped(src, i)) {
			const ref = parseWikilink(src, i, to);
			if (ref) {
				out.push(ref);
				i = ref.to;
				prev = ']';
				continue;
			}
		}
		const boundary = prev === null || isWhitespace(prev) || prev === '(';
		if (c === '#' && boundary && !escaped(src, i)) {
			const ref = parseTag(src, i, to);
			if (ref) {
				out.push(ref);
				i = ref.to;
				prev = [...src.slice(ref.targetFrom, ref.to)].pop()!;
				continue;
			}
		}
		prev = c;
		i += c.length;
	}
	return out;
}

/** Parses `[[Title]]` / `[[Title|alias]]` at `at` (`end` bounds the run). */
function parseWikilink(src: string, at: number, end: number): NoteRef | null {
	const close = src.indexOf(']]', at + 2);
	if (close < 0 || close + 2 > end) return null;
	const inner = src.slice(at + 2, close);
	if (/[[\]#\n\r]/.test(inner)) return null;
	let titlePart = inner;
	let alias: string | null = null;
	const pipe = inner.indexOf('|');
	if (pipe >= 0) {
		alias = inner.slice(pipe + 1);
		if (alias.includes('|')) return null;
		titlePart = inner.slice(0, pipe);
		// In a table the separator is written `\|`.
		if (titlePart.endsWith('\\')) titlePart = titlePart.slice(0, -1);
	}
	const title = titlePart.trim();
	if (!title) return null;
	const titleFrom = at + 2 + (titlePart.length - titlePart.trimStart().length);
	const trimmedAlias = alias?.trim() || null;
	return {
		kind: 'link',
		target: title.toLowerCase(),
		from: at,
		to: close + 2,
		targetFrom: titleFrom,
		targetTo: titleFrom + title.length,
		alias: trimmedAlias
	};
}

/** Length of the tag body (tag chars) starting at `from`, before trimming. */
function tagBodyEnd(src: string, from: number, end: number): number {
	let i = from;
	while (i < end) {
		const ch = String.fromCodePoint(src.codePointAt(i)!);
		if (!isTagChar(ch)) break;
		i += ch.length;
	}
	return i;
}

/** Parses `#tag` at `at`: no trailing `/`, no leading `/`, not digits only. */
function parseTag(src: string, at: number, end: number): NoteRef | null {
	const body = src.slice(at + 1, tagBodyEnd(src, at + 1, end)).replace(/\/+$/, '');
	if (!body || body.startsWith('/') || numericRe.test(body)) return null;
	return {
		kind: 'tag',
		target: body.toLowerCase(),
		from: at,
		to: at + 1 + body.length,
		targetFrom: at + 1,
		targetTo: at + 1 + body.length,
		alias: null
	};
}

/** The normalized tag when `term` (a search term) is a `#tag`, e.g. `#Acme,` → `acme`. */
export function tagTerm(term: string): string | null {
	if (!term.startsWith('#')) return null;
	return parseTag(term, 0, term.length)?.target ?? null;
}

/** Splits a search query like `search_notes` does: `#tag` terms and the remaining text. */
export function splitSearchQuery(query: string): { tags: string[]; text: string } {
	const tags: string[] = [];
	const text: string[] = [];
	for (const term of query.split(/\s+/).filter(Boolean)) {
		const tag = tagTerm(term);
		// Tags are normalized (lowercased), so `#Acme #acme` is one filter.
		if (tag) {
			if (!tags.includes(tag)) tags.push(tag);
		} else text.push(term);
	}
	return { tags, text: text.join(' ') };
}

/** Does `tag` (normalized) match `filter` (normalized), itself or nested below it? */
export function tagMatches(tag: string, filter: string): boolean {
	return tag === filter || tag.startsWith(`${filter}/`);
}

/**
 * Tag slug of a customer or project name: lowercased, every run of characters that
 * can't appear in a tag segment and any `-` becomes one `-` (`Acme - Corp` →
 * `acme-corp`), leading/trailing `-` dropped. Null when nothing usable or digits only remain.
 */
export function slug(name: string): string | null {
	let out = '';
	let gap = false;
	for (const c of name) {
		if (c !== '/' && c !== '-' && isTagChar(c)) {
			if (gap && out) out += '-';
			gap = false;
			out += c;
		} else {
			gap = true;
		}
	}
	out = out.toLowerCase();
	return out && !numericRe.test(out) ? out : null;
}

/** `#issue/<id>` / `#todo/<id>` (normalized tag) → the entity it chips. */
export function entityTag(tag: string): { kind: 'issue' | 'todo'; id: number } | null {
	const m = /^(issue|todo)\/(\d+)$/.exec(tag);
	if (!m) return null;
	const id = Number(m[2]);
	return Number.isSafeInteger(id) ? { kind: m[1] as 'issue' | 'todo', id } : null;
}

// ── Autocomplete triggers ────────────────────────────────────────────────────

/**
 * True when `text` ends inside an inline code span. Same as `insideInlineCode` in
 * slash-menu.ts, copied because node:test can't follow `#lib/*.js` value imports.
 */
function insideInlineCode(text: string): boolean {
	let open = 0;
	for (let i = 0; i < text.length; ) {
		if (text[i] === '\\' && !open) {
			i += 2;
			continue;
		}
		if (text[i] !== '`') {
			i++;
			continue;
		}
		let run = 0;
		while (text[i] === '`') {
			run++;
			i++;
		}
		if (!open) open = run;
		else if (run === open) open = 0;
	}
	return open > 0;
}

export interface LinkTrigger {
	/** Offset of the `#` or `[[` in the text before the cursor. */
	from: number;
	/** What was typed after it (filters the suggestions). */
	query: string;
}

const tagTriggerRe = /(^|[\p{White_Space}(])#([\p{Alphabetic}\p{N}_\-/]+)$/u;

/**
 * Should `#` completion open for the text before the cursor on its line? Only for `#` +
 * at least one tag character at a tag boundary (line start, whitespace or `(`), so never
 * for a heading `# `, `##`, `a#b`, URL fragments or inside inline code.
 */
export function tagTrigger(lineBefore: string): LinkTrigger | null {
	const match = tagTriggerRe.exec(lineBefore);
	if (!match) return null;
	const from = match.index + match[1].length;
	if (insideInlineCode(lineBefore.slice(0, from))) return null;
	const tokenStart = lineBefore.slice(0, from).search(/\S*$/u);
	if (lineBefore.slice(tokenStart, from).includes('://')) return null;
	return { from, query: match[2] };
}

const wikilinkTriggerRe = /\[\[([^[\]|#\n]*)$/;

/** Should `[[` completion open? For `[[` + title text (no `]`, `|`, `#`), not `![[`/`\[[`. */
export function wikilinkTrigger(lineBefore: string): LinkTrigger | null {
	const match = wikilinkTriggerRe.exec(lineBefore);
	if (!match) return null;
	const from = match.index;
	const before = lineBefore[from - 1];
	if (before === '!' || before === '\\' || before === '[') return null;
	if (insideInlineCode(lineBefore.slice(0, from))) return null;
	return { from, query: match[1] };
}

/** Text replacing `#query`. */
export function tagCompletion(tag: string): string {
	return `#${tag}`;
}

/**
 * Text replacing `[[query`, plus how many characters right after the cursor to replace
 * too (a `]]` or `]` that is already there), so the link is never closed twice.
 */
export function wikilinkCompletion(title: string, after: string): { insert: string; consumeAfter: number } {
	const consumeAfter = after.startsWith(']]') ? 2 : after.startsWith(']') ? 1 : 0;
	return { insert: `[[${title}]]`, consumeAfter };
}

// ── Autocomplete options ─────────────────────────────────────────────────────

/** Does `q` start some word of `text` (words split on whitespace and `/ - _ — (`)? */
function hasWordStart(text: string, q: string): boolean {
	for (let at = text.indexOf(q); at >= 0; at = text.indexOf(q, at + 1)) {
		if (at === 0 || /[\s/\-_—(]/u.test(text[at - 1])) return true;
	}
	return false;
}

/** A `#` completion entry: what `list_tag_suggestions` returns (src/lib/types.ts). */
export interface TagOption {
	kind: 'tag' | 'customer' | 'project' | 'issue' | 'todo';
	tag: string;
	label: string;
	detail: string | null;
	status: string | null;
}

/**
 * `#` suggestions for `query` (what follows the `#`), best first: the exact tag, then tag
 * prefix, then a word of the tag or label starting with it, then substrings; ties keep the input order (tags in
 * use, customers, projects, issues, todos). Issues and todos match on their title too.
 */
export function filterTagOptions<T extends TagOption>(options: T[], query: string, limit = 50): T[] {
	const q = query.toLowerCase();
	const ranked: { option: T; rank: number; index: number }[] = [];
	options.forEach((option, index) => {
		const tag = option.tag.toLowerCase();
		const label = option.label.toLowerCase();
		let rank: number;
		if (tag === q) rank = 0;
		else if (tag.startsWith(q)) rank = 1;
		else if (hasWordStart(tag, q) || hasWordStart(label, q)) rank = 2;
		else if (tag.includes(q) || label.includes(q)) rank = 3;
		else return;
		ranked.push({ option, rank, index });
	});
	ranked.sort((a, b) => a.rank - b.rank || a.index - b.index);
	return ranked.slice(0, limit).map((r) => r.option);
}

/**
 * Which `#` menu entry to preselect: none (-1) when the typed tag already is the top option
 * (case-insensitive), so Enter keeps its normal meaning; otherwise the top one (0).
 */
export function tagPreselect(query: string, topTag: string | undefined): 0 | -1 {
	return topTag !== undefined && query.toLowerCase() === topTag.toLowerCase() ? -1 : 0;
}

/**
 * `[[` suggestions for `query`: matching note titles (prefix, word start, substring; then
 * alphabetical), and the trimmed query as `create` when no title equals it (case-insensitive).
 */
export function wikilinkOptions<T extends { title: string }>(
	notes: T[],
	query: string,
	limit = 50
): { notes: T[]; create: string | null } {
	const typed = query.trim();
	const q = typed.toLowerCase();
	const ranked: { note: T; rank: number }[] = [];
	for (const note of notes) {
		const title = note.title.toLowerCase();
		const rank = !q || title.startsWith(q) ? 0 : hasWordStart(title, q) ? 1 : title.includes(q) ? 2 : -1;
		if (rank >= 0) ranked.push({ note, rank });
	}
	ranked.sort((a, b) => a.rank - b.rank || a.note.title.localeCompare(b.note.title));
	const exact = notes.some((n) => n.title.toLowerCase() === q);
	return { notes: ranked.slice(0, limit).map((r) => r.note), create: typed && !exact ? typed : null };
}

// ── Resolution (editor live preview) ─────────────────────────────────────────

/** What `get_link_targets` returns (src/lib/types.ts). */
export interface LinkTargetData {
	notes: { id: number; title: string }[];
	issues: { id: number; title: string; context: string }[];
	todos: { id: number; title: string; status: string }[];
}

export interface LinkIndex {
	/** Lowercased note title → note id. */
	notes: Map<string, number>;
	issues: Map<number, { title: string; context: string }>;
	todos: Map<number, { title: string; status: string }>;
}

export function indexLinkTargets(data: LinkTargetData): LinkIndex {
	return {
		notes: new Map(data.notes.map((n) => [n.title.toLowerCase(), n.id])),
		issues: new Map(data.issues.map((i) => [i.id, { title: i.title, context: i.context }])),
		todos: new Map(data.todos.map((t) => [t.id, { title: t.title, status: t.status }]))
	};
}

/** A chip for a normalized tag, when it names an existing issue or todo. */
export function chipFor(
	tag: string,
	index: LinkIndex
): { kind: 'issue' | 'todo'; id: number; title: string; status: string | null; detail: string | null } | null {
	const entity = entityTag(tag);
	if (!entity) return null;
	if (entity.kind === 'issue') {
		const issue = index.issues.get(entity.id);
		return issue ? { ...entity, title: issue.title, status: null, detail: issue.context } : null;
	}
	const todo = index.todos.get(entity.id);
	return todo ? { ...entity, title: todo.title, status: todo.status, detail: null } : null;
}

/** What following a tag, chip or wikilink does (⌘-click in the editor, click in the read view). */
export type LinkFollow = { kind: 'tag'; tag: string } | { kind: 'note'; title: string; id: number | null };
