// The note editor's `/` snippet menu, minus the UI: when `/` opens it, the snippets it
// offers (CodeMirror snippet syntax: `${name}` is a tab stop) and how they are inserted.
// Needs no DOM (testable under node:test); the menu itself is src/lib/editor/slash.ts.
import { snippet, type Completion } from '@codemirror/autocomplete';
import type { EditorState, Transaction } from '@codemirror/state';
import type { SyntaxNode, Tree } from '@lezer/common';

export interface SlashTrigger {
	/** Offset of the `/` in the text before the cursor. */
	from: number;
	/** What was typed after the `/` (filters the menu). */
	query: string;
}

export interface SlashItem {
	id: 'table' | 'code' | 'task' | 'callout' | 'date';
	label: string;
	/** Short preview shown next to the label. */
	detail: string;
	/** Starts its own line (and a table also needs a blank line before it). */
	block: boolean;
	template: (now: Date) => string;
}

/** True when `text` ends inside an inline code span (open backtick run, maybe unclosed). */
export function insideInlineCode(text: string): boolean {
	let open = 0;
	for (let i = 0; i < text.length; ) {
		const ch = text[i];
		if (ch === '\\' && !open) {
			i += 2; // escaped character, e.g. \`
			continue;
		}
		if (ch !== '`') {
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

/**
 * Should the menu open for the text before the cursor on its line? Only for a `/` at the
 * line start or after whitespace, followed by letters/digits, outside inline code — so
 * never inside words, URLs or paths like `a/b`.
 */
export function slashTrigger(lineBefore: string): SlashTrigger | null {
	const match = /(^|\s)\/([\p{L}\p{N}]*)$/u.exec(lineBefore);
	if (!match) return null;
	const from = match.index + match[1].length;
	if (insideInlineCode(lineBefore.slice(0, from))) return null;
	return { from, query: match[2] };
}

const codeNodes = new Set(['FencedCode', 'CodeBlock', 'InlineCode', 'CodeText', 'CodeInfo', 'CodeMark']);

/** True when the markdown syntax tree has code (fenced, indented or inline) at `pos`. */
export function isCodeAt(tree: Tree, pos: number): boolean {
	for (let node: SyntaxNode | null = tree.resolve(pos, 1); node; node = node.parent) {
		if (codeNodes.has(node.name)) return true;
	}
	return false;
}

const tableRowNodes = new Set(['TableHeader', 'TableRow', 'TableDelimiter']);

/** Items offered at `pos`: in a raw table row only inline ones, since a block breaks the row. */
export function slashItemsAt(tree: Tree, pos: number): SlashItem[] {
	for (let node: SyntaxNode | null = tree.resolve(pos, 1); node; node = node.parent) {
		if (tableRowNodes.has(node.name)) return SLASH_ITEMS.filter((item) => !item.block);
	}
	return SLASH_ITEMS;
}

/** `YYYY-MM-DD` in local time. */
export function formatLocalDate(date: Date): string {
	const pad = (n: number, width = 2) => String(n).padStart(width, '0');
	return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

const tableTemplate = [
	'| ${Column 1} | ${Column 2} | ${Column 3} |',
	'| --- | --- | --- |',
	'| ${} | ${} | ${} |',
	'| ${} | ${} | ${} |',
	'',
	'${}'
].join('\n');

export const SLASH_ITEMS: SlashItem[] = [
	{ id: 'table', label: 'Table', detail: '3 × 3', block: true, template: () => tableTemplate },
	{ id: 'code', label: 'Code block', detail: '```', block: true, template: () => '```${}\n${}\n```' },
	{ id: 'task', label: 'Task list', detail: '- [ ]', block: true, template: () => '- [ ] ${}' },
	{ id: 'callout', label: 'Callout', detail: '> [!note]', block: true, template: () => '> [!note] ${}' },
	{
		id: 'date',
		label: "Today's date",
		detail: 'YYYY-MM-DD',
		block: false,
		template: (now) => `${formatLocalDate(now)}\${}`
	}
];

/** Text that is only list, task and quote markers (`- `, `1. `, `- [ ] `, `> `). */
const containerPrefix = /^\s*(?:(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?|>\s?)*$/;

/**
 * The snippet that replaces `/query`, and how many characters before the `/` to remove.
 * `before` is the line's text before the `/`; `prevLine` the previous line (null on the first).
 * After list/quote markers only, one-line items stay in that list item or quote (a task
 * replaces the bullet); a table or code block replaces the empty item's markers, keeping
 * its indentation, since snippet() gives every following line the line's indentation.
 * Block items after other text start on a new line; a table also needs a blank line
 * before it, or markdown reads it as part of the paragraph above.
 */
export function slashInsertion(
	item: SlashItem,
	before: string,
	prevLine: string | null,
	now: Date
): { deleteBefore: number; template: string } {
	const template = item.template(now);
	if (!item.block) return { deleteBefore: 0, template };
	const multiline = item.id === 'table' || item.id === 'code';
	if (before.trim() && containerPrefix.test(before)) {
		// Only list/quote markers before the `/`. A one-line item stays in that list item or
		// quote; a table or code block can't, so it replaces the (empty) item.
		// Keep the indentation: snippet() indents every following line with it.
		if (multiline) {
			const blank = item.id === 'table' && !!prevLine?.trim();
			const indent = before.length - before.trimStart().length;
			return { deleteBefore: before.length - indent, template: (blank ? '\n' : '') + template };
		}
		if (item.id !== 'task') return { deleteBefore: 0, template };
		const bullet = /[-*+]\s+(?:\[[ xX]\]\s+)?$/.exec(before);
		if (bullet) return { deleteBefore: bullet[0].length, template }; // replace the bullet
		const task = /\d+[.)]\s+(\[[ xX]\]\s+)?$/.exec(before);
		if (task) return { deleteBefore: task[1]?.length ?? 0, template: '[ ] ${}' }; // keep the number
		return { deleteBefore: 0, template };
	}
	if (before.trim()) {
		const trailing = before.length - before.trimEnd().length;
		return { deleteBefore: trailing, template: (item.id === 'table' ? '\n\n' : '\n') + template };
	}
	if (item.id === 'table' && prevLine?.trim()) return { deleteBefore: 0, template: '\n' + template };
	return { deleteBefore: 0, template };
}

/** What snippet() needs: an EditorView, or a test stand-in. */
export interface SnippetTarget {
	state: EditorState;
	dispatch: (tr: Transaction) => void;
}

/** Replaces `/query` (the `/` at `slash`, the query ending at `to`) with `item`'s snippet. */
export function applySlashItem(
	target: SnippetTarget,
	item: SlashItem,
	slash: number,
	to: number,
	now: Date,
	completion: Completion | null = null
): void {
	const { doc } = target.state;
	const line = doc.lineAt(slash);
	const before = doc.sliceString(line.from, slash);
	const prevLine = line.number > 1 ? doc.line(line.number - 1).text : null;
	const { deleteBefore, template } = slashInsertion(item, before, prevLine, now);
	snippet(template)(target, completion, slash - deleteBefore, to);
}
