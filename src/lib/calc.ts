// Formulas in ```calc blocks of markdown notes, minus the UI: the evaluator, the number
// format, and finding the blocks in a markdown syntax tree. One evaluator serves both the
// editor (src/lib/editor/calc.ts) and the read view (src/lib/calc-reader.ts). Results are
// display-only: nothing here writes to the note. Needs no DOM (testable under node:test).
//
// A line is `name = expr`, a bare `expr`, a `//` or `#` comment, or blank; a trailing
// ` // text` is a comment. Variables apply to the whole note, top to bottom, across blocks.
// `sum` adds the results above it, back to the previous sum line, a blank line or the
// block start; lines that error are skipped and flag the sum with ⚠.
import type { Text } from '@codemirror/state';
import { IterMode, type SyntaxNode, type Tree } from '@lezer/common';
export type CalcKind = 'value' | 'sum' | 'error' | 'comment' | 'blank';

export interface CalcLine {
	kind: CalcKind;
	/** The result slot: `12 424,50`, `9 850 ⚠`, or an error message; '' for comments and blanks. */
	text: string;
	/** What clicking the result copies: the number as shown, without ⚠; '' when there is none. */
	copy: string;
}

/** Results from this size on (and NaN, ±Infinity) show as `too large`: doubles lose the cents. */
const LIMIT = 1e15;
const WARN = ' ⚠';
const RESERVED = new Set(['sum', 'of']);

// ── Format ────────────────────────────────────────────────────────────────────

/**
 * `n` rounded to 2 decimals: whole results bare (`16 900`), others with exactly two
 * (`12 424,50`). Groups with U+00A0, a decimal comma and an ASCII minus; never `-0`.
 * Hand-made rather than Intl, whose output differs between engines. Null when `n` is not
 * finite or too large to show to the cent.
 */
export function formatNumber(n: number): string | null {
	if (!Number.isFinite(n) || Math.abs(n) >= LIMIT) return null;
	const abs = Math.abs(n);
	if (abs < 1e-6) return '0'; // toPrecision would switch to an exponent
	// 15 significant digits drop binary noise (1.005 is stored as 1.00499…); then round
	// half up on the decimal digits.
	const digits = abs.toPrecision(15);
	if (digits.includes('e')) return null;
	const [whole, fraction = ''] = digits.split('.');
	const frac = (fraction + '000').slice(0, 3);
	let cents = BigInt(whole) * 100n + BigInt(frac.slice(0, 2));
	if (frac[2] >= '5') cents += 1n;
	if (cents === 0n) return '0';
	const int = (cents / 100n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, '\u00a0');
	const rest = cents % 100n;
	return (n < 0 ? '-' : '') + int + (rest === 0n ? '' : ',' + rest.toString().padStart(2, '0'));
}

// ── Tokens ────────────────────────────────────────────────────────────────────

type Op = '+' | '-' | '*' | '/' | '^' | '(' | ')' | '%' | '=';

type Token =
	| { type: 'num'; value: number; raw: string }
	| { type: 'name'; name: string; raw: string; /** After whitespace (or at the start): can be a label. */ spaced: boolean }
	| { type: 'op'; op: Op; raw: string }
	| { type: 'currency'; raw: string };

class CalcError extends Error {}

const cantRead = (raw: string) => new CalcError(`can’t read “${raw}”`);

const SPACE = /\s+/y;
// Grouping spaces only in 3-digit groups after a 1-3 digit lead; then `.`/`,` parts.
const NUMBER = /(?:\d{1,3}(?:[ \u00a0\u202f]\d{3}(?!\d))+|\d+)(?:[.,]\d+)*/y;
const NAME = /[\p{L}_][\p{L}\d_]*/uy;
const CURRENCY = /\p{Sc}/uy;
const OPS: Record<string, Op> = { '+': '+', '-': '-', '−': '-', '*': '*', '/': '/', '^': '^', '(': '(', ')': ')', '%': '%', '=': '=' };

/**
 * A number as typed: `.` and `,` are both decimal separators; with both, the last one is
 * and the others group thousands (in 3s, like spaces). Null when it doesn't read.
 */
function readNumber(raw: string): number | null {
	const spaced = /[ \u00a0\u202f]/.test(raw);
	const plain = raw.replace(/[ \u00a0\u202f]/g, '');
	const seps = plain.match(/[.,]/g) ?? [];
	if (!seps.length) return Number(plain);
	const decimal = seps[seps.length - 1];
	const groups = seps.slice(0, -1);
	if (groups.length && (spaced || groups.some((s) => s === decimal))) return null;
	const at = plain.lastIndexOf(decimal);
	const int = plain.slice(0, at);
	if (groups.length) {
		const parts = int.split(groups[0]);
		if (!/^\d{1,3}$/.test(parts[0]) || parts.slice(1).some((p) => !/^\d{3}$/.test(p))) return null;
	}
	return Number(`${int.replace(/[.,]/g, '')}.${plain.slice(at + 1)}`);
}

/** The line's tokens, up to the first character that doesn't read (`error`). */
function tokenize(text: string): { tokens: Token[]; error: CalcError | null } {
	const tokens: Token[] = [];
	const match = (re: RegExp, at: number) => {
		re.lastIndex = at;
		return re.exec(text)?.[0] ?? null;
	};
	let at = 0;
	while (at < text.length) {
		const space = match(SPACE, at);
		if (space) {
			at += space.length;
			continue;
		}
		const num = match(NUMBER, at);
		if (num) {
			const value = readNumber(num);
			if (value === null) return { tokens, error: cantRead(num) };
			tokens.push({ type: 'num', value, raw: num });
			at += num.length;
			continue;
		}
		const name = match(NAME, at);
		if (name) {
			tokens.push({ type: 'name', name, raw: name, spaced: at === 0 || /\s/.test(text[at - 1]) });
			at += name.length;
			continue;
		}
		const currency = match(CURRENCY, at);
		if (currency) {
			tokens.push({ type: 'currency', raw: currency });
			at += currency.length;
			continue;
		}
		const ch = String.fromCodePoint(text.codePointAt(at) ?? 0);
		const op = OPS[ch];
		if (!op) return { tokens, error: cantRead(ch) };
		tokens.push({ type: 'op', op, raw: ch });
		at += ch.length;
	}
	return { tokens, error: null };
}

/** Currency symbols are labels next to a number (`€ 120`, `$5`, `120 €`): dropped. */
function dropCurrency(tokens: Token[]): Token[] {
	return tokens.filter((t, i) => {
		if (t.type !== 'currency') return true;
		if (tokens[i + 1]?.type === 'num' || tokens[i - 1]?.type === 'num') return false;
		throw cantRead(t.raw);
	});
}

// ── Evaluation ────────────────────────────────────────────────────────────────

/** A variable: its value (flagged when it came from a ⚠ sum), or failed. */
type Binding = { failed: false; value: number; warn: boolean } | { failed: true };

/** The running `sum`: results since the last boundary, and whether any was skipped or flagged. */
interface Run {
	total: number;
	warn: boolean;
}

/** A partial result: `pct` while it is a bare `N%`, which `+`, `-`, `*` and `/` read specially. */
interface Value {
	value: number;
	pct: boolean;
}

const plain = (v: Value) => (v.pct ? v.value / 100 : v.value);

/** Recursive descent over one line's tokens. */
class Parser {
	private at = 0;
	private readonly tokens: Token[];
	private readonly vars: Map<string, Binding>;
	private readonly run: Run;
	/** Read a flagged variable or sum. */
	warn = false;

	constructor(tokens: Token[], vars: Map<string, Binding>, run: Run) {
		this.tokens = tokens;
		this.vars = vars;
		this.run = run;
	}

	private peek(): Token | undefined {
		return this.tokens[this.at];
	}

	private isOp(op: Op): boolean {
		const t = this.peek();
		return t?.type === 'op' && t.op === op;
	}

	private unexpected(): CalcError {
		const t = this.peek();
		return t ? cantRead(t.raw) : new CalcError('incomplete');
	}

	/**
	 * The whole line: an expression, maybe followed by one label word set apart by a space
	 * (`1200 kr`); a word glued on (`1e3`, `2x`) is an error, not a label.
	 */
	line(): number {
		const result = plain(this.expr());
		const rest = this.tokens.slice(this.at);
		if (rest.length === 1 && rest[0].type === 'name' && !RESERVED.has(rest[0].name)) {
			if (!rest[0].spaced) throw this.unexpected();
			if (this.vars.has(rest[0].name)) throw new CalcError(`${rest[0].name} is a variable`);
		} else if (rest.length) {
			throw this.unexpected();
		}
		return result;
	}

	private expr(): Value {
		let left = this.term();
		while (this.isOp('+') || this.isOp('-')) {
			const sign = this.isOp('+') ? 1 : -1;
			this.at++;
			const right = this.term();
			// `x + 10%` is x·1.1, `x - 10%` is x·0.9.
			const value = right.pct ? plain(left) * (1 + (sign * right.value) / 100) : plain(left) + sign * right.value;
			left = { value, pct: false };
		}
		return left;
	}

	private term(): Value {
		let left = this.unary();
		while (this.isOp('*') || this.isOp('/')) {
			const divide = this.isOp('/');
			this.at++;
			const right = plain(this.unary());
			if (divide && right === 0) throw new CalcError('÷ 0');
			left = { value: divide ? plain(left) / right : plain(left) * right, pct: false };
		}
		return left;
	}

	/** Below `^`, so `-2 ^ 2` is -4. */
	private unary(): Value {
		if (this.isOp('-')) {
			this.at++;
			const inner = this.unary();
			return { value: -inner.value, pct: inner.pct };
		}
		if (this.isOp('+')) {
			this.at++;
			return this.unary();
		}
		return this.power();
	}

	/** Right-associative: `2 ^ 3 ^ 2` is 2⁹. */
	private power(): Value {
		const base = this.percent();
		if (!this.isOp('^')) return base;
		this.at++;
		return { value: plain(base) ** plain(this.unary()), pct: false };
	}

	/** `N%` and `N% of x`. */
	private percent(): Value {
		const value = this.primary();
		if (!this.isOp('%')) return { value, pct: false };
		this.at++;
		const next = this.peek();
		if (next?.type === 'name' && next.name === 'of') {
			this.at++;
			return { value: (value / 100) * plain(this.unary()), pct: false };
		}
		return { value, pct: true };
	}

	private primary(): number {
		const t = this.peek();
		if (t?.type === 'num') {
			this.at++;
			return t.value;
		}
		if (t?.type === 'name' && t.name !== 'of') {
			this.at++;
			if (t.name === 'sum') {
				this.warn ||= this.run.warn;
				return this.run.total;
			}
			const binding = this.vars.get(t.name);
			if (!binding) throw new CalcError(`unknown: ${t.name}`);
			if (binding.failed) throw new CalcError(`${t.name} has an error`);
			this.warn ||= binding.warn;
			return binding.value;
		}
		if (t?.type === 'op' && t.op === '(') {
			this.at++;
			const inner = plain(this.expr());
			if (!this.isOp(')')) throw this.unexpected();
			this.at++;
			return inner;
		}
		throw this.unexpected();
	}
}

const COMMENT = /^(?:\/\/|#)/;
const TRAILING_COMMENT = /\s\/\/.*$/;

function evaluateLine(source: string, vars: Map<string, Binding>, run: Run): CalcLine {
	// NFC: a name typed with a combining accent (a + ◌́) is the same name as á.
	const text = source.normalize('NFC').replace(TRAILING_COMMENT, '').trim();
	if (!text) {
		run.total = 0;
		run.warn = false;
		return { kind: 'blank', text: '', copy: '' };
	}
	if (COMMENT.test(text)) return { kind: 'comment', text: '', copy: '' };

	const lexed = tokenize(text);
	let tokens = lexed.tokens;
	let name: string | null = null;
	if (tokens[0]?.type === 'name' && tokens[1]?.type === 'op' && tokens[1].op === '=') {
		name = tokens[0].name;
		tokens = tokens.slice(2);
	}
	const isSum = tokens.some((t) => t.type === 'name' && t.name === 'sum');

	let value: number;
	let warn: boolean;
	try {
		if (name && RESERVED.has(name)) throw new CalcError(`${name} is reserved`);
		if (lexed.error) throw lexed.error;
		const parser = new Parser(dropCurrency(tokens), vars, run);
		value = parser.line();
		warn = parser.warn;
	} catch (e) {
		if (!(e instanceof CalcError)) throw e;
		if (name && !RESERVED.has(name)) vars.set(name, { failed: true });
		if (isSum) {
			run.total = 0;
			run.warn = false;
		} else {
			run.warn = true; // skipped by the next sum
		}
		return { kind: 'error', text: e.message, copy: '' };
	}

	const shown = formatNumber(value);
	if (shown === null) {
		if (name) vars.set(name, { failed: true });
		if (isSum) {
			run.total = 0;
			run.warn = false;
		} else {
			run.warn = true;
		}
		return { kind: 'error', text: 'too large', copy: '' };
	}
	if (name) vars.set(name, { failed: false, value, warn });
	if (isSum) {
		run.total = 0;
		run.warn = false;
	} else {
		run.total += value;
		run.warn ||= warn;
	}
	return { kind: isSum ? 'sum' : 'value', text: warn ? shown + WARN : shown, copy: shown };
}

/**
 * Results for every line of a note's calc blocks, in note order. Variables carry from
 * block to block; `sum` starts afresh in each block.
 */
export function evaluateBlocks(blocks: string[][]): CalcLine[][] {
	const vars = new Map<string, Binding>();
	return blocks.map((lines) => {
		const run: Run = { total: 0, warn: false };
		return lines.map((line) => evaluateLine(line, vars, run));
	});
}

// ── Blocks ────────────────────────────────────────────────────────────────────

const CALC_FENCE = /(?:`{3,}|~{3,})[ \t]*calc(?=\s|$)/i;

/**
 * Could the note have a calc block? A cheap scan of its lines for a calc fence (in a
 * quote or list item too), so notes without one skip parsing and evaluation.
 */
export function hasCalcFence(lines: Iterable<string>): boolean {
	for (const line of lines) if (CALC_FENCE.test(line)) return true;
	return false;
}

/** Only the info string `calc` qualifies (first word, any case): `Calc budget` too. */
export function isCalcInfo(info: string): boolean {
	return info.trim().split(/\s+/)[0].toLowerCase() === 'calc';
}

export interface CalcSourceLine {
	/** The line as the code block holds it: container prefixes (`> `, indentation) left out. */
	text: string;
	/** The document line's start and end. */
	from: number;
	to: number;
}

export interface CalcBlock {
	from: number;
	to: number;
	lines: CalcSourceLine[];
}

/** Nodes a fenced block can sit in; everything else is skipped without descending. */
const CONTAINERS = new Set(['Document', 'Blockquote', 'BulletList', 'OrderedList', 'ListItem']);

function children(node: SyntaxNode): SyntaxNode[] {
	const out: SyntaxNode[] = [];
	const cursor = node.cursor(IterMode.IgnoreMounts);
	if (!cursor.firstChild()) return out;
	do out.push(cursor.node);
	while (cursor.nextSibling());
	return out;
}

function calcBlockOf(node: SyntaxNode, doc: Text): CalcBlock | null {
	const parts = children(node);
	const marks = parts.filter((c) => c.name === 'CodeMark');
	const info = parts.find((c) => c.name === 'CodeInfo');
	if (!marks.length || !info || !isCalcInfo(doc.sliceString(info.from, info.to))) return null;
	const open = doc.lineAt(marks[0].from).number;
	// Unterminated, the block follows the parser: to the end of the note or its container.
	const closed = marks.length > 1 && doc.lineAt(marks[marks.length - 1].from).number > open;
	const last = closed ? doc.lineAt(marks[marks.length - 1].from).number - 1 : doc.lineAt(node.to).number;
	// In document order; one CodeText may span several lines. The index only moves forward,
	// so this is linear in lines plus CodeText nodes.
	const texts = parts.filter((c) => c.name === 'CodeText');
	const lines: CalcSourceLine[] = [];
	let first = 0;
	for (let n = open + 1; n <= last; n++) {
		const line = doc.line(n);
		while (first < texts.length && texts[first].to < line.from) first++;
		let text = '';
		for (let i = first; i < texts.length && texts[i].from <= line.to; i++) {
			text += doc.sliceString(Math.max(texts[i].from, line.from), Math.min(texts[i].to, line.to));
		}
		lines.push({ text, from: line.from, to: line.to });
	}
	return { from: node.from, to: node.to, lines };
}

/**
 * The note's ```calc blocks in order, from its markdown syntax tree (code inside quotes
 * and list items too). Lines come from the parser's CodeText, as the read view sees them.
 */
export function calcBlocks(tree: Tree, doc: Text): CalcBlock[] {
	const blocks: CalcBlock[] = [];
	tree.iterate({
		mode: IterMode.IgnoreMounts,
		enter(ref) {
			if (ref.name === 'FencedCode') {
				const block = calcBlockOf(ref.node, doc);
				if (block) blocks.push(block);
				return false;
			}
			return CONTAINERS.has(ref.name);
		}
	});
	return blocks;
}
