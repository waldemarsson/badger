import './helpers/lib-resolve.mjs';
import assert from 'node:assert/strict';
import test from 'node:test';

const { evaluateBlocks, formatNumber } = await import('../src/lib/calc.ts');

/** Shown texts of one block, with U+00A0 grouping read as a plain space. */
const show = (lines) => lines.map((l) => l.text.replaceAll('\u00a0', ' '));
const run = (...blocks) => evaluateBlocks(blocks.map((b) => b.split('\n'))).map(show);
const one = (source) => run(source)[0];
/** The result of a single expression line. */
const calc = (expr) => one(expr)[0];

// ── Evaluator ─────────────────────────────────────────────────────────────────

const SAMPLE = `// Fixed costs
flights = 2 * 8 450
hotel = 9 * 1 380,50
rail = 2 * 3 200 kr
fixed = sum

// Daily
food = 9 * 650
extras = 4 000 kr
daily = sum

total = fixed + daily
share = total / 2
with_buffer = total + 10%
souvenirs = 1 500 + gifts`;

test('the mockup sample evaluates to its results', () => {
	assert.deepEqual(one(SAMPLE), [
		'',
		'16 900',
		'12 424,50',
		'6 400',
		'35 724,50',
		'',
		'',
		'5 850',
		'4 000',
		'9 850',
		'',
		'45 574,50',
		'22 787,25',
		'50 131,95',
		'unknown: gifts'
	]);
	const kinds = evaluateBlocks([SAMPLE.split('\n')])[0].map((l) => l.kind);
	assert.deepEqual(kinds, [
		'comment',
		'value',
		'value',
		'value',
		'sum',
		'blank',
		'comment',
		'value',
		'value',
		'sum',
		'blank',
		'value',
		'value',
		'value',
		'error'
	]);
});

test('operators, precedence and right-associative power', () => {
	assert.equal(calc('1 + 2'), '3');
	assert.equal(calc('5 - 8'), '-3');
	assert.equal(calc('4 * 2,5'), '10');
	assert.equal(calc('7 / 2'), '3,50');
	assert.equal(calc('2 ^ 10'), '1 024');
	assert.equal(calc('1 + 2 * 3'), '7');
	assert.equal(calc('(1 + 2) * 3'), '9');
	assert.equal(calc('10 - 4 - 3'), '3');
	assert.equal(calc('24 / 4 / 2'), '3');
	assert.equal(calc('2 ^ 3 ^ 2'), '512');
	assert.equal(calc('2 * 3 ^ 2'), '18');
	assert.equal(calc('-2 ^ 2'), '-4');
	assert.equal(calc('2 ^ -1'), '0,50');
	assert.equal(calc('-(3 + 4)'), '-7');
	assert.equal(calc('--5'), '5');
	assert.equal(calc('+5'), '5');
	assert.equal(calc('3 − 5'), '-2'); // U+2212
	assert.equal(calc('((2))'), '2');
});

test('percent forms', () => {
	assert.equal(calc('200 + 10%'), '220');
	assert.equal(calc('200 - 10%'), '180');
	assert.equal(calc('20% of 50'), '10');
	assert.equal(calc('200 * 10%'), '20');
	assert.equal(calc('200 / 10%'), '2 000');
	assert.equal(calc('10%'), '0,10');
	assert.equal(calc('20% of 50 + 1'), '11');
	assert.deepEqual(one('x = 1 000\nx + 25%\n5% of x'), ['1 000', '1 250', '50']);
});

test('output format: 2-decimal rounding, whole numbers bare, NBSP grouping, comma, ASCII minus, no -0', () => {
	assert.equal(formatNumber(16900), '16\u00a0900');
	assert.equal(formatNumber(12424.5), '12\u00a0424,50');
	assert.equal(formatNumber(0.1 * 3 * 10), '3');
	assert.equal(formatNumber(0.1 + 0.2), '0,30');
	assert.equal(formatNumber(1.005), '1,01');
	assert.equal(formatNumber(2.999), '3');
	assert.equal(formatNumber(-1234567.891), '-1\u00a0234\u00a0567,89');
	assert.equal(formatNumber(-0), '0');
	assert.equal(formatNumber(-0.001), '0');
	assert.equal(formatNumber(0.000001), '0');
	assert.equal(formatNumber(999), '999');
	assert.equal(formatNumber(1000), '1\u00a0000');
	assert.equal(formatNumber(Infinity), null);
	assert.equal(formatNumber(NaN), null);
	assert.equal(formatNumber(1e15), null);
	assert.equal(calc('0.1 + 0.2'), '0,30');
	assert.equal(calc('0 * -1'), '0');
});

test('numbers: both decimal separators, grouping spaces, mixed separators, strict grouping', () => {
	assert.equal(calc('1.5'), '1,50');
	assert.equal(calc('1,5'), '1,50');
	assert.equal(calc('1,200'), '1,20');
	assert.equal(calc('1 380,50'), '1 380,50');
	assert.equal(calc('1\u00a0380,50'), '1 380,50');
	assert.equal(calc('1\u202f380.50'), '1 380,50');
	assert.equal(calc('12 345 678'), '12 345 678');
	assert.equal(calc('2 300'), '2 300');
	assert.equal(calc('1.200,50'), '1 200,50');
	assert.equal(calc('1,200.50'), '1 200,50');
	assert.equal(calc('1.234.567,5'), '1 234 567,50');
	assert.equal(calc('1 2'), 'can’t read “2”');
	assert.equal(calc('1 2345'), 'can’t read “2345”');
	assert.equal(calc('1234 567'), 'can’t read “567”');
	assert.equal(calc('1.2.3'), 'can’t read “1.2.3”');
	assert.equal(calc('1.20,5'), 'can’t read “1.20,5”');
	assert.equal(calc('1 200.300,5'), 'can’t read “1 200.300,5”');
});

test('sum adds back to a blank line, the previous sum or the block start; comments do not break it', () => {
	assert.deepEqual(one('1\n2\nsum'), ['1', '2', '3']);
	assert.deepEqual(one('1\n\n2\n3\nsum'), ['1', '', '2', '3', '5']);
	assert.deepEqual(one('1\n2\nsum\n4\nsum'), ['1', '2', '3', '4', '4']);
	assert.deepEqual(one('1\n// note\n# note\n2\nsum'), ['1', '', '', '2', '3']);
	assert.deepEqual(one('sum'), ['0']);
	// A sum used in an expression is still a sum line, and its result is not counted.
	assert.deepEqual(one('1\n2\nsum * 2\n5\nsum'), ['1', '2', '6', '5', '5']);
	assert.deepEqual(one('1\n2\nx = sum\nx + 1'), ['1', '2', '3', '4']);
	// Block start: the second block's sum does not reach into the first.
	assert.deepEqual(run('1\n2', '3\nsum'), [
		['1', '2'],
		['3', '3']
	]);
});

test('sum skips lines that error and flags itself, and the flag carries through', () => {
	const lines = evaluateBlocks([['1', 'oops', '2', 'a = sum', 'a * 2', 'b = 4', 'b + 1']])[0];
	assert.deepEqual(show(lines), ['1', 'unknown: oops', '2', '3 ⚠', '6 ⚠', '4', '5']);
	assert.equal(lines[3].kind, 'sum');
	assert.equal(lines[3].copy, '3');
	assert.equal(lines[4].copy, '6');
	// A flagged line counted by a later sum flags that sum too.
	assert.deepEqual(one('1\nx\nsum\n\nsum * 1\n'), ['1', 'unknown: x', '1 ⚠', '', '0', '']);
	assert.deepEqual(one('1\nx\ns = sum\n\ns\n2\nsum'), ['1', 'unknown: x', '1 ⚠', '', '1 ⚠', '2', '3 ⚠']);
});

test('variables span blocks top to bottom; use before definition is an error; redefinition applies below', () => {
	assert.deepEqual(run('a = 2', 'b = a * 3', 'a + b'), [['2'], ['6'], ['8']]);
	assert.deepEqual(run('b * 2', 'b = 1'), [['unknown: b'], ['1']]);
	assert.deepEqual(one('x = 1\nx\nx = 5\nx'), ['1', '1', '5', '5']);
	assert.deepEqual(one('x = 1\nx = x + 1\nx'), ['1', '2', '2']);
	// Names: letters (incl. åäöé), digits, `_`; case-sensitive.
	assert.deepEqual(one('hyra_år = 3\nhyra_år * 2'), ['3', '6']);
	assert.deepEqual(one('café2 = 1\ncafé2'), ['1', '1']);
	assert.deepEqual(one('A = 1\na'), ['1', 'unknown: a']);
});

test('a failed assignment marks the variable, and uses say so', () => {
	assert.deepEqual(one('x = 1 / 0\nx + 1\ny = x\ny'), ['÷ 0', 'x has an error', 'x has an error', 'y has an error']);
	assert.deepEqual(one('x = nope\nx = 2\nx'), ['unknown: nope', '2', '2']);
});

test('labels: one trailing word, currency symbols before or after, a defined variable is an error', () => {
	assert.equal(calc('1200 kr'), '1 200');
	assert.equal(calc('2 * 3 200 kr'), '6 400');
	assert.equal(calc('€ 120'), '120');
	assert.equal(calc('$5'), '5');
	assert.equal(calc('120 €'), '120');
	assert.equal(calc('£3 + 4£'), '7');
	assert.equal(calc('(2 + 3) apples'), '5');
	assert.deepEqual(one('kr = 10\n1200 kr'), ['10', 'kr is a variable']);
	assert.equal(calc('5 apples pears'), 'can’t read “apples”');
	assert.equal(calc('5 sum'), 'can’t read “sum”');
	assert.equal(calc('€'), 'can’t read “€”');
	assert.equal(calc('5 + €'), 'can’t read “€”');
});

test('a label is set apart by a space: a word glued to a number or `)` is an error', () => {
	assert.equal(calc('1e3'), 'can’t read “e3”');
	assert.equal(calc('2x'), 'can’t read “x”');
	assert.equal(calc('1200kr'), 'can’t read “kr”');
	assert.equal(calc('(2 + 3)apples'), 'can’t read “apples”');
	assert.deepEqual(one('kr = 10\n5kr'), ['10', 'can’t read “kr”']);
	// Still labels or currency: spaced words, and symbols before or after the number.
	assert.equal(calc('1200 kr'), '1 200');
	assert.equal(calc('120 €'), '120');
	assert.equal(calc('120€'), '120');
	assert.equal(calc('$5'), '5');
	assert.equal(calc('€120'), '120');
	assert.equal(calc('(2 + 3) apples'), '5');
});

test('comments and blanks', () => {
	const lines = evaluateBlocks([['// a', '# b', '', '   ', '3 // three', '4 # four']])[0];
	assert.deepEqual(
		lines.map((l) => l.kind),
		['comment', 'comment', 'blank', 'blank', 'value', 'error']
	);
	assert.equal(lines[4].text, '3');
	assert.equal(lines[5].text, 'can’t read “#”');
	for (const l of lines.slice(0, 4)) {
		assert.equal(l.text, '');
		assert.equal(l.copy, '');
	}
});

test('error messages', () => {
	assert.equal(calc('gifts'), 'unknown: gifts');
	assert.equal(calc('5 / 0'), '÷ 0');
	assert.equal(calc('5 / (2 - 2)'), '÷ 0');
	assert.equal(calc('5 / 0%'), '÷ 0');
	assert.equal(calc('5 +'), 'incomplete');
	assert.equal(calc('(5 + 1'), 'incomplete');
	assert.equal(calc('x ='), 'incomplete');
	assert.equal(calc('20% of'), 'incomplete');
	assert.equal(calc('5 @ 2'), 'can’t read “@”');
	assert.equal(calc('2 = 3'), 'can’t read “=”');
	assert.equal(calc('x == 3'), 'can’t read “=”');
	assert.equal(calc('5 )'), 'can’t read “)”');
	assert.equal(calc('of'), 'can’t read “of”');
	assert.equal(calc('10 ^ 400'), 'too large');
	assert.equal(calc('(0 - 8) ^ 0,5'), 'too large');
	assert.equal(calc('sum = 3'), 'sum is reserved');
	assert.equal(calc('of = 3'), 'of is reserved');
	// Other lines still evaluate.
	assert.deepEqual(one('1 +\n2'), ['incomplete', '2']);
	const err = evaluateBlocks([['5 +']])[0][0];
	assert.equal(err.kind, 'error');
	assert.equal(err.copy, '');
});

test('names are compared in NFC, so a decomposed á is the same name', () => {
	assert.deepEqual(one('m\u00e1l = 4\nma\u0301l * 2'), ['4', '8']);
	assert.deepEqual(one('ma\u0301l = 4\nm\u00e1l + 1'), ['4', '5']);
});

test('the copied text is the shown number without the flag', () => {
	const [a, b] = evaluateBlocks([['1 380,50 * 9', 'nope', 'sum']])[0].filter((l) => l.kind !== 'error');
	assert.equal(a.copy, '12\u00a0424,50');
	assert.equal(a.text, a.copy);
	assert.equal(b.text, '12\u00a0424,50 ⚠');
	assert.equal(b.copy, '12\u00a0424,50');
});

// ── Blocks in a note ──────────────────────────────────────────────────────────

const { calcBlocks, isCalcInfo, hasCalcFence } = await import('../src/lib/calc.ts');
const { Text } = await import('@codemirror/state');
const { GFM, parser } = await import('@lezer/markdown');
const { jsonLanguage } = await import('@codemirror/lang-json');

const md = parser.configure(GFM);
const docOf = (source) => Text.of(source.split('\n'));
/** Each calc block's line texts. */
const blockTexts = (source) => calcBlocks(md.parse(source), docOf(source)).map((b) => b.lines.map((l) => l.text));

test('hasCalcFence: a quick scan for a calc fence anywhere, in containers too', () => {
	for (const doc of ['```calc\n1', '> ~~~ Calc budget', '  - ````CALC', 'x\n```calc']) assert.equal(hasCalcFence(doc.split('\n')), true, doc);
	for (const doc of ['```calculator', '```js calc', 'calc', '``calc', 'no fences', '']) assert.equal(hasCalcFence(doc.split('\n')), false, doc);
});

test('only the calc info string qualifies: first word, any case', () => {
	for (const info of ['calc', 'Calc', 'CALC budget', ' calc  x']) assert.equal(isCalcInfo(info), true, info);
	for (const info of ['calculator', 'js', '', 'mycalc']) assert.equal(isCalcInfo(info), false, info);
});

test('finds calc blocks in note order, with each line and its document position', () => {
	const source = 'Intro\n\n```calc\na = 1\n\nb = a + 1\n```\n\n```js\nx = 1\n```\n\n```calc\na + b\n```';
	const doc = docOf(source);
	const found = calcBlocks(md.parse(source), doc);
	assert.deepEqual(
		found.map((b) => b.lines.map((l) => l.text)),
		[['a = 1', '', 'b = a + 1'], ['a + b']]
	);
	for (const block of found) {
		for (const line of block.lines) {
			const docLine = doc.lineAt(line.from);
			assert.equal(line.from, docLine.from);
			assert.equal(line.to, docLine.to);
		}
	}
	assert.equal(doc.sliceString(found[0].lines[2].from, found[0].lines[2].to), 'b = a + 1');
});

test('calc blocks in a quote and in a list item drop the container prefix', () => {
	assert.deepEqual(blockTexts('> ```calc\n> 1 + 1\n> x = 2\n> ```'), [['1 + 1', 'x = 2']]);
	assert.deepEqual(blockTexts('- item\n\n  ```calc\n  2 * 3\n  ```\n- next'), [['2 * 3']]);
	assert.deepEqual(blockTexts('1. a\n   ```calc\n   5\n   ```'), [['5']]);
});

test('tilde fences, longer fences and an info string with words qualify', () => {
	assert.deepEqual(blockTexts('~~~calc\n1\n~~~'), [['1']]);
	assert.deepEqual(blockTexts('````Calc budget\n2\n````'), [['2']]);
	assert.deepEqual(blockTexts('```CALC\n3\n```'), [['3']]);
});

test('an unterminated calc fence runs to the end of the note (or its container)', () => {
	assert.deepEqual(blockTexts('Intro\n\n```calc\n1\n2'), [['1', '2']]);
	assert.deepEqual(blockTexts('```calc\n1\n\n'), [['1', '', '']]);
	assert.deepEqual(blockTexts('```calc'), [[]]);
	assert.deepEqual(blockTexts('> ```calc\n> 4\n\nafter'), [['4']]);
});

test('empty blocks, other languages, indented code, calc-free and JSON notes give no lines', () => {
	assert.deepEqual(blockTexts('```calc\n```'), [[]]);
	assert.deepEqual(blockTexts('```js\n1 + 1\n```\n\n    1 + 1\n\n```\n1\n```'), []);
	assert.deepEqual(blockTexts('# Plain note\n\nNo formulas, `calc` inline.'), []);
	assert.deepEqual(blockTexts(''), []);
	const json = '{"calc": "```calc", "a": [1, 2]}';
	assert.deepEqual(calcBlocks(jsonLanguage.parser.parse(json), docOf(json)), []);
});

// ── Syntax colors ─────────────────────────────────────────────────────────────

const { calcLanguage, calcTags, calcNumbers, matchCodeLanguage } = await import('../src/lib/editor/calc-language.ts');
const { highlightCode, tagHighlighter, tags } = await import('@lezer/highlight');

const tokenClasses = tagHighlighter([
	{ tag: calcTags.name, class: 'name' },
	{ tag: calcTags.operator, class: 'op' },
	{ tag: calcTags.label, class: 'label' },
	{ tag: tags.number, class: 'num' },
	{ tag: tags.keyword, class: 'kw' },
	{ tag: tags.comment, class: 'comment' },
	{ tag: tags.unit, class: 'unit' },
	{ tag: tags.invalid, class: 'invalid' }
]);

/** `[text, class]` for every styled piece of `text`. */
function tokensOf(text) {
	const out = [];
	highlightCode(
		text,
		calcLanguage.parser.parse(text),
		tokenClasses,
		(piece, cls) => cls && out.push([piece, cls]),
		() => {}
	);
	return out;
}

test('calc tokens: names, numbers (with grouping), operators, keywords, currency, comments', () => {
	assert.deepEqual(tokensOf('hotel = 9 * 1 380,50 kr // nights'), [
		['hotel', 'name'],
		['=', 'op'],
		['9', 'num'],
		['*', 'op'],
		['1 380,50', 'num'],
		['kr', 'label'],
		['// nights', 'comment']
	]);
	assert.deepEqual(tokensOf('x = sum + 20% of (y − 1)'), [
		['x', 'name'],
		['=', 'op'],
		['sum', 'kw'],
		['+', 'op'],
		['20', 'num'],
		['%', 'op'],
		['of', 'kw'],
		['(', 'op'],
		['y', 'name'],
		['−', 'op'],
		['1', 'num'],
		[')', 'op']
	]);
	assert.deepEqual(tokensOf('€ 120 @'), [
		['€', 'unit'],
		['120', 'num'],
		['@', 'invalid']
	]);
	assert.deepEqual(tokensOf('// Fixed costs\n  # also'), [
		['// Fixed costs', 'comment'],
		['  # also', 'comment']
	]);
});

test('calc tokens: a word after a number or `)` is a label, elsewhere a name', () => {
	assert.deepEqual(tokensOf('(2 + 3) apples'), [
		['(', 'op'],
		['2', 'num'],
		['+', 'op'],
		['3', 'num'],
		[')', 'op'],
		['apples', 'label']
	]);
	assert.deepEqual(tokensOf('total = fixed + daily\n5 kr'), [
		['total', 'name'],
		['=', 'op'],
		['fixed', 'name'],
		['+', 'op'],
		['daily', 'name'],
		['5', 'num'],
		['kr', 'label']
	]);
});

test('calc tokens: a word glued to a number or `)` is not a label but invalid', () => {
	assert.deepEqual(tokensOf('1e3'), [
		['1', 'num'],
		['e3', 'invalid']
	]);
	assert.deepEqual(tokensOf('(2)x'), [
		['(', 'op'],
		['2', 'num'],
		[')', 'op'],
		['x', 'invalid']
	]);
});

test('the calc language matches only the info string calc (first word, any case)', () => {
	for (const info of ['calc', 'Calc', 'CALC']) assert.equal(matchCodeLanguage(info)?.name, 'calc', info);
	for (const info of ['calculator', 'calcx', 'js calc', 'mycalc']) assert.notEqual(matchCodeLanguage(info)?.name, 'calc', info);
	assert.equal(matchCodeLanguage('rust')?.name, 'Rust');
	assert.equal(matchCodeLanguage('calc').support.language, calcLanguage);
});

test('calcNumbers marks whole numbers, grouping included, so the read view keeps them on one line', () => {
	const pieces = [];
	const text = 'x = 1 380,50 + 2';
	highlightCode(text, calcLanguage.parser.parse(text), calcNumbers, (piece, cls) => cls && pieces.push([piece, cls]), () => {});
	assert.deepEqual(pieces, [
		['1 380,50', 'calc-num'],
		['2', 'calc-num']
	]);
});

// ── Editor results ────────────────────────────────────────────────────────────

const { calcResults, calcDecorations, CalcResultWidget } = await import('../src/lib/editor/calc.ts');
const { EditorState } = await import('@codemirror/state');
const { markdown, markdownLanguage } = await import('@codemirror/lang-markdown');

const editorState = (doc) =>
	EditorState.create({ doc, extensions: [markdown({ base: markdownLanguage, codeLanguages: matchCodeLanguage })] });

test('editor results: one per value, sum or error line, at that line, across blocks', () => {
	const doc = 'Trip\n\n```calc\n// costs\na = 2 * 3\n\nb = nope\n```\n\n> ```calc\n> a + 1\n> sum\n> ```';
	const state = editorState(doc);
	const found = calcResults(state).map(({ from, to, result }) => [state.doc.sliceString(from, to), result.kind, result.text]);
	assert.deepEqual(found, [
		['a = 2 * 3', 'value', '6'],
		['b = nope', 'error', 'unknown: nope'],
		['> a + 1', 'value', '7'],
		['> sum', 'sum', '7']
	]);
	assert.deepEqual(calcResults(editorState('No calc here\n\n```js\n1 + 1\n```')), []);
	assert.deepEqual(calcResults(editorState('```calc\n```')), []);
	assert.deepEqual(calcResults(editorState('```calculator\n1 + 1\n```')), []);
});

test('editor results are widgets at line ends that leave the document alone', () => {
	const doc = '```calc\nx = 1 380,50\nx * 2\n```';
	const state = editorState(doc);
	const widgets = [];
	calcDecorations(state).between(0, doc.length, (from, to, deco) => {
		if (deco.spec.widget) widgets.push([from, deco.spec.widget.result.text]);
	});
	assert.deepEqual(widgets, [
		[doc.indexOf('\nx * 2'), '1 380,50'],
		[doc.lastIndexOf('\n```'), '2 761']
	]);
	assert.equal(state.doc.toString(), doc);
});

test('result widgets compare by text, so unchanged results keep their DOM', () => {
	const a = new CalcResultWidget({ kind: 'value', text: '6', copy: '6' });
	assert.equal(a.eq(new CalcResultWidget({ kind: 'value', text: '6', copy: '6' })), true);
	assert.equal(a.eq(new CalcResultWidget({ kind: 'value', text: '7', copy: '7' })), false);
	assert.equal(a.eq(new CalcResultWidget({ kind: 'sum', text: '6', copy: '6' })), false);
	assert.equal(a.ignoreEvent(), true);
});

// ── Read view ─────────────────────────────────────────────────────────────────

const { renderCalcBlocks } = await import('../src/lib/calc-reader.ts');

/** Just enough DOM for renderCalcBlocks, with an outerHTML to compare. */
class FakeNode {
	constructor(doc, tag, text = null) {
		this.ownerDocument = doc;
		this.tagName = tag;
		this.text = text;
		this.childNodes = [];
		this.parentNode = null;
		this.attrs = new Map();
		this.hidden = false;
	}
	get className() {
		return this.attrs.get('class') ?? '';
	}
	set className(v) {
		this.attrs.set('class', v);
	}
	setAttribute(name, value) {
		this.attrs.set(name, String(value));
	}
	getAttribute(name) {
		return this.attrs.get(name) ?? null;
	}
	set colSpan(v) {
		this.attrs.set('colspan', String(v));
	}
	set type(v) {
		this.attrs.set('type', v);
	}
	set title(v) {
		this.attrs.set('title', v);
	}
	get textContent() {
		return this.text ?? this.childNodes.map((c) => c.textContent).join('');
	}
	set textContent(v) {
		this.childNodes = [];
		this.append(this.ownerDocument.createTextNode(v));
	}
	append(...nodes) {
		for (const n of nodes) {
			n.parentNode = this;
			this.childNodes.push(n);
		}
	}
	replaceChildren(...nodes) {
		this.childNodes = [];
		this.append(...nodes);
	}
	*walk() {
		for (const c of this.childNodes) {
			yield c;
			yield* c.walk();
		}
	}
	/** Only `pre > code` and `tag`/`.class` selectors. */
	querySelectorAll(selector) {
		const all = [...this.walk()];
		if (selector === 'pre > code') return all.filter((n) => n.tagName === 'code' && n.parentNode.tagName === 'pre');
		if (selector.startsWith('.')) return all.filter((n) => n.className.split(' ').includes(selector.slice(1)));
		return all.filter((n) => n.tagName === selector);
	}
	get outerHTML() {
		if (this.text !== null) return this.text;
		const attrs = [...this.attrs].map(([k, v]) => ` ${k}="${v}"`).join('');
		return `<${this.tagName}${attrs}${this.hidden ? ' hidden' : ''}>${this.childNodes.map((c) => c.outerHTML).join('')}</${this.tagName}>`;
	}
}

class FakeDocument {
	createElement(tag) {
		return new FakeNode(this, tag);
	}
	createTextNode(text) {
		return new FakeNode(this, '#text', text);
	}
}

/** `<div>` holding `<pre><code class=…>text</code></pre>` per block, as render_markdown emits. */
function rendered(...blocks) {
	const doc = new FakeDocument();
	const root = doc.createElement('div');
	for (const [lang, text] of blocks) {
		const pre = doc.createElement('pre');
		const code = doc.createElement('code');
		if (lang) code.className = `language-${lang}`;
		code.textContent = text;
		pre.append(code);
		root.append(pre);
	}
	return root;
}

const rowsOf = (root) =>
	root.querySelectorAll('tr').map((tr) => [
		tr.className,
		...tr.childNodes.map((td) => td.textContent.replaceAll(' ', ' '))
	]);

test('read view: calc blocks become expression/result rows, evaluated across blocks', () => {
	const root = rendered(['calc', '// Fixed\na = 2 * 8 450\nb = 1 380,50\nsum\n\nc = nope\n'], ['Calc', 'a + 1\n']);
	assert.equal(renderCalcBlocks(root), 2);
	assert.deepEqual(rowsOf(root), [
		['calc-row calc-row-comment', '// Fixed'],
		['calc-row', 'a = 2 * 8 450', '16 900'],
		['calc-row', 'b = 1 380,50', '1 380,50'],
		['calc-row calc-row-sum', 'sum', '18 280,50'],
		['calc-row calc-row-blank', ''],
		['calc-row calc-row-error', 'c = nope', 'unknown: nope'],
		['calc-row', 'a + 1', '16 901']
	]);
	const pres = root.querySelectorAll('pre');
	assert.equal(pres.length, 2, 'the <pre> elements stay');
	assert.ok(pres.every((pre) => pre.className === 'calc-block' && pre.childNodes[0].tagName === 'table'));
	assert.equal(root.querySelectorAll('code').length, 0);
});

test('read view: a result copies its shown number without the flag; errors have nothing to copy', () => {
	const root = rendered(['calc', '1 380,50 * 9\nnope\nsum\n']);
	renderCalcBlocks(root);
	const buttons = root.querySelectorAll('button');
	assert.deepEqual(
		buttons.map((b) => [b.textContent, b.getAttribute('data-calc-copy'), b.getAttribute('type')]),
		[
			['12 424,50', '12 424,50', 'button'],
			['12 424,50 ⚠', '12 424,50', 'button']
		]
	);
	const error = root.querySelectorAll('.calc-row-error')[0];
	assert.equal(error.querySelectorAll('button').length, 0);
});

test('read view: other code blocks stay byte-identical; empty calc blocks render nothing', () => {
	const root = rendered(['js', 'const x = 1 + 2;\n'], [null, 'plain 1 + 1\n'], ['calculator', '1 + 1\n'], ['calc', '\n\n']);
	const before = root.childNodes.slice(0, 3).map((n) => n.outerHTML);
	const nodes = root.childNodes.slice(0, 3).map((n) => [n, n.childNodes[0], n.childNodes[0].childNodes[0]]);
	assert.equal(renderCalcBlocks(root), 1);
	assert.deepEqual(
		root.childNodes.slice(0, 3).map((n) => n.outerHTML),
		before
	);
	root.childNodes.slice(0, 3).forEach((n, i) => {
		assert.equal(n, nodes[i][0]);
		assert.equal(n.childNodes[0], nodes[i][1]);
		assert.equal(n.childNodes[0].childNodes[0], nodes[i][2]);
	});
	const empty = root.childNodes[3];
	assert.equal(empty.hidden, true);
	assert.equal(empty.childNodes.length, 0);
	// Nothing to do without calc blocks.
	const plain = rendered(['rust', 'fn main() {}\n']);
	const html = plain.outerHTML;
	assert.equal(renderCalcBlocks(plain), 0);
	assert.equal(plain.outerHTML, html);
});

test('read view: highlighted sources come from the given highlighter', () => {
	const root = rendered(['calc', 'x = 2 // two\n']);
	const doc = root.ownerDocument;
	renderCalcBlocks(root, {
		highlight: (text) => {
			const span = doc.createElement('span');
			span.className = 'hl';
			span.textContent = text;
			return span;
		}
	});
	const [expr, result] = root.querySelectorAll('tr')[0].childNodes;
	assert.equal(expr.childNodes[0].className, 'hl');
	assert.equal(expr.textContent, 'x = 2 // two');
	assert.equal(result.textContent, '2');
});
