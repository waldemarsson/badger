import assert from 'node:assert/strict';
import test from 'node:test';
import { EditorState, Text } from '@codemirror/state';
import { GFM, parser } from '@lezer/markdown';
import { json, jsonLanguage } from '@codemirror/lang-json';

const { formatJson, minifyJson, jsonErrorMessage, jsonLintError, jsonTreeLintError, jsonStateLintError, changedSpan, isJsonInfo, findJsonBlock, jsonBlockEdit } = await import(
	'../src/lib/json-format.ts'
);

const ok = (result) => {
	assert.equal(result.ok, true, JSON.stringify(result));
	return result.text;
};

test('format pretty-prints with a 2-space indent and keeps empty containers compact', () => {
	const text = '{"a":[1,{"b":null}],"c":{},"d":[],"e":true,"f":false}';
	assert.equal(
		ok(formatJson(text)),
		'{\n  "a": [\n    1,\n    {\n      "b": null\n    }\n  ],\n  "c": {},\n  "d": [],\n  "e": true,\n  "f": false\n}'
	);
	assert.equal(ok(formatJson(' [ { } , [ ] ] ')), '[\n  {},\n  []\n]');
	assert.equal(ok(formatJson('{ }')), '{}');
	assert.equal(ok(formatJson('\n  42 \n')), '42');
	assert.equal(ok(formatJson('"x"')), '"x"');
});

test('minify drops all insignificant whitespace and round-trips with format', () => {
	const pretty = '{\n  "a": [\n    1,\n    2\n  ],\n  "b": {\n    "c": "d e"\n  }\n}';
	assert.equal(ok(minifyJson(pretty)), '{"a":[1,2],"b":{"c":"d e"}}');
	assert.equal(ok(formatJson(ok(minifyJson(pretty)))), pretty);
	assert.equal(ok(minifyJson('\t[ ]\r\n')), '[]');
});

test('tokens are copied verbatim: big ints, number spelling, escapes, unicode, duplicate keys', () => {
	const text = '{"n":12345678901234567890,"f":1.0,"e":1e5,"E":-2.50E-3,"s":"tab\\t \\u00e5 \\"q\\" \\/","u":"åäö 🚀","k":1,"k":2}';
	const pretty = ok(formatJson(text));
	assert.equal(
		pretty,
		'{\n  "n": 12345678901234567890,\n  "f": 1.0,\n  "e": 1e5,\n  "E": -2.50E-3,\n  "s": "tab\\t \\u00e5 \\"q\\" \\/",\n  "u": "åäö 🚀",\n  "k": 1,\n  "k": 2\n}'
	);
	assert.equal(ok(minifyJson(pretty)), text);
	// JSON.parse would lose these: the reason for the tree printer.
	assert.notEqual(JSON.stringify(JSON.parse(text)), text);
});

test('invalid input is refused with the line and column of the first error', () => {
	const cases = [
		['', 1, 1],
		['   ', 1, 4],
		['{"a":1,}', 1, 8],
		['[1 2]', 1, 4],
		['{\n  "a": 1\n  "b": 2\n}', 3, 3],
		['{} x', 1, 4],
		['"open', 1, 1],
		['01', 1, 2],
		['{"a": 1}}', 1, 9],
		['// c\n{}', 1, 1],
		['{"a": "\t"}', 1, 7]
	];
	for (const [text, line, col] of cases) {
		for (const run of [formatJson, minifyJson]) {
			const result = run(text);
			assert.equal(result.ok, false, `${JSON.stringify(text)} must be refused`);
			assert.deepEqual([result.line, result.col], [line, col], JSON.stringify(text));
		}
		// The linter reports the same place, except for blank text (a fresh note).
		const lint = jsonLintError(text);
		if (text.trim()) assert.equal(lint.message, jsonErrorMessage({ from: 0, to: 0, line, col }), JSON.stringify(text));
		else assert.equal(lint, null);
	}
	assert.equal(jsonErrorMessage({ from: 0, to: 0, line: 3, col: 7 }), 'Invalid JSON: line 3, col 7');
	assert.equal(jsonLintError('{"ok": [1, 2.5, "x"]}'), null);
});

test('lint: one error with a visible range, none for blank or valid text', () => {
	assert.equal(jsonLintError(''), null);
	assert.equal(jsonLintError(' \n\t'), null);
	assert.equal(jsonLintError('{"a": [1]}'), null);
	assert.deepEqual(jsonLintError('[1 2]'), { from: 3, to: 4, message: 'Invalid JSON: line 1, col 4' });
	// A missing token marks the next character, or the last one at the end.
	assert.deepEqual(jsonLintError('[1,]'), { from: 3, to: 4, message: 'Invalid JSON: line 1, col 4' });
	assert.deepEqual(jsonLintError('{"a":'), { from: 4, to: 5, message: 'Invalid JSON: line 1, col 6' });
});

test('lint reads the given tree and document (the editor\'s), not a fresh parse', () => {
	const lint = (text, tree = jsonLanguage.parser.parse(text)) => jsonTreeLintError(tree, Text.of(text.split('\n')));
	assert.deepEqual(lint('{\n  "a": 1\n  "b": 2\n}'), { from: 13, to: 14, message: 'Invalid JSON: line 3, col 3' });
	assert.equal(lint('[1]'), null);
	// Blank stays clean whatever the tree says (an empty JSON note is not an error).
	assert.equal(lint(' \n ', jsonLanguage.parser.parse('[1 2]')), null);
	// The tree is the source of truth: a valid tree for invalid text lints nothing.
	assert.equal(lint('[1 2]', jsonLanguage.parser.parse('[1,2]')), null);
});

// A JSON document far larger than one parse slice, with its only error at the very end.
const bigJson = (tail) => '[' + Array.from({ length: 20000 }, (_, i) => `{"id": ${i}, "name": "item ${i}"}`).join(',') + tail;

test('state lint parses in slices, yielding between them, until the tree is complete', async () => {
	const text = bigJson(' x]');
	const state = EditorState.create({ doc: text, extensions: json() });
	let pauses = 0;
	const error = await jsonStateLintError(() => state, 1, async () => void pauses++);
	assert.ok(pauses > 1, `expected several slices, got ${pauses + 1}`);
	assert.deepEqual(error, jsonLintError(text));
	assert.equal(error.from, text.length - 2);
});

test('state lint gives up when the document changes between slices', async () => {
	let state = EditorState.create({ doc: bigJson(' x]'), extensions: json() });
	let pauses = 0;
	const pause = async () => {
		pauses++;
		state = state.update({ changes: { from: 0, insert: ' ' } }).state;
	};
	assert.equal(await jsonStateLintError(() => state, 1, pause), null);
	assert.equal(pauses, 1);
});

test('state lint: none for a blank document, one without a parser or a destroyed view; one yield for small ones', async () => {
	const never = async () => assert.fail('must not pause');
	const blank = EditorState.create({ doc: ' \n\t', extensions: json() });
	assert.equal(await jsonStateLintError(() => blank, 1, never), null);
	const plain = EditorState.create({ doc: '[1 2]' });
	assert.equal(await jsonStateLintError(() => plain, 1, never), null);
	assert.equal(await jsonStateLintError(() => null, 1, never), null);
	const small = EditorState.create({ doc: '[1 2]', extensions: json() });
	let pauses = 0;
	assert.deepEqual(await jsonStateLintError(() => small, 50, async () => void pauses++), jsonLintError('[1 2]'));
	assert.equal(pauses, 1, 'yields once between building the tree and walking it');
});

test('state lint drops the result when the document changes after the tree is built', async () => {
	let state = EditorState.create({ doc: '[1 2]', extensions: json() });
	const edit = async () => {
		state = state.update({ changes: { from: 0, insert: ' ' } }).state;
	};
	assert.equal(await jsonStateLintError(() => state, 50, edit), null);
});

test('changedSpan replaces only what differs', () => {
	assert.equal(changedSpan('same', 'same'), null);
	assert.deepEqual(changedSpan('{"a":1}', '{\n  "a": 1\n}'), { from: 1, to: 6, insert: '\n  "a": 1\n' });
	const apply = (s, c) => s.slice(0, c.from) + c.insert + s.slice(c.to);
	for (const [a, b] of [['aaa', 'aa'], ['ab', 'aab'], ['', 'x'], ['x', ''], ['[1]', '[\n  1\n]']]) {
		assert.equal(apply(a, changedSpan(a, b)), b, `${a} -> ${b}`);
	}
});

// ── ```json blocks ─────────────────────────────────────────────────────────────

const md = parser.configure(GFM);

function block(source, pos) {
	const doc = Text.of(source.split('\n'));
	const at = pos ?? source.indexOf('{');
	return { doc, block: findJsonBlock(md.parse(source), doc, at) };
}

function apply(doc, edit) {
	return doc.replace(edit.from, edit.to, Text.of(edit.insert.split('\n'))).toString();
}

test('only the json info string qualifies, in any case', () => {
	assert.equal(isJsonInfo('json'), true);
	assert.equal(isJsonInfo('JSON'), true);
	assert.equal(isJsonInfo(' Json title="x"'), true);
	for (const info of ['jsonc', 'json5', 'js', '', 'ndjson']) assert.equal(isJsonInfo(info), false, info);
});

test('finds the json fence around a position and formats its content in place', () => {
	const source = 'Intro\n\n```json\n{"a":[1,2],"b":{}}\n```\n\nOutro';
	const { doc, block: b } = block(source);
	assert.ok(b);
	assert.equal(b.text, '{"a":[1,2],"b":{}}');
	assert.equal(b.prefix, '');
	assert.equal(b.fenceLineEnd, source.indexOf('\n', source.indexOf('```json')));
	const edit = jsonBlockEdit(b, 'format');
	assert.equal(apply(doc, edit), 'Intro\n\n```json\n{\n  "a": [\n    1,\n    2\n  ],\n  "b": {}\n}\n```\n\nOutro');
	// On the fences themselves too.
	assert.ok(block(source, source.indexOf('```json')).block);
	assert.ok(block(source, source.lastIndexOf('```') + 3).block);
	// Outside: nothing.
	assert.equal(block(source, 2).block, null);
	assert.equal(block(source, source.length).block, null);
});

test('ignores jsonc, other languages, unterminated fences and plain code', () => {
	assert.equal(block('```jsonc\n{"a": 1}\n```').block, null);
	assert.equal(block('```js\n{"a": 1}\n```').block, null);
	assert.equal(block('```\n{"a": 1}\n```').block, null);
	assert.equal(block('```json\n{"a": 1}\n').block, null);
	assert.equal(block('    {"a": 1}').block, null);
});

test('tilde and longer fences qualify', () => {
	const { doc, block: b } = block('~~~~ JSON\n{"a":1}\n~~~~');
	assert.equal(apply(doc, jsonBlockEdit(b, 'format')), '~~~~ JSON\n{\n  "a": 1\n}\n~~~~');
	const longer = block('````json\n[ 1 ]\n````', 10);
	assert.equal(apply(longer.doc, jsonBlockEdit(longer.block, 'minify')), '````json\n[1]\n````');
});

test('preserves list indentation and blockquote prefixes', () => {
	const list = '- item\n\n  ```json\n  {"a":\n     [1]}\n  ```\n- next';
	const l = block(list);
	assert.equal(l.block.prefix, '  ');
	assert.equal(l.block.text, '{"a":\n   [1]}');
	assert.equal(apply(l.doc, jsonBlockEdit(l.block, 'format')), '- item\n\n  ```json\n  {\n    "a": [\n      1\n    ]\n  }\n  ```\n- next');

	const quote = '> ```json\n> [1,\n>2]\n> ```';
	const q = block(quote, quote.indexOf('[1'));
	assert.equal(q.block.prefix, '> ');
	assert.equal(apply(q.doc, jsonBlockEdit(q.block, 'format')), '> ```json\n> [\n>   1,\n>   2\n> ]\n> ```');

	const nested = '- > ```json\n  > {"k": "v"}\n  > ```';
	const n = block(nested);
	assert.equal(n.block.prefix, '  > ');
	assert.equal(apply(n.doc, jsonBlockEdit(n.block, 'minify')), '- > ```json\n  > {"k":"v"}\n  > ```');
});

test('invalid block content is refused with a position relative to the block', () => {
	const { block: b } = block('# Title\n\nText\n\n```json\n{\n  "a": 1,\n}\n```');
	const edit = jsonBlockEdit(b, 'format');
	assert.equal(edit.ok, false);
	assert.deepEqual([edit.line, edit.col], [3, 1]);
	// An empty block has nothing to format.
	const empty = block('```json\n```', 3).block;
	assert.ok(empty);
	assert.equal(empty.text, '');
	assert.equal(jsonBlockEdit(empty, 'format').ok, false);
});
