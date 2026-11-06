import assert from 'node:assert/strict';
import test from 'node:test';

const { NoteSaveQueue, SAVE_DELAY_MS } = await import('../src/lib/note-save-queue.ts');

/** Lets pending promise callbacks run (setImmediate is not mocked). */
const settle = () => new Promise((resolve) => setImmediate(resolve));

function deferred() {
	let resolve;
	let reject;
	const promise = new Promise((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

/** A queue whose saves are recorded and settled by the test. */
function harness(t, options = {}) {
	t.mock.timers.enable({ apis: ['setTimeout'] });
	const calls = [];
	let inFlight = 0;
	let maxInFlight = 0;
	const saved = [];
	let changes = 0;
	const queue = new NoteSaveQueue({
		save: (id, body) => {
			const d = deferred();
			inFlight++;
			maxInFlight = Math.max(maxInFlight, inFlight);
			calls.push({ id, body, ...d });
			return d.promise.finally(() => inFlight--);
		},
		onSaved: (id, result) => saved.push({ id, result }),
		onChange: () => changes++,
		...options
	});
	return {
		queue,
		calls,
		saved,
		maxInFlight: () => maxInFlight,
		changes: () => changes,
		tick: (ms) => t.mock.timers.tick(ms)
	};
}

test('debounce: one save of the latest body after the quiet period', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'a');
	h.tick(500);
	h.queue.edit(1, 'ab');
	h.tick(500);
	h.queue.edit(1, 'abc');
	assert.equal(h.calls.length, 0);
	assert.equal(h.queue.status(1), 'pending');
	h.tick(SAVE_DELAY_MS - 1);
	assert.equal(h.calls.length, 0);
	h.tick(1);
	assert.deepEqual(
		h.calls.map((c) => [c.id, c.body]),
		[[1, 'abc']]
	);
	assert.equal(h.queue.status(1), 'saving');
	h.calls[0].resolve({ id: 1, excerpt: 'abc' });
	await settle();
	assert.equal(h.queue.status(1), 'saved');
	assert.deepEqual(h.saved, [{ id: 1, result: { id: 1, excerpt: 'abc' } }]);
	assert.equal(h.queue.busy, false);
	assert.ok(h.changes() > 0, 'status changes are reported');
});

test('unchanged content is never sent', async (t) => {
	const h = harness(t);
	h.queue.track(1, 'same');
	h.queue.edit(1, 'same');
	assert.equal(h.queue.busy, false);
	h.queue.edit(1, 'same!');
	h.queue.edit(1, 'same'); // typed then undone
	h.tick(SAVE_DELAY_MS);
	assert.equal(await h.queue.flush(), true);
	assert.equal(h.calls.length, 0);
	assert.equal(h.queue.status(1), 'saved');
});

test('single in-flight save per note; the latest queued body wins', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'a');
	h.tick(SAVE_DELAY_MS);
	assert.equal(h.calls.length, 1);
	h.queue.edit(1, 'ab');
	h.queue.edit(1, 'abc');
	h.tick(SAVE_DELAY_MS);
	assert.equal(h.calls.length, 1, 'no second save while one is in flight');
	h.calls[0].resolve({});
	await settle();
	assert.deepEqual(
		h.calls.map((c) => c.body),
		['a', 'abc']
	);
	h.calls[1].resolve({});
	await settle();
	assert.equal(h.maxInFlight(), 1);
	assert.equal(h.queue.busy, false);
});

test('reverting to the saved body while a save is in flight sends the revert', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'x');
	h.tick(SAVE_DELAY_MS);
	h.queue.edit(1, ''); // undo while 'x' is being written
	h.calls[0].resolve({});
	await settle();
	assert.deepEqual(
		h.calls.map((c) => c.body),
		['x', '']
	);
	h.calls[1].resolve({});
	await settle();
	assert.equal(h.queue.busy, false);
});

test('flush saves every pending note now and waits for in-flight saves', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.track(2, '');
	h.queue.edit(1, 'one');
	h.tick(SAVE_DELAY_MS); // note 1 in flight
	h.queue.edit(2, 'two');
	h.queue.edit(1, 'one more');
	let done = null;
	const flushing = h.queue.flush().then((ok) => (done = ok));
	assert.deepEqual(
		h.calls.map((c) => [c.id, c.body]),
		[
			[1, 'one'],
			[2, 'two']
		]
	);
	h.calls[0].resolve({});
	h.calls[1].resolve({});
	await settle();
	assert.equal(done, null, 'still waiting for the queued body of note 1');
	assert.deepEqual(h.calls[2] && [h.calls[2].id, h.calls[2].body], [1, 'one more']);
	h.calls[2].resolve({});
	await flushing;
	assert.equal(done, true);
	h.tick(SAVE_DELAY_MS * 2);
	assert.equal(h.calls.length, 3, 'the cancelled debounce does not save again');
});

test('failed saves stay dirty, show an error and are retried on every edit and flush', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.track(2, '');
	h.queue.edit(1, 'draft');
	const first = h.queue.flush();
	h.calls[0].reject('disk full');
	assert.equal(await first, false);
	assert.equal(h.queue.status(1), 'error');
	assert.equal(h.queue.error(1), 'disk full');
	assert.deepEqual(h.queue.failed(), [{ id: 1, error: 'disk full' }]);
	assert.equal(h.queue.busy, true);

	// An edit in another note retries note 1 too.
	h.queue.edit(2, 'other');
	h.tick(SAVE_DELAY_MS);
	assert.deepEqual(
		h.calls.slice(1).map((c) => [c.id, c.body]),
		[
			[1, 'draft'],
			[2, 'other']
		]
	);
	h.calls[1].reject('still full');
	h.calls[2].resolve({});
	await settle();
	assert.equal(h.queue.error(1), 'still full');
	assert.equal(h.queue.status(2), 'saved');

	// A flush retries, and success clears the error.
	const retry = h.queue.flush();
	assert.equal(h.calls[3].body, 'draft');
	h.calls[3].resolve({});
	assert.equal(await retry, true);
	assert.equal(h.queue.status(1), 'saved');
	assert.equal(h.queue.error(1), null);
	assert.deepEqual(h.queue.failed(), []);
	assert.equal(h.queue.busy, false);
});

test('undoing a failed edit back to the saved body clears the error', async (t) => {
	const h = harness(t);
	h.queue.track(1, 'stored');
	h.queue.edit(1, 'stored!');
	const first = h.queue.flush();
	h.calls[0].reject('disk full');
	assert.equal(await first, false);
	assert.equal(h.queue.status(1), 'error');

	h.queue.edit(1, 'stored');
	assert.equal(h.queue.status(1), 'saved');
	assert.equal(h.queue.error(1), null);
	assert.deepEqual(h.queue.failed(), []);
	assert.equal(h.queue.busy, false);
	assert.equal(await h.queue.flush(), true, 'leaving is no longer blocked');
	assert.equal(h.calls.length, 1, 'nothing is sent');
});

test('a "Note not found" response is ignored and drops the note', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'late');
	const flushing = h.queue.flush();
	h.calls[0].reject('Note not found');
	assert.equal(await flushing, true);
	assert.equal(h.queue.error(1), null);
	assert.equal(h.queue.busy, false);
	assert.deepEqual(h.saved, []);
});

test('forget cancels a pending save and ignores a late response', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'never sent');
	h.queue.forget(1);
	h.tick(SAVE_DELAY_MS);
	assert.equal(h.calls.length, 0);
	assert.equal(h.queue.busy, false);

	h.queue.track(2, '');
	h.queue.edit(2, 'sent');
	h.tick(SAVE_DELAY_MS);
	h.queue.forget(2);
	assert.equal(h.queue.busy, false, 'a forgotten in-flight save does not block leaving');
	h.calls[0].reject('boom');
	await settle();
	assert.equal(h.queue.error(2), null);
	assert.deepEqual(h.queue.failed(), []);

	h.queue.track(3, '');
	h.queue.edit(3, 'x');
	h.tick(SAVE_DELAY_MS);
	h.queue.forget(3);
	h.calls[1].resolve({ id: 3 });
	await settle();
	assert.deepEqual(h.saved, [], 'onSaved is not called for a forgotten note');
});

test('track does not overwrite the baseline while a save is in flight', async (t) => {
	const h = harness(t);
	h.queue.track(1, 'old');
	h.queue.edit(1, 'new');
	h.tick(SAVE_DELAY_MS);
	h.queue.track(1, 'old'); // re-opened from a stale read
	h.calls[0].resolve({});
	await settle();
	h.queue.edit(1, 'new');
	h.tick(SAVE_DELAY_MS);
	assert.equal(h.calls.length, 1, 'body equals the last saved body');
});

test('dispose stops the debounce timer', (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'x');
	h.queue.dispose();
	h.tick(SAVE_DELAY_MS);
	assert.equal(h.calls.length, 0);
});

test('pause stops the debounce but keeps unsaved bodies for the next flush', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.edit(1, 'kept');
	h.queue.pause();
	h.tick(SAVE_DELAY_MS);
	assert.equal(h.calls.length, 0);
	assert.equal(h.queue.status(1), 'pending');
	assert.equal(h.queue.busy, true);

	const flushing = h.queue.flush();
	assert.deepEqual(
		h.calls.map((c) => [c.id, c.body]),
		[[1, 'kept']]
	);
	h.calls[0].resolve({});
	assert.equal(await flushing, true);
});

test('flush resolves true when edits arrive mid-flush but nothing failed', async (t) => {
	const h = harness(t);
	h.queue.track(1, '');
	h.queue.track(2, '');
	h.queue.edit(1, 'one');
	const flushing = h.queue.flush();
	h.queue.edit(2, 'typed during the flush'); // not part of this flush
	h.calls[0].resolve({});
	assert.equal(await flushing, true, 'pending edits elsewhere are not a save failure');
	assert.equal(h.queue.status(2), 'pending');
	assert.equal(h.queue.busy, true);
});
