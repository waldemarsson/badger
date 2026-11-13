import assert from 'node:assert/strict';
import test from 'node:test';

const {
	beginRun,
	clearOutput,
	consoleEntries,
	emptyRunStore,
	endRun,
	errorLocation,
	errorTitle,
	filterScripts,
	forgetScript,
	memberCompletions,
	neighbourAfterRemoval,
	noteRunId,
	noteTitleContext,
	PLAYGROUND_GLOBALS,
	quoteTitle,
	resetOutputs,
	runSummary,
	sortScripts,
	upsertScript
} = await import('../src/lib/playground.ts');

const s = (id, title, updated_at) => ({ id, title, updated_at });
const outcome = (over = {}) => ({ lines: [], value: null, error: null, truncated: false, duration_ms: 3, ...over });

// ── Script list ──

test('scripts sort by last change, newest id first on ties', () => {
	const list = [s(1, 'a', '2026-01-01T00:00:00.000'), s(2, 'b', '2026-03-01T00:00:00.000'), s(3, 'c', '2026-01-01T00:00:00.000')];
	assert.deepEqual(sortScripts(list).map((x) => x.id), [2, 3, 1]);
	assert.deepEqual(list.map((x) => x.id), [1, 2, 3], 'input untouched');
});

test('upsertScript replaces in place and re-sorts (an edit moves it to the top)', () => {
	const list = [s(2, 'b', '2026-03-01'), s(1, 'a', '2026-01-01')];
	const next = upsertScript(list, { ...s(1, 'A', '2026-04-01'), body: 'dropped', created_at: 'x' });
	assert.deepEqual(next, [s(1, 'A', '2026-04-01'), s(2, 'b', '2026-03-01')]);
	assert.deepEqual(upsertScript([], s(5, 'new', '2026-01-01')), [s(5, 'new', '2026-01-01')]);
});

test('filterScripts matches every word case-insensitively', () => {
	const list = [s(1, 'Tally invoices', 'x'), s(2, 'Åsa report', 'x'), s(3, 'Invoice cleanup', 'x')];
	assert.deepEqual(filterScripts(list, '  '), list);
	assert.deepEqual(filterScripts(list, 'INVOICE').map((x) => x.id), [1, 3]);
	assert.deepEqual(filterScripts(list, 'invoice tal').map((x) => x.id), [1]);
	assert.deepEqual(filterScripts(list, 'åsa').map((x) => x.id), [2]);
	assert.deepEqual(filterScripts(list, 'zzz'), []);
});

test('neighbourAfterRemoval picks the next, then the previous row', () => {
	assert.equal(neighbourAfterRemoval([1, 2, 3], 2), 3);
	assert.equal(neighbourAfterRemoval([1, 2, 3], 3), 2);
	assert.equal(neighbourAfterRemoval([1], 1), null);
	assert.equal(neighbourAfterRemoval([4, 5], 9), 4);
});

// ── Run store ──

test('one run at a time; its output lands on the script that started it', () => {
	const store = emptyRunStore();
	assert.equal(beginRun(store, 7, 100), true);
	assert.equal(beginRun(store, 8, 101), false, 'second run refused');
	assert.deepEqual(store.running, { scriptId: 7, runId: null, startedAt: 100 });
	noteRunId(store, 42);
	noteRunId(store, 43);
	assert.equal(store.running.runId, 42, 'first id sticks');
	// The user switched to script 8 meanwhile: the output still goes to 7.
	endRun(store, { ok: { run_id: 42, outcome: outcome({ value: '3' }) } }, 200);
	assert.equal(store.running, null);
	assert.deepEqual(store.outputs[7], { outcome: outcome({ value: '3' }), failure: null, at: 200 });
	assert.equal(store.outputs[8], undefined);
	assert.equal(beginRun(store, 8, 300), true, 'free again');
});

test('a run that could not start records its failure', () => {
	const store = emptyRunStore();
	beginRun(store, 1, 0);
	endRun(store, { failure: 'A script is already running' }, 5);
	assert.deepEqual(store.outputs[1], { outcome: null, failure: 'A script is already running', at: 5 });
	assert.deepEqual(consoleEntries(store.outputs[1]), [
		{ kind: 'error', title: 'Could not run', message: 'A script is already running', line: null, col: null, errorKind: 'user' }
	]);
});

test('deleting the running script discards its output when the run ends', () => {
	const store = emptyRunStore();
	store.outputs[3] = { outcome: outcome(), failure: null, at: 1 };
	beginRun(store, 3, 10);
	forgetScript(store, 3);
	assert.equal(store.outputs[3], undefined);
	assert.equal(store.running.scriptId, null, 'still running');
	endRun(store, { ok: { run_id: 1, outcome: outcome({ value: 'late' }) } }, 20);
	assert.deepEqual(store.outputs, {});
	assert.equal(store.running, null);
});

test('clearOutput and resetOutputs', () => {
	const store = emptyRunStore();
	store.outputs[1] = { outcome: outcome(), failure: null, at: 1 };
	store.outputs[2] = { outcome: outcome(), failure: null, at: 1 };
	clearOutput(store, 1);
	assert.deepEqual(Object.keys(store.outputs), ['2']);
	beginRun(store, 2, 5);
	resetOutputs(store);
	assert.deepEqual(store.outputs, {});
	assert.equal(store.running.scriptId, null, 'a restore orphans the running script');
	endRun(store, { ok: { run_id: 1, outcome: outcome() } }, 6);
	assert.deepEqual(store.outputs, {});
});

test('endRun without a run in progress is a no-op', () => {
	const store = emptyRunStore();
	endRun(store, { failure: 'x' }, 1);
	assert.deepEqual(store, emptyRunStore());
});

// ── Output shaping ──

test('consoleEntries orders lines, truncation marker, value, then error', () => {
	const out = {
		outcome: outcome({
			lines: [
				{ level: 'log', text: 'a' },
				{ level: 'warn', text: 'b' }
			],
			truncated: true,
			value: '42',
			error: { kind: 'timeout', message: 'Script timed out after 10 s', line: null, col: null }
		}),
		failure: null,
		at: 0
	};
	assert.deepEqual(consoleEntries(out), [
		{ kind: 'line', level: 'log', text: 'a' },
		{ kind: 'line', level: 'warn', text: 'b' },
		{ kind: 'truncated' },
		{ kind: 'value', text: '42' },
		{ kind: 'error', title: 'Timed out', message: 'Script timed out after 10 s', line: null, col: null, errorKind: 'timeout' }
	]);
	assert.deepEqual(consoleEntries(null), []);
	assert.deepEqual(consoleEntries(undefined), []);
	assert.deepEqual(consoleEntries({ outcome: outcome(), failure: null, at: 0 }), [], 'empty run, empty console');
	assert.deepEqual(consoleEntries({ outcome: outcome({ value: '' }), failure: null, at: 0 }), [{ kind: 'value', text: '' }], 'an empty string value still shows');
});

test('error titles and locations', () => {
	assert.equal(errorTitle('user'), 'Error');
	assert.equal(errorTitle('syntax'), 'Syntax error');
	assert.equal(errorTitle('stack'), 'Stack overflow');
	assert.equal(errorTitle('oom'), 'Out of memory');
	assert.equal(errorTitle('timeout'), 'Timed out');
	assert.equal(errorTitle('stopped'), 'Stopped');
	assert.equal(errorLocation({ line: 3, col: 5 }), 'Line 3, column 5');
	assert.equal(errorLocation({ line: 3, col: null }), 'Line 3');
	assert.equal(errorLocation({ line: null, col: null }), null);
});

test('runSummary', () => {
	assert.equal(runSummary(outcome({ duration_ms: 12 })), 'Ran in 12 ms');
	assert.equal(runSummary(outcome({ duration_ms: 1340 })), 'Ran in 1.3 s');
	assert.equal(runSummary(outcome({ duration_ms: 10_020, error: { kind: 'timeout', message: '', line: null, col: null } })), 'Failed after 10 s');
	assert.equal(runSummary(outcome({ duration_ms: 2310, error: { kind: 'stopped', message: 'Stopped', line: null, col: null } })), 'Stopped after 2.3 s');
});

// ── Completion ──

test('noteTitleContext triggers inside the string argument of note(', () => {
	assert.deepEqual(noteTitleContext('const d = note("'), { from: 16, quote: '"', typed: '' });
	assert.deepEqual(noteTitleContext("note('Inv"), { from: 6, quote: "'", typed: 'Inv' });
	assert.deepEqual(noteTitleContext('x(note( `Åsa d'), { from: 9, quote: '`', typed: 'Åsa d' });
	for (const before of [
		'note(',
		'note(x',
		'note("done") + "',
		'keynote("',
		'obj.note("',
		'note("a\\',
		'note("it\'s',
		'// note',
		'$note("'
	]) {
		assert.equal(noteTitleContext(before), null, before);
	}
});

test('quoteTitle escapes for the opening quote', () => {
	assert.equal(quoteTitle('Plan', '"'), 'Plan');
	assert.equal(quoteTitle('Say "hi"', '"'), 'Say \\"hi\\"');
	assert.equal(quoteTitle("It's", "'"), "It\\'s");
	assert.equal(quoteTitle("It's", '"'), "It's");
	assert.equal(quoteTitle('a\\b', "'"), 'a\\\\b');
	assert.equal(quoteTitle('`${x}`', '`'), '\\`\\${x}\\`');
});

test('curated globals and members', () => {
	const labels = PLAYGROUND_GLOBALS.map((g) => g.label);
	assert.ok(labels.includes('note') && labels.includes('console'));
	assert.equal(new Set(labels).size, labels.length, 'no duplicates');
	for (const absent of ['setTimeout', 'fetch', 'window', 'document', 'require', 'structuredClone']) {
		assert.ok(!labels.includes(absent), absent);
	}
	assert.deepEqual(memberCompletions('console'), ['log', 'info', 'warn', 'error', 'debug']);
	assert.ok(memberCompletions('JSON').includes('parse'));
	assert.equal(memberCompletions('foo'), null);
	assert.equal(memberCompletions('toString'), null, 'own keys only');
});
