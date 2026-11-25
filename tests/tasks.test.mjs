import assert from 'node:assert/strict';
import test from 'node:test';

const {
	ymd,
	addDays,
	weekStart,
	formatDay,
	formatWhen,
	chipWhen,
	completedLabel,
	dueAnchor,
	isPastDue,
	needsAttention,
	overdueCount,
	taskGroup,
	groupTasks,
	completedTasks,
	todayTasks,
	dueTodayCount,
	offsetLabel,
	reminderLabel,
	repeatLabel,
	taskChips,
	relativeFireAt,
	reminderStatus,
	reminderPresets,
	customReminderStart,
	taskInput,
	dueEdit,
	matchIssue,
	planQuickAdd,
	planInput,
	planChips,
	planError,
	completionMessage,
	undoEntry,
	undoActive,
	nextFocusId,
	toastStack,
	TASKS_CHANGED_EVENT,
	REMINDERS_CHANGED_EVENT,
	TASKS_OPEN_EVENT,
	UNDO_MS
} = await import('../src/lib/tasks.ts');
const { parseReminder } = await import('../src/lib/time-phrases.ts');

// Wednesday 2026-11-25 11:20 local time (the mockup's "now").
const NOW = new Date(2026, 10, 25, 11, 20, 0);

let nextId = 1;
const task = (over = {}) => ({
	id: nextId++,
	title: 'Task',
	notes: null,
	done: false,
	completed_at: null,
	due_date: null,
	due_time: null,
	repeat: 'none',
	weekdays: [],
	month_day: null,
	issue_id: null,
	issue_title: null,
	project_id: null,
	project_name: null,
	customer_id: null,
	customer_name: null,
	note_id: null,
	note_title: null,
	mac_only: false,
	created_at: '2026-11-20T10:00:00.000',
	updated_at: '2026-11-20T10:00:00.000',
	reminders: [],
	...over
});
const rem = (over = {}) => ({
	id: nextId++,
	task_id: 0,
	offset_minutes: null,
	at: null,
	state: 'pending',
	snoozed_until: null,
	fire_at: null,
	created_at: '2026-11-20T10:00:00.000',
	...over
});

test('event names match Rust', () => {
	assert.equal(TASKS_CHANGED_EVENT, 'tasks-changed');
	assert.equal(REMINDERS_CHANGED_EVENT, 'reminders-changed');
	assert.equal(TASKS_OPEN_EVENT, 'tasks-open');
});

test('date helpers', () => {
	assert.equal(ymd(NOW), '2026-11-25');
	assert.equal(addDays('2026-11-30', 1), '2026-12-01');
	assert.equal(addDays('2026-03-01', -1), '2026-02-28');
	assert.equal(weekStart('2026-11-25'), '2026-11-23');
	assert.equal(weekStart('2026-11-29'), '2026-11-23', 'Sunday belongs to the week before');
	assert.equal(weekStart('2026-11-23'), '2026-11-23');
	assert.equal(formatDay('2026-11-27', NOW), 'Fri 27 Nov');
	assert.equal(formatDay('2027-01-04', NOW), 'Mon 4 Jan 2027');
	assert.equal(formatWhen('2026-11-25T09:00:00.000', NOW), 'Today 09:00');
	assert.equal(formatWhen('2026-11-26T09:00', NOW), 'Tomorrow 09:00');
	assert.equal(formatWhen('2026-11-24T18:00', NOW), 'Yesterday 18:00');
	assert.equal(formatWhen('2026-11-27T10:00', NOW), 'Fri 27 Nov 10:00');
	assert.equal(chipWhen('2026-11-25T15:30:00.000', NOW), '15:30');
	assert.equal(chipWhen('2026-11-26T09:00', NOW), 'Thu 26 Nov 09:00');
	assert.equal(completedLabel({ completed_at: '2026-11-25T08:00:00.000' }, NOW), 'Completed today');
	assert.equal(completedLabel({ completed_at: '2026-11-24T08:00:00.000' }, NOW), 'Completed yesterday');
	assert.equal(completedLabel({ completed_at: '2026-11-23T08:00:00.000' }, NOW), 'Completed Mon 23 Nov');
});

test('overdue (D10): past due date, today with a time that has come, or a sent reminder', () => {
	assert.equal(dueAnchor(task({ due_date: '2026-11-25' })), '2026-11-25T09:00');
	assert.equal(dueAnchor(task({ due_date: '2026-11-25', due_time: '14:00' })), '2026-11-25T14:00');
	assert.equal(dueAnchor(task()), null);

	const yesterday = task({ due_date: '2026-11-24' });
	const todayUntimed = task({ due_date: '2026-11-25' });
	const todayPast = task({ due_date: '2026-11-25', due_time: '11:20' });
	const todayLater = task({ due_date: '2026-11-25', due_time: '14:00' });
	const sent = task({ reminders: [rem({ state: 'sent', at: '2026-11-25T08:00:00.000', fire_at: '2026-11-25T08:00:00.000' })] });
	const dismissed = task({ reminders: [rem({ state: 'dismissed', fire_at: '2026-11-25T08:00:00.000' })] });
	const doneLate = task({ done: true, due_date: '2026-11-20', reminders: [rem({ state: 'sent' })] });

	assert.equal(isPastDue(yesterday, NOW), true);
	assert.equal(isPastDue(todayUntimed, NOW), false, 'a date alone is due all day');
	assert.equal(isPastDue(todayPast, NOW), true, 'time ≤ now');
	assert.equal(isPastDue(todayLater, NOW), false);
	assert.equal(needsAttention(sent, NOW), true);
	assert.equal(needsAttention(dismissed, NOW), false, 'dismissed stops counting');
	assert.equal(needsAttention(doneLate, NOW), false);
	assert.equal(overdueCount([yesterday, todayUntimed, todayPast, todayLater, sent, dismissed, doneLate], NOW), 3);
	assert.equal(overdueCount([], NOW), 0);
});

test('groups: overdue, today, upcoming, no date; ordered by due date, time (untimed last), created', () => {
	const a = task({ title: 'a', due_date: '2026-11-25' });
	const b = task({ title: 'b', due_date: '2026-11-25', due_time: '14:00' });
	const c = task({ title: 'c', due_date: '2026-11-25', due_time: '09:00' });
	const d = task({ title: 'd', due_date: '2026-11-23' });
	const e = task({ title: 'e', due_date: '2026-11-27', due_time: '10:00' });
	const f = task({ title: 'f', due_date: '2026-11-27' });
	const g = task({ title: 'g', created_at: '2026-11-21T00:00:00.000' });
	const h = task({ title: 'h', created_at: '2026-11-19T00:00:00.000' });
	const done = task({ title: 'done', done: true, completed_at: '2026-11-25T08:00:00.000' });
	const groups = groupTasks([a, b, c, d, e, f, g, h, done], NOW);
	assert.deepEqual(
		groups.map((x) => [x.key, x.label, x.items.map((t) => t.title).join('')]),
		[
			['overdue', 'Overdue', 'd'],
			['today', 'Today', 'cba'],
			['upcoming', 'Upcoming', 'ef'],
			['none', 'No date', 'hg']
		]
	);
	assert.deepEqual(groupTasks([done], NOW), [], 'empty groups are left out');
	assert.equal(taskGroup(done, NOW), 'done');
	assert.equal(dueTodayCount([a, b, c, d, done], NOW), 3);
});

test('completed: newest first, split at this week’s Monday', () => {
	const mon = task({ title: 'mon', done: true, completed_at: '2026-11-23T08:00:00.000' });
	const wed = task({ title: 'wed', done: true, completed_at: '2026-11-25T08:00:00.000' });
	const tue = task({ title: 'tue', done: true, completed_at: '2026-11-24T08:00:00.000' });
	const sun = task({ title: 'sun', done: true, completed_at: '2026-11-22T20:00:00.000' });
	const open = task({ title: 'open' });
	const { thisWeek, older } = completedTasks([mon, wed, open, tue, sun], NOW);
	assert.deepEqual(thisWeek.map((t) => t.title), ['wed', 'tue', 'mon']);
	assert.deepEqual(older.map((t) => t.title), ['sun']);
});

test('Today card: overdue and due today, plus open tasks with a sent reminder', () => {
	const over = task({ title: 'over', due_date: '2026-11-24' });
	const today = task({ title: 'today', due_date: '2026-11-25' });
	const later = task({ title: 'later', due_date: '2026-11-26' });
	const pinged = task({ title: 'pinged', reminders: [rem({ state: 'sent', fire_at: '2026-11-25T08:00:00.000' })] });
	const done = task({ title: 'done', done: true, due_date: '2026-11-25' });
	assert.deepEqual(todayTasks([later, today, pinged, over, done], NOW).map((t) => t.title), ['over', 'today', 'pinged']);
});

test('labels: offsets, custom, repeat', () => {
	assert.deepEqual([0, 15, 60, 1440, 10080, 90, 120, 2880].map(offsetLabel), [
		'At due time',
		'15 min before',
		'1 hour before',
		'1 day before',
		'1 week before',
		'90 min before',
		'2 hours before',
		'2 days before'
	]);
	assert.equal(reminderLabel({ offset_minutes: null, at: '2026-11-25T15:30:00.000' }), 'Custom');
	assert.equal(reminderLabel({ offset_minutes: 60, at: null }), '1 hour before');
	const r = (repeat, weekdays = [], month_day = null) => repeatLabel({ repeat, weekdays, month_day });
	assert.equal(r('none'), '');
	assert.equal(r('daily'), 'Daily');
	assert.equal(r('weekdays'), 'Weekdays');
	assert.equal(r('weekly', [4, 1]), 'Weekly · Mon, Thu');
	assert.equal(r('weekly'), 'Weekly');
	assert.equal(r('monthly', [], 21), 'Monthly on the 21st');
	assert.equal(r('monthly'), 'Monthly');
});

test('chips: due tone per group, a reminder at the due folds into the due chip', () => {
	const kinds = (chips) => chips.map((c) => `${c.icon}:${c.tone}:${c.label}`);
	// Overdue, untimed, linked issue.
	assert.deepEqual(
		kinds(taskChips(task({ due_date: '2026-11-23', issue_title: 'Invoicing', project_name: 'Northwind', customer_name: 'NW' }), NOW)),
		['cal:red:Mon 23 Nov', 'link:violet:Northwind · Invoicing']
	);
	// Today 09:00 with "At due time", already past: a red ringing due chip, nothing else.
	const atDue = rem({ offset_minutes: 0, state: 'sent', fire_at: '2026-11-25T09:00:00.000' });
	assert.deepEqual(kinds(taskChips(task({ due_date: '2026-11-25', due_time: '09:00', reminders: [atDue] }), NOW)), ['ring:red:09:00']);
	// Today 14:00 with 1 h before: amber due, sky next reminder.
	const hourBefore = rem({ offset_minutes: 60, fire_at: '2026-11-25T13:00:00.000' });
	assert.deepEqual(kinds(taskChips(task({ due_date: '2026-11-25', due_time: '14:00', reminders: [hourBefore] }), NOW)), [
		'cal:amber:14:00',
		'bell:sky:13:00'
	]);
	// Today untimed, a custom reminder later today.
	const custom = rem({ at: '2026-11-25T15:30:00.000', fire_at: '2026-11-25T15:30:00.000' });
	assert.deepEqual(kinds(taskChips(task({ due_date: '2026-11-25', reminders: [custom] }), NOW)), ['cal:amber:Today', 'bell:sky:15:30']);
	// Upcoming with an at-due bell and two more pending: +1.
	const two = [
		rem({ offset_minutes: 0, fire_at: '2026-11-27T10:00:00.000' }),
		rem({ offset_minutes: 1440, fire_at: '2026-11-26T10:00:00.000' }),
		rem({ offset_minutes: 10080, fire_at: '2026-11-20T10:00:00.000', state: 'dismissed' }),
		rem({ offset_minutes: 60, fire_at: '2026-11-27T09:00:00.000' })
	];
	assert.deepEqual(kinds(taskChips(task({ due_date: '2026-11-27', due_time: '10:00', reminders: two }), NOW)), [
		'bell:slate:Fri 27 Nov · 10:00',
		'bell:sky:Thu 26 Nov 10:00 +1'
	]);
	// A sent reminder (not at due) wins over pending ones.
	const mixed = [rem({ state: 'sent', fire_at: '2026-11-25T08:00:00.000' }), rem({ fire_at: '2026-11-26T08:00:00.000' })];
	assert.deepEqual(kinds(taskChips(task({ reminders: mixed }), NOW)), ['ring:red:08:00']);
	// Relative without a due: no chip. Repeat, note, Mac only (only while sync is on).
	const dormant = rem({ offset_minutes: 15, fire_at: null });
	const t = task({ reminders: [dormant], note_title: 'Architecture review', mac_only: true, repeat: 'daily', due_date: '2026-11-25', due_time: '18:00' });
	assert.deepEqual(kinds(taskChips(t, NOW)), ['cal:amber:18:00', 'repeat:slate:Daily', 'note:slate:Architecture review']);
	assert.deepEqual(kinds(taskChips(t, NOW, true)).at(-1), 'mac:slate:Mac only');
	// Done: slate due, no reminder chip.
	assert.deepEqual(kinds(taskChips(task({ done: true, due_date: '2026-11-20', reminders: mixed }), NOW)), ['cal:slate:Fri 20 Nov']);
});

test('reminder math: relative to the due, untimed at 09:00, across months and DST', () => {
	assert.equal(relativeFireAt({ due_date: '2026-11-27', due_time: '10:00' }, 1440), '2026-11-26T10:00');
	assert.equal(relativeFireAt({ due_date: '2026-11-27', due_time: null }, 0), '2026-11-27T09:00');
	assert.equal(relativeFireAt({ due_date: '2026-12-01', due_time: '00:10' }, 15), '2026-11-30T23:55');
	assert.equal(relativeFireAt({ due_date: '2026-12-01', due_time: null }, 10080), '2026-11-24T09:00');
	assert.equal(relativeFireAt({ due_date: null, due_time: null }, 0), null);
	// Wall-clock math: a day before 09:00 is 09:00 the day before, also over a DST change.
	assert.equal(relativeFireAt({ due_date: '2026-03-30', due_time: '09:00' }, 1440), '2026-03-29T09:00');
});

test('reminder status: sent, done, snoozed, needs a due date', () => {
	const at = '2026-11-25T09:00:00.000';
	assert.deepEqual(reminderStatus(rem({ fire_at: at }), false, NOW), { when: 'Today 09:00', tone: 'pending' });
	assert.deepEqual(reminderStatus(rem({ fire_at: at, state: 'sent' }), false, NOW), { when: 'Today 09:00 · sent', tone: 'sent' });
	assert.deepEqual(reminderStatus(rem({ fire_at: at, state: 'sent' }), true, NOW), { when: 'Today 09:00 · done', tone: 'dismissed' });
	assert.deepEqual(reminderStatus(rem({ fire_at: at, state: 'dismissed' }), false, NOW), { when: 'Today 09:00 · done', tone: 'dismissed' });
	assert.deepEqual(reminderStatus(rem({ fire_at: '2026-11-25T12:00:00.000', snoozed_until: '2026-11-25T12:00:00.000' }), false, NOW), {
		when: 'Today 12:00 · snoozed',
		tone: 'pending'
	});
	assert.deepEqual(reminderStatus(rem({ offset_minutes: 15 }), false, NOW), { when: 'Needs a due date', tone: 'dormant' });
});

test('add reminder presets: hints, taken, no due, at most 10', () => {
	const t = task({ due_date: '2026-11-27', due_time: '10:00', reminders: [rem({ offset_minutes: 60 })] });
	assert.deepEqual(reminderPresets(t, NOW), [
		{ offset: 0, label: 'At due time', hint: 'Fri 27 Nov 10:00', disabled: false },
		{ offset: 15, label: '15 min before', hint: 'Fri 27 Nov 09:45', disabled: false },
		{ offset: 60, label: '1 hour before', hint: 'Added', disabled: true },
		{ offset: 1440, label: '1 day before', hint: 'Tomorrow 10:00', disabled: false },
		{ offset: 10080, label: '1 week before', hint: 'Fri 20 Nov 10:00', disabled: false }
	]);
	assert.ok(reminderPresets(task(), NOW).every((p) => p.disabled && p.hint === 'Needs a due date'));
	const full = task({ due_date: '2026-11-27', reminders: Array.from({ length: 10 }, (_, i) => rem({ at: `2026-11-26T1${i}:00` })) });
	assert.ok(reminderPresets(full, NOW).every((p) => p.disabled));
});

test('custom reminder starts at the due when ahead, else the next whole hour', () => {
	assert.equal(customReminderStart({ due_date: '2026-11-27', due_time: '10:00' }, NOW), '2026-11-27T10:00');
	assert.equal(customReminderStart({ due_date: '2026-11-27', due_time: null }, NOW), '2026-11-27T09:00');
	assert.equal(customReminderStart({ due_date: '2026-11-24', due_time: null }, NOW), '2026-11-25T12:00');
	assert.equal(customReminderStart({ due_date: null, due_time: null }, new Date(2026, 10, 25, 23, 30)), '2026-11-26T00:00');
});

test('update input keeps every field; due edits follow D4/D6', () => {
	const t = task({ id: 5, title: 'X', notes: 'n', due_date: '2026-11-27', due_time: '10:00', repeat: 'weekly', weekdays: [5], issue_id: 3, note_id: 4, mac_only: true });
	assert.deepEqual(taskInput(t, { title: 'Y' }), {
		title: 'Y',
		notes: 'n',
		dueDate: '2026-11-27',
		dueTime: '10:00',
		repeat: 'weekly',
		weekdays: [5],
		monthDay: null,
		issueId: 3,
		noteId: 4,
		macOnly: true
	});
	assert.deepEqual(dueEdit('2026-11-28', null, NOW), { dueDate: '2026-11-28', dueTime: null });
	assert.deepEqual(dueEdit(null, '14:00', NOW), { dueDate: '2026-11-25', dueTime: '14:00' }, 'a time without a date sets today');
	assert.deepEqual(dueEdit(null, null, NOW), { dueDate: null, dueTime: null, repeat: 'none', weekdays: [], monthDay: null });
	assert.deepEqual(dueEdit('', '', NOW), { dueDate: null, dueTime: null, repeat: 'none', weekdays: [], monthDay: null });
});

const ISSUES = [
	{ id: 1, title: 'Invoicing', context: 'Northwind / Billing' },
	{ id: 2, title: 'Apple sync', context: 'Badger / Desktop' },
	{ id: 3, title: 'Portal v2', context: 'Contoso / Web' }
];

test('matchIssue: by id, then title, then customer/project', () => {
	assert.equal(matchIssue('issue/2', ISSUES)?.id, 2);
	assert.equal(matchIssue('issue/9', ISSUES), null);
	assert.equal(matchIssue('portal', ISSUES)?.id, 3);
	assert.equal(matchIssue('Contoso', ISSUES)?.id, 3);
	assert.equal(matchIssue('apple-sync', ISSUES)?.id, 2);
	assert.equal(matchIssue('nope', ISSUES), null);
});

test('quick add: parsed date → due date, typed time → due time + At due time, nothing → no due', () => {
	const plan = (s) => planQuickAdd(s, NOW, ISSUES, parseReminder);
	const p = plan('Send offer to Contoso tomorrow at 9 #Contoso');
	assert.equal(p.title, 'Send offer to Contoso');
	assert.equal(p.dueDate, '2026-11-26');
	assert.equal(p.dueTime, '09:00');
	assert.deepEqual(p.reminders, [{ offsetMinutes: 0 }]);
	assert.equal(p.issue?.id, 3);
	assert.equal(p.unmatchedTag, null);

	const dateOnly = plan('Prepare sprint demo friday');
	assert.deepEqual([dateOnly.title, dateOnly.dueDate, dateOnly.dueTime, dateOnly.reminders], ['Prepare sprint demo', '2026-11-27', null, []]);

	const none = plan('Try the new keyboard layout');
	assert.deepEqual([none.title, none.dueDate, none.dueTime, none.reminders, none.repeat], ['Try the new keyboard layout', null, null, [], 'none']);

	// A time without a date: today while ahead, else tomorrow.
	assert.deepEqual([plan('Call Anna 14:00').dueDate, plan('Call Anna 14:00').dueTime], ['2026-11-25', '14:00']);
	assert.equal(plan('Call Anna 10:00').dueDate, '2026-11-26');

	const rep = plan('Water the plants every day at 18');
	assert.deepEqual([rep.title, rep.repeat, rep.dueDate, rep.dueTime], ['Water the plants', 'daily', '2026-11-25', '18:00']);

	// ⌘K aliases: a leading remind / påminn is dropped.
	assert.equal(plan('remind me to call Anna').title, 'call Anna');
	assert.equal(plan('påminn mig att ringa Anna imorgon').dueDate, '2026-11-26');

	const miss = plan('Call #Fabrikam');
	assert.deepEqual([miss.title, miss.issue, miss.unmatchedTag], ['Call', null, 'Fabrikam']);
	assert.deepEqual(planChips(miss, NOW), [{ label: 'No issue matches #Fabrikam', tone: 'slate', icon: 'link' }]);

	assert.deepEqual(planChips(p, NOW).map((c) => c.label), ['Thu 26 Nov · 09:00', 'Reminder at due time', 'Portal v2']);
	assert.deepEqual(planInput(p), {
		input: { title: 'Send offer to Contoso', dueDate: '2026-11-26', dueTime: '09:00', repeat: 'none', weekdays: [], monthDay: null, issueId: 3 },
		reminders: [{ offsetMinutes: 0 }]
	});
});

test('quick add: empty and too-long titles are caught inline', () => {
	const plan = (s) => planQuickAdd(s, NOW, ISSUES, parseReminder);
	assert.equal(plan('tomorrow at 9').title, '');
	assert.equal(planError(plan('tomorrow at 9')), 'Type what the task is, not only when');
	assert.equal(planError(plan('   ')), 'Type what the task is, not only when');
	assert.equal(planError(plan('x'.repeat(501))), 'Title is too long (max 500 characters)');
	assert.equal(planError(plan('Call Anna')), '');
});

test('undo (D21): entry passes the snapshot back unchanged and expires after the window', () => {
	const snapshot = { task_id: 7, completed_updated_at: '2026-11-25T11:20:00.000', done: false, reminders: [{ id: 1, state: 'pending' }] };
	const done = task({ id: 7, title: 'Dentist', done: true, completed_at: '2026-11-25T11:20:00.000' });
	const u = undoEntry({ task: done, undo: snapshot }, { title: 'Dentist' }, NOW);
	assert.equal(u.undo, snapshot, 'the exact object goes back to undo_complete_task');
	assert.equal(u.taskId, 7);
	assert.equal(u.message, 'Completed “Dentist”');
	assert.equal(u.expires, NOW.getTime() + UNDO_MS);
	assert.equal(undoActive(u, NOW.getTime() + UNDO_MS - 1), true);
	assert.equal(undoActive(u, NOW.getTime() + UNDO_MS), false);
	assert.equal(undoActive(null, NOW.getTime()), false);
	assert.equal(undoEntry({ task: done, undo: null }, done, NOW), null, 'already done: nothing to undo');
	// A repeating task advances and stays open.
	const next = task({ title: 'Water', due_date: '2026-11-26', due_time: '18:00', repeat: 'daily' });
	assert.equal(completionMessage({ title: 'Water' }, next, NOW), '“Water” is next due Thu 26 Nov 18:00');
});

test('focus after completion: the next row, else the previous', () => {
	assert.equal(nextFocusId([1, 2, 3], 2), 3);
	assert.equal(nextFocusId([1, 2, 3], 3), 2);
	assert.equal(nextFocusId([1], 1), null);
	assert.equal(nextFocusId([1, 2], 9), null);
});

test('toasts: sent reminders on open tasks, oldest first, three then +N', () => {
	const mk = (fire, state = 'sent', done = false) => task({ done, reminders: [rem({ state, fire_at: fire })] });
	const list = [
		mk('2026-11-25T09:00:00.000'),
		mk('2026-11-25T08:00:00.000'),
		mk('2026-11-25T10:00:00.000'),
		mk('2026-11-25T11:00:00.000'),
		mk('2026-11-25T07:00:00.000', 'dismissed'),
		mk('2026-11-25T06:00:00.000', 'pending'),
		mk('2026-11-25T05:00:00.000', 'sent', true)
	];
	const s = toastStack(list);
	assert.deepEqual(s.shown.map((x) => x.reminder.fire_at.slice(11, 16)), ['08:00', '09:00', '10:00']);
	assert.equal(s.more, 1);
	assert.deepEqual(toastStack([]), { shown: [], more: 0 });
});
