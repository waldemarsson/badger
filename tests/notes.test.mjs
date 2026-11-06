import assert from 'node:assert/strict';
import test from 'node:test';

const {
	sortNotes,
	splitHighlights,
	formatBytes,
	plainRows,
	searchRows,
	upsertNote,
	neighborAfterRemoval,
	parseTimestamp,
	formatNoteDate,
	splitTableRow,
	parseTableAlign,
	parseTable,
	parseInline
} = await import('../src/lib/notes.ts');

const note = (id, updated_at, extra = {}) => ({
	id,
	folder_id: null,
	title: `Note ${id}`,
	excerpt: '',
	pinned: false,
	created_at: '2026-11-01T09:00:00.000',
	updated_at,
	...extra
});

const ids = (rows) => rows.map((r) => r.id);

// ── List order and filter ─────────────────────────────────────────────────────

test('sortNotes: pinned first, then updated_at desc, then id', () => {
	const rows = [
		note(1, '2026-11-05T10:00:00.000'),
		note(2, '2026-11-06T10:00:00.000'),
		note(3, '2026-11-01T10:00:00.000', { pinned: true }),
		note(4, '2026-11-05T10:00:00.000'),
		note(5, '2026-11-02T10:00:00.000', { pinned: true })
	];
	assert.deepEqual(ids(sortNotes(rows)), [5, 3, 2, 1, 4]);
	assert.deepEqual(ids(rows), [1, 2, 3, 4, 5], 'input is not mutated');
});

test('splitHighlights: marked text into hit and plain segments', () => {
	assert.deepEqual(splitHighlights('a \u0002Budget\u0003 plan'), [
		{ text: 'a ', hit: false },
		{ text: 'Budget', hit: true },
		{ text: ' plan', hit: false }
	]);
	assert.deepEqual(splitHighlights('\u0002x\u0003\u0002y\u0003'), [
		{ text: 'x', hit: true },
		{ text: 'y', hit: true }
	]);
	assert.deepEqual(splitHighlights('…möte 🎉 \u0002är\u0003'), [
		{ text: '…möte 🎉 ', hit: false },
		{ text: 'är', hit: true }
	]);
	assert.deepEqual(splitHighlights(''), []);
	assert.deepEqual(splitHighlights('plain'), [{ text: 'plain', hit: false }]);
});

test('searchRows: hit order, current summaries, deleted and renamed notes', () => {
	const notes = [
		note(1, 'b', { title: 'Budget', excerpt: 'numbers' }),
		note(2, 'a', { title: 'Renamed', excerpt: 'new' }),
		note(3, 'c', { title: 'Notes', excerpt: 'first line' })
	];
	const hit = (n, title_marked, snippet = '') => ({ ...n, title_marked, snippet });
	const rows = searchRows(notes, [
		hit(note(3, 'c', { title: 'Notes' }), 'Notes', 'the \u0002budget\u0003'),
		hit(note(1, 'b', { title: 'Budget' }), '\u0002Budget\u0003'),
		hit(note(2, 'a', { title: 'Budget old' }), '\u0002Budget\u0003 old'),
		hit(note(9, 'z', { title: 'Gone' }), '\u0002Gone\u0003')
	]);
	assert.deepEqual(rows.map((r) => r.note.id), [3, 1, 2], 'hit order; deleted note 9 dropped');
	assert.equal(rows[0].note, notes[2], 'current summary, not the hit copy');
	assert.deepEqual(rows[0].snippet, [
		{ text: 'the ', hit: false },
		{ text: 'budget', hit: true }
	]);
	assert.deepEqual(rows[1].title, [{ text: 'Budget', hit: true }]);
	assert.equal(rows[1].snippet, null, 'no snippet → excerpt');
	assert.deepEqual(rows[2].title, [{ text: 'Renamed', hit: false }], 'renamed: marks dropped');
});

test('plainRows: every note, unhighlighted', () => {
	const rows = plainRows([note(1, 'x', { title: 'A' })]);
	assert.deepEqual(rows[0].title, [{ text: 'A', hit: false }]);
	assert.equal(rows[0].snippet, null);
});

test('upsertNote replaces by id or adds, and keeps list order', () => {
	const rows = [note(1, '2026-11-05T10:00:00.000'), note(2, '2026-11-04T10:00:00.000')];
	const updated = upsertNote(rows, note(2, '2026-11-06T10:00:00.000', { title: 'Renamed' }));
	assert.deepEqual(ids(updated), [2, 1]);
	assert.equal(updated[0].title, 'Renamed');
	assert.deepEqual(ids(upsertNote(rows, note(3, '2026-11-01T10:00:00.000'))), [1, 2, 3]);
	assert.equal(rows.length, 2, 'input is not mutated');
});

test('neighborAfterRemoval: next in list, else previous, else null', () => {
	assert.equal(neighborAfterRemoval([4, 7, 9], 7), 9);
	assert.equal(neighborAfterRemoval([4, 7, 9], 9), 7);
	assert.equal(neighborAfterRemoval([4], 4), null);
	assert.equal(neighborAfterRemoval([4, 7], 99), null);
});

// ── Dates ───────────────────────────────────────────────────────────────────

test('parseTimestamp reads the stored local timestamp as local time', () => {
	const d = parseTimestamp('2026-11-06T09:05:03.250');
	assert.deepEqual(
		[d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds()],
		[2026, 10, 6, 9, 5, 3]
	);
	assert.ok(Number.isNaN(parseTimestamp('garbage').getTime()));
});

test('formatNoteDate: date and time this year, date with year otherwise, empty when invalid', () => {
	const now = new Date(2026, 10, 6, 12, 0, 0);
	const thisYear = new Date(2026, 9, 3, 14, 7);
	assert.equal(
		formatNoteDate('2026-10-03T14:07:00.000', now),
		`${thisYear.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}, ${thisYear.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}`
	);
	assert.equal(
		formatNoteDate('2025-12-24T08:00:00.000', now),
		new Date(2025, 11, 24).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
	);
	assert.equal(formatNoteDate('not a date', now), '');
});

// ── Tables ──────────────────────────────────────────────────────────────────

test('splitTableRow: trims outer pipes and cells, keeps offsets of the cell text', () => {
	assert.deepEqual(splitTableRow('| a | bc |'), [
		{ text: 'a', from: 2, to: 3 },
		{ text: 'bc', from: 6, to: 8 }
	]);
	assert.deepEqual(splitTableRow('a|b'), [
		{ text: 'a', from: 0, to: 1 },
		{ text: 'b', from: 2, to: 3 }
	]);
	assert.deepEqual(splitTableRow('|  |x|'), [
		{ text: '', from: 1, to: 1 },
		{ text: 'x', from: 4, to: 5 }
	]);
});

test('splitTableRow: escaped pipes stay inside the cell', () => {
	assert.deepEqual(
		splitTableRow('| a \\| b | c |').map((c) => c.text),
		['a \\| b', 'c']
	);
});

test('parseTableAlign reads colons per column', () => {
	assert.deepEqual(parseTableAlign('| :-- | :-: | --: | --- |'), ['left', 'center', 'right', null]);
	assert.deepEqual(parseTableAlign(':---|---:'), ['left', 'right']);
});

test('parseTable: header, alignment and rows with absolute offsets', () => {
	const src = '| A | B |\n|:--|--:|\n| 1 | 2 |';
	const t = parseTable(src);
	assert.deepEqual(
		t.header.map((c) => c.text),
		['A', 'B']
	);
	assert.deepEqual(t.align, ['left', 'right']);
	assert.equal(t.rows.length, 1);
	assert.deepEqual(t.rows[0], [
		{ text: '1', from: 22, to: 23 },
		{ text: '2', from: 26, to: 27 }
	]);
	assert.equal(src.slice(t.rows[0][1].from, t.rows[0][1].to), '2');
	assert.equal(src.slice(t.header[0].from, t.header[0].to), 'A');
});

test('parseTable: malformed rows are padded or truncated to the header width; never throws', () => {
	const src = '| A | B | C |\n|---|---|---|\n| 1 |\n| 1 | 2 | 3 | 4 | 5 |';
	const t = parseTable(src);
	assert.deepEqual(
		t.rows.map((r) => r.map((c) => c.text)),
		[
			['1', '', ''],
			['1', '2', '3']
		]
	);
	// Padding cells point at the end of their row, so a click lands on that line.
	assert.equal(t.rows[0][2].from, src.indexOf('| 1 |') + '| 1 |'.length);
	assert.deepEqual(t.align, [null, null, null]);

	assert.doesNotThrow(() => parseTable(''));
	assert.doesNotThrow(() => parseTable('| only header |'));
	assert.deepEqual(parseTable('| only header |').rows, []);
	// A short delimiter row still yields one alignment entry per header column.
	assert.deepEqual(parseTable('| A | B |\n| :-: |').align, ['center', null]);
});

// ── Inline cell parsing ─────────────────────────────────────────────────────

const flat = (spans) =>
	spans.map((s) => {
		const marks = ['bold', 'italic', 'code', 'strike', 'link'].filter((m) => s[m]);
		return marks.length ? `${marks.join('+')}:${s.text}` : s.text;
	});

test('parseInline: plain text and empty input', () => {
	assert.deepEqual(flat(parseInline('hello world')), ['hello world']);
	assert.deepEqual(parseInline(''), []);
});

test('parseInline: bold, italic, strike, code', () => {
	assert.deepEqual(flat(parseInline('a **b** c')), ['a ', 'bold:b', ' c']);
	assert.deepEqual(flat(parseInline('__b__')), ['bold:b']);
	assert.deepEqual(flat(parseInline('*i* and _j_')), ['italic:i', ' and ', 'italic:j']);
	assert.deepEqual(flat(parseInline('~~gone~~')), ['strike:gone']);
	assert.deepEqual(flat(parseInline('run `npm *test*` now')), ['run ', 'code:npm *test*', ' now']);
	assert.deepEqual(flat(parseInline('``a ` b``')), ['code:a ` b']);
});

test('parseInline: nesting and triple delimiters', () => {
	assert.deepEqual(flat(parseInline('**bold *both***')), ['bold:bold ', 'bold+italic:both']);
	assert.deepEqual(flat(parseInline('***x***')), ['bold+italic:x']);
	assert.deepEqual(flat(parseInline('~~**x**~~')), ['bold+strike:x']);
});

test('parseInline: links show their text only; images stay raw', () => {
	assert.deepEqual(flat(parseInline('see [the docs](https://x.dev/a_b) now')), ['see ', 'link:the docs', ' now']);
	assert.deepEqual(flat(parseInline('[**b**](u)')), ['bold+link:b']);
	assert.deepEqual(flat(parseInline('<https://x.dev>')), ['link:https://x.dev']);
	assert.deepEqual(flat(parseInline('![alt](img.png)')), ['![alt](img.png)']);
	assert.deepEqual(flat(parseInline('[not a link]')), ['[not a link]']);
});

test('parseInline: unmatched delimiters, intraword underscores and escapes stay literal', () => {
	assert.deepEqual(flat(parseInline('**open')), ['**open']);
	assert.deepEqual(flat(parseInline('snake_case_name')), ['snake_case_name']);
	assert.deepEqual(flat(parseInline('2 * 3 * 4')), ['2 * 3 * 4']);
	assert.deepEqual(flat(parseInline('\\*not\\* a \\| pipe')), ['*not* a | pipe']);
	assert.deepEqual(flat(parseInline('`unclosed')), ['`unclosed']);
	assert.deepEqual(flat(parseInline('emoji 🎉 **å**')), ['emoji 🎉 ', 'bold:å']);
});

test('formatBytes: B, KB, MB with one decimal under 10', () => {
	assert.equal(formatBytes(0), '0 B');
	assert.equal(formatBytes(1023), '1023 B');
	assert.equal(formatBytes(1536), '1.5 KB');
	assert.equal(formatBytes(48 * 1024), '48 KB');
	assert.equal(formatBytes(12.4 * 1024 * 1024), '12 MB');
	assert.equal(formatBytes(3.25 * 1024 * 1024), '3.3 MB');
	assert.equal(formatBytes(2048 * 1024 ** 3), '2048 GB');
});
