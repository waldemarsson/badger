// JSON Format/Minify for JSON notes and ```json blocks in markdown, minus the UI. The
// printer walks the syntax tree of @codemirror/lang-json's parser (the one the editor
// uses) and copies every string, key and number token verbatim, so it is lossless: big
// integers, number spelling (1.0, 1e5), escapes and duplicate keys survive. Text with any
// error node is refused. Needs no DOM (testable under node:test); the editor side lives
// in src/lib/editor/json-mode.ts and src/lib/editor/json-blocks.ts.
import { jsonLanguage } from '@codemirror/lang-json';
import { ensureSyntaxTree, language } from '@codemirror/language';
import { Text, type EditorState } from '@codemirror/state';
import { IterMode, type SyntaxNode, type Tree } from '@lezer/common';

export type JsonFormatMode = 'format' | 'minify';

/** The first syntax error: its range, and where it is (1-based line and column). */
export interface JsonError {
	from: number;
	to: number;
	line: number;
	col: number;
}

export type JsonFormatResult = { ok: true; text: string } | ({ ok: false } & JsonError);

/** Shown inline when Format or Minify refuses: "Invalid JSON: line 3, col 7". */
export function jsonErrorMessage(error: JsonError): string {
	return `Invalid JSON: line ${error.line}, col ${error.col}`;
}

const INDENT = '  ';
const CLOSING: Record<string, string> = { '{': '}', '[': ']' };
/** Tokens copied as written. */
const VERBATIM = new Set(['String', 'PropertyName', 'Number', 'True', 'False', 'Null']);
const PUNCTUATION = new Set(['{', '}', '[', ']', ',', ':']);

function parse(text: string): Tree {
	return jsonLanguage.parser.parse(text);
}

function locate(text: string, from: number, to: number): JsonError {
	const lineStart = text.lastIndexOf('\n', from - 1) + 1;
	let line = 1;
	for (let i = text.indexOf('\n'); i !== -1 && i < from; i = text.indexOf('\n', i + 1)) line++;
	return { from, to, line, col: from - lineStart + 1 };
}

interface Token {
	name: string;
	from: number;
	to: number;
}

/** Leaf tokens in document order, or the first error node. One iterative cursor walk. */
function scan(text: string): Token[] | JsonError {
	const tokens: Token[] = [];
	const cursor = parse(text).cursor();
	do {
		if (cursor.type.isError) return locate(text, cursor.from, cursor.to);
		const { name } = cursor;
		if (VERBATIM.has(name) || PUNCTUATION.has(name)) tokens.push({ name, from: cursor.from, to: cursor.to });
	} while (cursor.next());
	return tokens;
}

export interface JsonLintError {
	from: number;
	to: number;
	message: string;
}

/** True when `doc` holds only whitespace. Stops at the first other character. */
function isBlank(doc: Text): boolean {
	for (const iter = doc.iter(); !iter.next().done; ) if (iter.value.trim()) return false;
	return true;
}

/**
 * The lint error for an editor holding `doc`, read from `tree` (the editor's incremental
 * parse of it, so no re-parse per edit): the first error with a non-empty range (a missing
 * token marks the next character, or the last one at the end). None for an empty or blank
 * document, which is just a fresh JSON note.
 */
export function jsonTreeLintError(tree: Tree, doc: Text): JsonLintError | null {
	if (isBlank(doc)) return null;
	const cursor = tree.cursor();
	do {
		if (!cursor.type.isError) continue;
		let { from, to } = cursor;
		const line = doc.lineAt(from);
		const message = jsonErrorMessage({ from, to, line: line.number, col: from - line.from + 1 });
		if (to === from) {
			if (to < doc.length) to++;
			else from = Math.max(0, from - 1);
		}
		return { from, to, message };
	} while (cursor.next());
	return null;
}

/** jsonTreeLintError for a plain text, parsed fresh. */
export function jsonLintError(text: string): JsonLintError | null {
	return jsonTreeLintError(parse(text), Text.of(text.split('\n')));
}

/** Hands the thread back (to input and rendering) between parse slices. */
const nextTask = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

/**
 * jsonTreeLintError for the editor's current state, once its incremental parse reaches the
 * end of the document. Advances that parse (shared with the editor's background parser) at
 * most `sliceMs` at a time and awaits `pause` between slices, so a freshly opened multi-MB
 * note gets its diagnostic without blocking input. Null when the document changes before
 * the tree is complete (the linter re-runs for the new one), for a blank document, when
 * the state has no parser, and when `current` returns null (the view is gone).
 */
export async function jsonStateLintError(
	current: () => EditorState | null,
	sliceMs = 50,
	pause: () => Promise<void> = nextTask
): Promise<JsonLintError | null> {
	const doc = current()?.doc;
	if (!doc || isBlank(doc)) return null;
	for (;;) {
		const state = current();
		if (!state || state.doc !== doc || !state.facet(language)) return null;
		const tree = ensureSyntaxTree(state, doc.length, sliceMs);
		if (tree) {
			// Building the final tree can take ~100 ms on multi-MB docs; yield before the walk.
			await pause();
			return current()?.doc === doc ? jsonTreeLintError(tree, doc) : null;
		}
		await pause();
	}
}

/** The smallest single replacement turning `before` into `after`, or null when equal. */
export function changedSpan(before: string, after: string): { from: number; to: number; insert: string } | null {
	if (before === after) return null;
	const max = Math.min(before.length, after.length);
	let start = 0;
	while (start < max && before.charCodeAt(start) === after.charCodeAt(start)) start++;
	let end = 0;
	while (end < max - start && before.charCodeAt(before.length - 1 - end) === after.charCodeAt(after.length - 1 - end)) end++;
	return { from: start, to: before.length - end, insert: after.slice(start, after.length - end) };
}

function print(text: string, mode: JsonFormatMode): JsonFormatResult {
	const tokens = scan(text);
	if (!Array.isArray(tokens)) return { ok: false, ...tokens };
	const pretty = mode === 'format';
	const indents = [''];
	const newline = (depth: number): string => {
		while (indents.length <= depth) indents.push(indents[indents.length - 1] + INDENT);
		return '\n' + indents[depth];
	};
	const out: string[] = [];
	let depth = 0;
	for (let i = 0; i < tokens.length; i++) {
		const { name, from, to } = tokens[i];
		switch (name) {
			case '{':
			case '[':
				if (tokens[i + 1]?.name === CLOSING[name]) {
					out.push(name, CLOSING[name]); // empty stays compact
					i++;
				} else {
					depth++;
					out.push(pretty ? name + newline(depth) : name);
				}
				break;
			case '}':
			case ']':
				depth--;
				out.push(pretty ? newline(depth) + name : name);
				break;
			case ',':
				out.push(pretty ? ',' + newline(depth) : ',');
				break;
			case ':':
				out.push(pretty ? ': ' : ':');
				break;
			default:
				out.push(text.slice(from, to));
		}
	}
	return { ok: true, text: out.join('') };
}

/** Pretty-prints with a 2-space indent; empty `{}` and `[]` stay compact. */
export function formatJson(text: string): JsonFormatResult {
	return print(text, 'format');
}

/** Removes all insignificant whitespace. */
export function minifyJson(text: string): JsonFormatResult {
	return print(text, 'minify');
}

export function formatJsonAs(text: string, mode: JsonFormatMode): JsonFormatResult {
	return print(text, mode);
}

// ── ```json blocks in markdown ────────────────────────────────────────────────

/** A closed ```json (or ~~~json) fenced block of a markdown document. */
export interface JsonBlock {
	/** The FencedCode node. */
	from: number;
	to: number;
	/** End of the opening fence line (where Format/Minify chips go). */
	fenceLineEnd: number;
	/** The content lines, start of the first to end of the last (empty at the closing fence when none). */
	contentFrom: number;
	contentTo: number;
	/** What every line of the block starts with: list indentation and/or `> `. */
	prefix: string;
	/** The content without its container prefixes. */
	text: string;
}

/** Only the info string `json` qualifies (first word, any case): not jsonc or json5. */
export function isJsonInfo(info: string): boolean {
	return info.trim().split(/\s+/)[0].toLowerCase() === 'json';
}

function children(node: SyntaxNode): SyntaxNode[] {
	const out: SyntaxNode[] = [];
	const cursor = node.cursor(IterMode.IgnoreMounts);
	if (!cursor.firstChild()) return out;
	do out.push(cursor.node);
	while (cursor.nextSibling());
	return out;
}

/** `node` as a closed json block, or null. */
export function jsonBlockOf(node: SyntaxNode, doc: Text): JsonBlock | null {
	if (node.name !== 'FencedCode') return null;
	const parts = children(node);
	const marks = parts.filter((c) => c.name === 'CodeMark');
	const info = parts.find((c) => c.name === 'CodeInfo');
	if (marks.length < 2 || !info || !isJsonInfo(doc.sliceString(info.from, info.to))) return null; // unterminated: no chips
	const open = doc.lineAt(marks[0].from);
	const close = doc.lineAt(marks[marks.length - 1].from);
	if (close.number <= open.number) return null;
	const prefix = doc.sliceString(close.from, marks[marks.length - 1].from);
	// The parser leaves container prefixes (indentation, `>`) out of CodeText.
	const lines: string[] = [];
	for (let n = open.number + 1; n < close.number; n++) {
		const line = doc.line(n);
		let text = '';
		for (const c of parts) {
			if (c.name !== 'CodeText' || c.to < line.from || c.from > line.to) continue;
			text += doc.sliceString(Math.max(c.from, line.from), Math.min(c.to, line.to));
		}
		lines.push(text);
	}
	const empty = close.number === open.number + 1;
	return {
		from: node.from,
		to: node.to,
		fenceLineEnd: open.to,
		contentFrom: empty ? close.from : doc.line(open.number + 1).from,
		contentTo: empty ? close.from : doc.line(close.number - 1).to,
		prefix,
		text: lines.join('\n')
	};
}

/** The closed json block containing `pos` (fences included), or null. */
export function findJsonBlock(tree: Tree, doc: Text, pos: number): JsonBlock | null {
	for (const side of [1, -1] as const) {
		for (let node: SyntaxNode | null = tree.resolve(pos, side); node; node = node.parent) {
			if (node.name === 'FencedCode') {
				if (pos < node.from || pos > node.to) break;
				return jsonBlockOf(node, doc);
			}
		}
	}
	return null;
}

export type JsonBlockEdit = { ok: true; from: number; to: number; insert: string } | ({ ok: false } & JsonError);

/**
 * The change that formats or minifies `block`: its content lines, re-prefixed with the
 * block's container prefix. Errors are relative to the block (its first content line is
 * line 1).
 */
export function jsonBlockEdit(block: JsonBlock, mode: JsonFormatMode): JsonBlockEdit {
	const result = formatJsonAs(block.text, mode);
	if (!result.ok) return result;
	const insert = result.text
		.split('\n')
		.map((line) => block.prefix + line)
		.join('\n');
	return { ok: true, from: block.contentFrom, to: block.contentTo, insert };
}
