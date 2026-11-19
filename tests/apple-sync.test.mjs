import assert from 'node:assert/strict';
import test from 'node:test';

const { appleSyncPill, formatLastSync, skippedSummary, TODOS_CHANGED_EVENT, APPLE_SYNC_STATUS_EVENT } =
	await import('../src/lib/apple-sync.ts');

const base = {
	available: true,
	enabled: true,
	access: 'granted',
	last_sync_at: '2026-11-19T12:00:00.000',
	last_error: null,
	paused: null,
	skipped: [],
	reminders_list: 'Badger Reminders',
	todos_list: 'Badger Todos'
};

test('event names match Rust', () => {
	assert.equal(TODOS_CHANGED_EVENT, 'todos-changed');
	assert.equal(APPLE_SYNC_STATUS_EVENT, 'apple-sync-status');
});

test('pill: availability, switch, access, guard and errors in that order', () => {
	assert.deepEqual(appleSyncPill({ ...base, available: false, enabled: false }), { label: 'Not available', tone: 'off' });
	assert.deepEqual(appleSyncPill({ ...base, enabled: false, last_error: 'x' }), { label: 'Off', tone: 'off' });
	assert.deepEqual(appleSyncPill({ ...base, access: 'denied', paused: 'access' }), { label: 'No access', tone: 'warn' });
	assert.deepEqual(appleSyncPill({ ...base, paused: 'empty_fetch', last_error: 'x' }), { label: 'Paused', tone: 'warn' });
	assert.deepEqual(appleSyncPill({ ...base, last_error: 'x' }), { label: 'Error', tone: 'error' });
	assert.deepEqual(appleSyncPill({ ...base, last_sync_at: null }), { label: 'Starting…', tone: 'off' });
	assert.deepEqual(appleSyncPill(base), { label: 'On', tone: 'ok' });
});

test('last sync reads relative, then as a time', () => {
	const now = new Date(2026, 10, 19, 12, 30);
	assert.equal(formatLastSync(null, now), 'Not yet');
	assert.equal(formatLastSync('2026-11-19T12:29:40.000', now), 'Just now');
	assert.equal(formatLastSync('2026-11-19T12:25:00.000', now), '5 min ago');
	assert.equal(formatLastSync('2026-11-19T09:05:00.000', now), 'Today 09:05');
	assert.equal(formatLastSync('2026-11-18T17:45:00.000', now), '2026-11-18 17:45');
	assert.equal(formatLastSync('garbage', now), 'garbage');
});

test('skipped summary counts per reason', () => {
	assert.equal(skippedSummary([]), '');
	const s = (reason) => ({ title: 't', list: 'Badger Reminders', reason });
	assert.equal(skippedSummary([s('needs a date')]), '1 needs a date');
	assert.equal(
		skippedSummary([s('needs a date'), s('needs a title'), s('needs a date')]),
		'2 need a date, 1 needs a title'
	);
});
