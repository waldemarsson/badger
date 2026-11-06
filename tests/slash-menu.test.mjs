import assert from 'node:assert/strict';
import test from 'node:test';
import { EditorState } from '@codemirror/state';
import { GFM, parser } from '@lezer/markdown';

const {
	slashTrigger,
	insideInlineCode,
	isCodeAt,
	formatLocalDate,
	SLASH_ITEMS,
	slashInsertion,
	slashItemsAt,
	applySlashItem
} = await import('../src/lib/slash-menu.ts');

const md = parser.configure(GFM);
const item = (id) => SLASH_ITEMS.find((i) => i.id === id);
/** Field placeholders in CodeMirror snippet syntax, in order. */
const fields = (template) => [...template.matchAll(/\$\{([^}]*)\}/g)].map((m) => m[1]);
const fixedDate = new Date(2026, 0, 5, 23, 59); // local time

test('slashTrigger opens at line start and after whitespace', () => {
	assert.deepEqual(slashTrigger('/'), { from: 0, query: '' });
	assert.deepEqual(slashTrigger('hello /'), { from: 6, query: '' });
	assert.deepEqual(slashTrigger('\t/ta'), { from: 1, query: 'ta' });
	assert.deepEqual(slashTrigger('- /task'), { from: 2, query: 'task' });
	assert.deepEqual(slashTrigger('> /'), { from: 2, query: '' });
	assert.deepEqual(slashTrigger('/tab'), { from: 0, query: 'tab' });
});

test('slashTrigger stays closed inside words, URLs and paths', () => {
	assert.equal(slashTrigger('and/or'), null);
	assert.equal(slashTrigger('a/'), null);
	assert.equal(slashTrigger('a/b'), null);
	assert.equal(slashTrigger('https:/'), null);
	assert.equal(slashTrigger('https://'), null);
	assert.equal(slashTrigger('see https://example.com/'), null);
	assert.equal(slashTrigger('/usr/'), null);
	assert.equal(slashTrigger('/usr/bin'), null);
	assert.equal(slashTrigger('[x](/'), null);
	assert.equal(slashTrigger('1/2'), null);
});

test('slashTrigger closes once the query has a non-word character', () => {
	assert.equal(slashTrigger('/ '), null);
	assert.equal(slashTrigger('/tab '), null);
	assert.equal(slashTrigger('/a.b'), null);
	assert.equal(slashTrigger(''), null);
	assert.equal(slashTrigger('no slash'), null);
});

test('slashTrigger stays closed inside inline code, even unclosed', () => {
	assert.equal(slashTrigger('`code /'), null);
	assert.equal(slashTrigger('run `npm /'), null);
	assert.equal(slashTrigger('``a ` /'), null);
	assert.deepEqual(slashTrigger('`code` /'), { from: 7, query: '' });
	assert.deepEqual(slashTrigger('``a`b`` /'), { from: 8, query: '' });
	assert.deepEqual(slashTrigger('\\`not code /'), { from: 11, query: '' });
});

test('insideInlineCode follows backtick runs', () => {
	assert.equal(insideInlineCode(''), false);
	assert.equal(insideInlineCode('`'), true);
	assert.equal(insideInlineCode('`a`'), false);
	assert.equal(insideInlineCode('``a`'), true);
	assert.equal(insideInlineCode('``a``'), false);
	assert.equal(insideInlineCode('\\`'), false);
	assert.equal(insideInlineCode('`a\\`'), false); // no escapes inside code spans
});

test('isCodeAt finds fenced, indented and inline code in the syntax tree', () => {
	const doc = 'Text /\n\n```ts\nconst a = 1 /\n```\n\nSome `x /y` here\n\n    indented /\n\nafter /';
	const tree = md.parse(doc);
	const at = (needle, nth = 0) => {
		let pos = -1;
		for (let i = 0; i <= nth; i++) pos = doc.indexOf(needle, pos + 1);
		return pos;
	};
	assert.equal(isCodeAt(tree, at('/')), false);
	assert.equal(isCodeAt(tree, at('1 /') + 2), true);
	assert.equal(isCodeAt(tree, at('/y')), true);
	assert.equal(isCodeAt(tree, at('indented /') + 9), true);
	assert.equal(isCodeAt(tree, at('after /') + 6), false);
});

test('isCodeAt treats an unclosed fence as code to the end', () => {
	const doc = 'Intro\n\n```\nstill code /';
	assert.equal(isCodeAt(md.parse(doc), doc.length - 1), true);
});

test('formatLocalDate is YYYY-MM-DD in local time, zero-padded', () => {
	assert.equal(formatLocalDate(fixedDate), '2026-01-05');
	assert.equal(formatLocalDate(new Date(2026, 10, 30, 0, 0)), '2026-11-30');
	assert.equal(formatLocalDate(new Date(999, 8, 9)), '0999-09-09');
});

test('menu items are table, code block, task list, callout and date', () => {
	assert.deepEqual(
		SLASH_ITEMS.map((i) => i.id),
		['table', 'code', 'task', 'callout', 'date']
	);
	for (const i of SLASH_ITEMS) {
		assert.ok(i.label, `${i.id} has a label`);
		assert.ok(i.detail, `${i.id} has a detail`);
	}
	assert.equal(new Set(SLASH_ITEMS.map((i) => i.label)).size, SLASH_ITEMS.length);
});

test('table snippet: header, delimiter, two rows, 3 columns, a stop per cell, then an exit stop', () => {
	const t = item('table').template(fixedDate);
	const lines = t.split('\n');
	assert.equal(lines[1], '| --- | --- | --- |');
	const rows = [lines[0], lines[2], lines[3]];
	for (const row of rows) {
		assert.match(row, /^\| .* \|$/);
		assert.equal(row.split(' | ').length, 3, row);
	}
	assert.deepEqual(lines.slice(4), ['', '${}']);
	const f = fields(t);
	assert.equal(f.length, 10); // 9 cells + exit
	assert.deepEqual(f.slice(0, 3), ['Column 1', 'Column 2', 'Column 3']);
	// Named fields with equal names are linked in CodeMirror; header names must differ.
	assert.equal(new Set(f.slice(0, 3)).size, 3);
	assert.ok(f.slice(3).every((name) => name === ''));
});

test('code block snippet: fence with a language stop, then a body stop', () => {
	const t = item('code').template(fixedDate);
	assert.equal(t, '```${}\n${}\n```');
});

test('task, callout and date insert their text with the cursor after it', () => {
	assert.equal(item('task').template(fixedDate), '- [ ] ${}');
	assert.equal(item('callout').template(fixedDate), '> [!note] ${}');
	assert.equal(item('date').template(fixedDate), '2026-01-05${}');
});

test('slashInsertion: inline items insert as-is', () => {
	assert.deepEqual(slashInsertion(item('date'), 'Due ', 'prev', fixedDate), {
		deleteBefore: 0,
		template: '2026-01-05${}'
	});
});

test('slashInsertion: block items at line start insert as-is', () => {
	assert.deepEqual(slashInsertion(item('code'), '', null, fixedDate), {
		deleteBefore: 0,
		template: '```${}\n${}\n```'
	});
	assert.deepEqual(slashInsertion(item('task'), '  ', 'text', fixedDate), {
		deleteBefore: 0,
		template: '- [ ] ${}'
	});
});

test('slashInsertion: block items after text start a new line and drop the trailing space', () => {
	assert.deepEqual(slashInsertion(item('task'), 'Buy milk  ', null, fixedDate), {
		deleteBefore: 2,
		template: '\n- [ ] ${}'
	});
	assert.deepEqual(slashInsertion(item('callout'), 'x ', null, fixedDate), {
		deleteBefore: 1,
		template: '\n> [!note] ${}'
	});
});

test('slashInsertion: a table needs a blank line before it', () => {
	const table = item('table').template(fixedDate);
	assert.equal(slashInsertion(item('table'), '', 'A paragraph', fixedDate).template, '\n' + table);
	assert.equal(slashInsertion(item('table'), '', '', fixedDate).template, table);
	assert.equal(slashInsertion(item('table'), '', '   ', fixedDate).template, table);
	assert.equal(slashInsertion(item('table'), '', null, fixedDate).template, table);
	assert.deepEqual(slashInsertion(item('table'), 'Text ', null, fixedDate), {
		deleteBefore: 1,
		template: '\n\n' + table
	});
});

test('slashInsertion: after list or quote markers, one-line items stay in that container', () => {
	assert.deepEqual(slashInsertion(item('callout'), '1. ', null, fixedDate), {
		deleteBefore: 0,
		template: '> [!note] ${}'
	});
	assert.deepEqual(slashInsertion(item('callout'), '> ', null, fixedDate), {
		deleteBefore: 0,
		template: '> [!note] ${}'
	});
});

test('slashInsertion: a table or code block replaces an empty list item or quote line', () => {
	const code = item('code').template(fixedDate);
	const table = item('table').template(fixedDate);
	assert.deepEqual(slashInsertion(item('code'), '> ', '', fixedDate), { deleteBefore: 2, template: code });
	// Indentation stays: snippet() indents the following lines with it too.
	assert.deepEqual(slashInsertion(item('code'), '  - ', 'prev', fixedDate), { deleteBefore: 2, template: code });
	assert.deepEqual(slashInsertion(item('code'), '\t> ', 'prev', fixedDate), { deleteBefore: 2, template: code });
	assert.deepEqual(slashInsertion(item('table'), '- ', '- prev', fixedDate), { deleteBefore: 2, template: '\n' + table });
});

test('slashInsertion: a task in a bullet item replaces the bullet instead of nesting', () => {
	assert.deepEqual(slashInsertion(item('task'), '- ', 'prev', fixedDate), { deleteBefore: 2, template: '- [ ] ${}' });
	assert.deepEqual(slashInsertion(item('task'), '  * [ ] ', null, fixedDate), {
		deleteBefore: 6,
		template: '- [ ] ${}'
	});
	assert.deepEqual(slashInsertion(item('task'), '> - ', null, fixedDate), { deleteBefore: 2, template: '- [ ] ${}' });
	// Ordered items keep their number and get just the checkbox.
	assert.deepEqual(slashInsertion(item('task'), '1. ', null, fixedDate), { deleteBefore: 0, template: '[ ] ${}' });
	assert.deepEqual(slashInsertion(item('task'), '2) [x] ', null, fixedDate), { deleteBefore: 4, template: '[ ] ${}' });
});

/** Runs the real snippet() for `/<query>` at the end of `doc`; returns the new doc. */
function insert(doc, id) {
	const target = {
		state: EditorState.create({ doc }),
		dispatch(tr) {
			this.state = tr.state;
		}
	};
	const slash = doc.lastIndexOf('/');
	applySlashItem(target, item(id), slash, doc.length, fixedDate);
	return target.state.doc.toString();
}
const nodes = (doc, name) => {
	const found = [];
	md.parse(doc).iterate({ enter: (n) => void (n.name === name && found.push(doc.slice(n.from, n.to))) });
	return found;
};

test('applySlashItem: code block from a nested list item keeps one indent and closes', () => {
	const doc = insert('- a\n  - b\n    - /code', 'code');
	assert.equal(doc, '- a\n  - b\n    ```\n    \n    ```');
	assert.deepEqual(nodes(doc, 'FencedCode'), ['```\n    \n    ```']);
});

test('applySlashItem: table from a tab-indented list item stays a table', () => {
	const doc = insert('- a\n\t- /table', 'table');
	const lines = doc.split('\n');
	assert.equal(lines[1], '\t');
	assert.equal(lines[2], '\t| Column 1 | Column 2 | Column 3 |');
	assert.ok(lines.slice(2, 6).every((l) => l.startsWith('\t| ') && l.endsWith(' |')), doc);
	assert.equal(nodes(doc, 'Table').length, 1);
	assert.equal(nodes(doc, 'TableRow').length, 2);
});

test('applySlashItem: code block and table from a blockquote line', () => {
	const code = insert('> /code', 'code');
	assert.equal(code, '```\n\n```');
	assert.equal(nodes(code, 'FencedCode').length, 1);
	const table = insert('> quote\n> /table', 'table');
	assert.ok(table.startsWith('> quote\n\n| Column 1 |'), table);
	assert.equal(nodes(table, 'Table').length, 1);
});

test('applySlashItem: the first field is selected and the slash is gone', () => {
	const target = {
		state: EditorState.create({ doc: 'Intro\n\n/tab' }),
		dispatch(tr) {
			this.state = tr.state;
		}
	};
	applySlashItem(target, item('table'), 7, 11, fixedDate);
	const { from, to } = target.state.selection.main;
	assert.equal(target.state.sliceDoc(from, to), 'Column 1');
	assert.ok(!target.state.doc.toString().includes('/'));
});

test('slashItemsAt: only inline items inside a raw table row', () => {
	const doc = '| a | b |\n| --- | --- |\n| x | / |\n\nText /';
	const tree = md.parse(doc);
	const ids = (pos) => slashItemsAt(tree, pos).map((i) => i.id);
	assert.deepEqual(ids(doc.indexOf('/ |')), ['date']);
	assert.deepEqual(ids(doc.indexOf('a')), ['date']);
	assert.deepEqual(ids(doc.indexOf('---')), ['date']);
	assert.deepEqual(ids(doc.lastIndexOf('/')), ['table', 'code', 'task', 'callout', 'date']);
});
