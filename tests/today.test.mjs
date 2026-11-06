import assert from 'node:assert/strict';
import test from 'node:test';

const {
	weekDays,
	dailyTotals,
	todayEntries,
	needsAttention,
	inProgress,
	progress,
	parseDayParam,
	sumHours,
	isWeekend,
	formatHours,
	formatDayLabel,
	parseDailyTarget
} = await import('../src/lib/today.ts');

const entry = (id, date, hours) => ({
	id,
	date,
	hours,
	notes: null,
	issue_id: 1,
	issue_title: 'Issue',
	project_id: 1,
	project_name: 'Web',
	customer_id: 1,
	customer_name: 'Acme'
});

const todo = (id, status, position, due_date = null) => ({
	id,
	title: `Todo ${id}`,
	notes: null,
	status,
	position,
	due_date,
	created_at: '2026-01-01 00:00:00',
	issue_id: null,
	issue_title: null,
	project_id: null,
	project_name: null,
	customer_id: null,
	customer_name: null
});

const ids = (rows) => rows.map((r) => r.id);

test('weekDays returns Monday..Sunday for any weekday', () => {
	const week = [
		'2026-11-02',
		'2026-11-03',
		'2026-11-04',
		'2026-11-05',
		'2026-11-06',
		'2026-11-07',
		'2026-11-08'
	];
	assert.deepEqual(weekDays('2026-11-06'), week);
	assert.deepEqual(weekDays('2026-11-02'), week);
	assert.deepEqual(weekDays('2026-11-08'), week);
});

test('weekDays spans month and year boundaries', () => {
	assert.deepEqual(weekDays('2026-10-01'), [
		'2026-09-28',
		'2026-09-29',
		'2026-09-30',
		'2026-10-01',
		'2026-10-02',
		'2026-10-03',
		'2026-10-04'
	]);
	assert.deepEqual(weekDays('2027-01-01'), [
		'2026-12-28',
		'2026-12-29',
		'2026-12-30',
		'2026-12-31',
		'2027-01-01',
		'2027-01-02',
		'2027-01-03'
	]);
});

test('dailyTotals sums hours per day in day order and ignores other dates', () => {
	const days = weekDays('2026-11-06');
	const totals = dailyTotals(
		[
			entry(1, '2026-11-06', 2.5),
			entry(2, '2026-11-02', 8),
			entry(3, '2026-11-06', 1),
			entry(4, '2026-10-30', 4),
			entry(5, '2026-11-08', 0.25)
		],
		days
	);
	assert.deepEqual(totals, [
		{ date: '2026-11-02', hours: 8 },
		{ date: '2026-11-03', hours: 0 },
		{ date: '2026-11-04', hours: 0 },
		{ date: '2026-11-05', hours: 0 },
		{ date: '2026-11-06', hours: 3.5 },
		{ date: '2026-11-07', hours: 0 },
		{ date: '2026-11-08', hours: 0.25 }
	]);
	assert.deepEqual(
		dailyTotals([], days).map((d) => d.hours),
		[0, 0, 0, 0, 0, 0, 0]
	);
});

test('todayEntries keeps only today in input order; sumHours totals', () => {
	const rows = todayEntries(
		[entry(3, '2026-11-06', 1), entry(2, '2026-11-05', 4), entry(1, '2026-11-06', 2.5)],
		'2026-11-06'
	);
	assert.deepEqual(ids(rows), [3, 1]);
	assert.equal(sumHours(rows), 3.5);
	assert.equal(sumHours([]), 0);
});

test('needsAttention: non-done todos due <= today, overdue first, then position', () => {
	const todos = [
		todo(1, 'not_started', 0, '2026-11-06'),
		todo(2, 'in_progress', 1, '2026-11-01'),
		todo(3, 'done', 0, '2026-11-01'),
		todo(4, 'not_started', 2, '2026-11-07'),
		todo(5, 'in_progress', 0, '2026-11-06'),
		todo(6, 'not_started', 3, null),
		todo(7, 'not_started', 0, '2026-11-01')
	];
	assert.deepEqual(ids(needsAttention(todos, '2026-11-06')), [7, 2, 1, 5]);
	assert.deepEqual(needsAttention([], '2026-11-06'), []);
});

test('inProgress: remaining in_progress todos in board order, no duplicates', () => {
	const todos = [
		todo(1, 'in_progress', 2, null),
		todo(2, 'in_progress', 0, '2026-11-06'),
		todo(3, 'in_progress', 1, '2026-11-20'),
		todo(4, 'not_started', 0, null),
		todo(5, 'done', 0, null),
		todo(6, 'in_progress', 3, '2026-11-02')
	];
	const today = '2026-11-06';
	const attention = needsAttention(todos, today);
	const active = inProgress(todos, today);
	assert.deepEqual(ids(active), [3, 1]);
	assert.deepEqual(ids(attention), [6, 2]);
	const all = [...ids(attention), ...ids(active)];
	assert.equal(new Set(all).size, all.length);
});

test('needsAttention/inProgress do not mutate the input', () => {
	const todos = [todo(1, 'in_progress', 1), todo(2, 'in_progress', 0)];
	inProgress(todos, '2026-11-06');
	needsAttention(todos, '2026-11-06');
	assert.deepEqual(ids(todos), [1, 2]);
});

test('progress is a 0..1 fraction, capped, and 0 when target is off', () => {
	assert.equal(progress(4, 8), 0.5);
	assert.equal(progress(10, 8), 1);
	assert.equal(progress(0, 8), 0);
	assert.equal(progress(5, 0), 0);
	assert.equal(progress(5, -1), 0);
	assert.equal(progress(-2, 8), 0);
});

test('parseDayParam accepts real YYYY-MM-DD dates only', () => {
	assert.equal(parseDayParam('2026-11-06'), '2026-11-06');
	assert.equal(parseDayParam('2024-02-29'), '2024-02-29');
	assert.equal(parseDayParam('2026-02-30'), null);
	assert.equal(parseDayParam('2026-13-01'), null);
	assert.equal(parseDayParam('2026-1-6'), null);
	assert.equal(parseDayParam('yesterday'), null);
	assert.equal(parseDayParam(''), null);
	assert.equal(parseDayParam(null), null);
	assert.equal(parseDayParam(undefined), null);
});

test('isWeekend flags Saturday and Sunday', () => {
	assert.deepEqual(
		weekDays('2026-11-06').map(isWeekend),
		[false, false, false, false, false, true, true]
	);
});

test('formatHours drops float noise and trailing zeros', () => {
	assert.equal(formatHours(8), '8');
	assert.equal(formatHours(3.5), '3.5');
	assert.equal(formatHours(0.1 + 0.2), '0.3');
	assert.equal(formatHours(0), '0');
});

test('formatDayLabel parses the date as local (no UTC day shift)', () => {
	const expected = new Date(2026, 10, 6).toLocaleDateString(undefined, {
		weekday: 'short',
		month: 'short',
		day: 'numeric'
	});
	assert.equal(formatDayLabel('2026-11-06'), expected);
});

test('parseDailyTarget accepts 0..24 with decimals and rejects the rest', () => {
	assert.deepEqual(parseDailyTarget('8'), { target: 8 });
	assert.deepEqual(parseDailyTarget(' 7.5 '), { target: 7.5 });
	assert.deepEqual(parseDailyTarget('7,5'), { target: 7.5 });
	assert.deepEqual(parseDailyTarget('0'), { target: 0 });
	assert.deepEqual(parseDailyTarget('24'), { target: 24 });
	assert.deepEqual(parseDailyTarget('24.5'), { error: 'Daily target must be between 0 and 24 hours' });
	assert.deepEqual(parseDailyTarget('-1'), { error: 'Daily target must be between 0 and 24 hours' });
	assert.deepEqual(parseDailyTarget('1e1'), { error: 'Daily target must be a number, e.g. 7.5' });
	assert.deepEqual(parseDailyTarget('abc'), { error: 'Daily target must be a number, e.g. 7.5' });
	assert.deepEqual(parseDailyTarget(''), { error: 'Daily target is required' });
});
