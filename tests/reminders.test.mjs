import assert from 'node:assert/strict';
import test from 'node:test';

const {
	parseReminder,
	snoozeUntil,
	groupReminders,
	formatRemindAt,
	formatRepeat,
	ordinal,
	toLocalTimestamp,
	parseLocalTimestamp,
	quickTime,
	splitTimestamp,
	formStart,
	isPast,
	activeForTodo,
	todoBells,
	toastKey,
	toastStack
} = await import('../src/lib/reminders.ts');

// Sunday 2026-11-08 14:30:15 local time.
const SUN = new Date(2026, 10, 8, 14, 30, 15);
// Monday 2026-11-09 08:00 local time.
const MON = new Date(2026, 10, 9, 8, 0, 0);
// Saturday 2026-11-14 10:00 local time.
const SAT = new Date(2026, 10, 14, 10, 0, 0);

const once = (text, remindAt) => ({ text, remindAt, repeat: 'none', weekdays: [], monthDay: null });
const parse = (input, now = SUN) => parseReminder(input, now);

test('timestamps round-trip in local time', () => {
	const d = new Date(2026, 0, 5, 7, 3, 9, 42);
	assert.equal(toLocalTimestamp(d), '2026-01-05T07:03:09.042');
	assert.equal(parseLocalTimestamp('2026-01-05T07:03:09.042').getTime(), d.getTime());
	assert.equal(parseLocalTimestamp('2026-01-05 07:03').getTime(), new Date(2026, 0, 5, 7, 3).getTime());
	assert.ok(Number.isNaN(parseLocalTimestamp('nope').getTime()));
});

test('in N min / h / days (English)', () => {
	assert.deepEqual(parse('call Anna in 20 min'), once('call Anna', '2026-11-08T14:50:00.000'));
	assert.deepEqual(parse('call Anna in 20min'), once('call Anna', '2026-11-08T14:50:00.000'));
	assert.deepEqual(parse('stretch in 2 hours'), once('stretch', '2026-11-08T16:30:00.000'));
	assert.deepEqual(parse('stretch in an hour'), once('stretch', '2026-11-08T15:30:00.000'));
	assert.deepEqual(parse('late call in 10 h'), once('late call', '2026-11-09T00:30:00.000'));
	// `days` keeps the current time of day.
	assert.deepEqual(parse('review PR in 3 days'), once('review PR', '2026-11-11T14:30:00.000'));
	// …unless a time is given.
	assert.deepEqual(parse('review PR in 3 days at 10'), once('review PR', '2026-11-11T10:00:00.000'));
});

test('om N min / tim / dagar (Swedish)', () => {
	assert.deepEqual(parse('ring Anna om 20 min'), once('ring Anna', '2026-11-08T14:50:00.000'));
	assert.deepEqual(parse('ring Anna om 2 tim'), once('ring Anna', '2026-11-08T16:30:00.000'));
	assert.deepEqual(parse('ring Anna om en timme'), once('ring Anna', '2026-11-08T15:30:00.000'));
	assert.deepEqual(parse('granska om 3 dagar'), once('granska', '2026-11-11T14:30:00.000'));
	assert.deepEqual(parse('granska om 3 dagar kl 10'), once('granska', '2026-11-11T10:00:00.000'));
	// `om` as an ordinary word is left alone.
	assert.deepEqual(parse('prata om budget imorgon'), once('prata om budget', '2026-11-09T09:00:00.000'));
});

test('today / tomorrow → 09:00 without a time', () => {
	assert.deepEqual(parse('send invoice tomorrow'), once('send invoice', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('skicka faktura imorgon'), once('skicka faktura', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('skicka faktura i morgon'), once('skicka faktura', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('send invoice tomorrow at 4pm'), once('send invoice', '2026-11-09T16:00:00.000'));
	assert.deepEqual(parse('send invoice today 17:00'), once('send invoice', '2026-11-08T17:00:00.000'));
	assert.deepEqual(parse('skicka faktura idag kl 17'), once('skicka faktura', '2026-11-08T17:00:00.000'));
	assert.deepEqual(parse('skicka faktura i dag 17'), once('skicka faktura', '2026-11-08T17:00:00.000'));
	assert.deepEqual(parse('standup today', MON), once('standup', '2026-11-09T09:00:00.000'));
});

test('time without a date → today, or tomorrow once passed', () => {
	assert.deepEqual(parse('call Anna at 16'), once('call Anna', '2026-11-08T16:00:00.000'));
	assert.deepEqual(parse('call Anna at 9'), once('call Anna', '2026-11-08T21:00:00.000'));
	assert.deepEqual(parse('call Anna 14:30'), once('call Anna', '2026-11-09T14:30:00.000'));
	assert.deepEqual(parse('ring Anna kl 9.30'), once('ring Anna', '2026-11-09T09:30:00.000'));
	assert.deepEqual(parse('ring Anna kl. 18.45'), once('ring Anna', '2026-11-08T18:45:00.000'));
	assert.deepEqual(parse('ring Anna klockan 20'), once('ring Anna', '2026-11-08T20:00:00.000'));
});

test('time formats: 9:30, 9.30, 4pm, 4 pm, 12am', () => {
	assert.deepEqual(parse('x 9:30'), once('x', '2026-11-09T09:30:00.000'));
	assert.deepEqual(parse('x 21.05'), once('x', '2026-11-08T21:05:00.000'));
	assert.deepEqual(parse('x 4pm'), once('x', '2026-11-08T16:00:00.000'));
	assert.deepEqual(parse('x 4 pm'), once('x', '2026-11-08T16:00:00.000'));
	assert.deepEqual(parse('x 11:15pm'), once('x', '2026-11-08T23:15:00.000'));
	assert.deepEqual(parse('x tomorrow 12am'), once('x', '2026-11-09T00:00:00.000'));
	assert.deepEqual(parse('x tomorrow 12pm'), once('x', '2026-11-09T12:00:00.000'));
	// Out-of-range values are not times.
	assert.deepEqual(parse('x 25:00'), { error: 'no_time', text: 'x 25:00' });
	assert.deepEqual(parse('x 13pm'), { error: 'no_time', text: 'x 13pm' });
});

test('bare numbers are hours only after a day word or at/kl', () => {
	assert.deepEqual(parse('do 9 pushups tomorrow'), once('do 9 pushups', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('gör 9 armhävningar imorgon'), once('gör 9 armhävningar', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('do 9 pushups tomorrow 7'), once('do 9 pushups', '2026-11-09T19:00:00.000'));
	assert.deepEqual(parse('buy 3 apples'), { error: 'no_time', text: 'buy 3 apples' });
	assert.deepEqual(parse('dentist friday 8'), once('dentist', '2026-11-13T08:00:00.000'));
	assert.deepEqual(parse('dentist at 8 friday'), once('dentist', '2026-11-13T08:00:00.000'));
	assert.deepEqual(parse('meet at office tomorrow'), once('meet at office', '2026-11-09T09:00:00.000'));
});

test('bare hours 1–12: next occurrence without a day, PM for 1–7 with one', () => {
	// No day: the next time the clock shows that hour (SUN is 14:30).
	assert.deepEqual(parse('call Anna at 3'), once('call Anna', '2026-11-08T15:00:00.000'));
	assert.deepEqual(parse('call Anna at 9'), once('call Anna', '2026-11-08T21:00:00.000'));
	assert.deepEqual(parse('call Anna at 9', MON), once('call Anna', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('ring Anna kl 3'), once('ring Anna', '2026-11-08T15:00:00.000'));
	assert.deepEqual(parse('call Anna at 2'), once('call Anna', '2026-11-09T02:00:00.000'));
	assert.deepEqual(parse('late call at 9', new Date(2026, 10, 8, 22, 0)), once('late call', '2026-11-09T09:00:00.000'));
	// With a day word: 1–7 → PM, 8–12 as typed.
	assert.deepEqual(parse('sync mon 3'), once('sync', '2026-11-09T15:00:00.000'));
	assert.deepEqual(parse('sync tomorrow 3'), once('sync', '2026-11-09T15:00:00.000'));
	assert.deepEqual(parse('synk fre kl 4'), once('synk', '2026-11-13T16:00:00.000'));
	assert.deepEqual(parse('sync tomorrow 8'), once('sync', '2026-11-09T08:00:00.000'));
	assert.deepEqual(parse('sync tomorrow at 12'), once('sync', '2026-11-09T12:00:00.000'));
	assert.deepEqual(parse('sync 20/11 kl 5'), once('sync', '2026-11-20T17:00:00.000'));
	// Explicit forms stay literal.
	assert.deepEqual(parse('sync tomorrow 03:00'), once('sync', '2026-11-09T03:00:00.000'));
	assert.deepEqual(parse('sync tomorrow 3:00'), once('sync', '2026-11-09T03:00:00.000'));
	assert.deepEqual(parse('sync tomorrow 3.00'), once('sync', '2026-11-09T03:00:00.000'));
	assert.deepEqual(parse('sync tomorrow 3am'), once('sync', '2026-11-09T03:00:00.000'));
	assert.deepEqual(parse('sync tomorrow kl 03'), once('sync', '2026-11-09T03:00:00.000'));
	assert.deepEqual(parse('sync at 15'), once('sync', '2026-11-08T15:00:00.000'));
	assert.deepEqual(parse('sync tomorrow 15'), once('sync', '2026-11-09T15:00:00.000'));
});

test('an explicit today keeps the next-occurrence rule for bare hours', () => {
	// SUN is 14:30: `today at 9` is 21:00, not the 09:00 already past.
	assert.deepEqual(parse('call Anna today at 9'), once('call Anna', '2026-11-08T21:00:00.000'));
	assert.deepEqual(parse('ring Anna idag kl 9'), once('ring Anna', '2026-11-08T21:00:00.000'));
	assert.deepEqual(parse('ring Anna i dag kl 9'), once('ring Anna', '2026-11-08T21:00:00.000'));
	assert.deepEqual(parse('call Anna today 3'), once('call Anna', '2026-11-08T15:00:00.000'));
	assert.deepEqual(parse('call Anna today at 9', MON), once('call Anna', '2026-11-09T09:00:00.000'));
	// Explicit forms stay literal.
	assert.deepEqual(parse('call Anna today 21:00'), once('call Anna', '2026-11-08T21:00:00.000'));
	assert.deepEqual(parse('call Anna today 9pm'), once('call Anna', '2026-11-08T21:00:00.000'));
});

test('bare 12 is noon; explicit 00:00 / 12am stay midnight', () => {
	// SUN is 14:30: noon has passed → tomorrow.
	assert.deepEqual(parse('lunch at 12'), once('lunch', '2026-11-09T12:00:00.000'));
	assert.deepEqual(parse('lunch kl 12'), once('lunch', '2026-11-09T12:00:00.000'));
	// MON is 08:00: noon is still ahead → today.
	assert.deepEqual(parse('lunch at 12', MON), once('lunch', '2026-11-09T12:00:00.000'));
	assert.deepEqual(parse('lunch kl 12', new Date(2026, 10, 8, 0, 0, 30)), once('lunch', '2026-11-08T12:00:00.000'));
	assert.deepEqual(parse('lunch kl 12', new Date(2026, 10, 8, 23, 30)), once('lunch', '2026-11-09T12:00:00.000'));
	// With a day word: that day's noon.
	assert.deepEqual(parse('lunch fri 12'), once('lunch', '2026-11-13T12:00:00.000'));
	assert.deepEqual(parse('lunch today at 12', MON), once('lunch', '2026-11-09T12:00:00.000'));
	// Explicit midnight.
	assert.deepEqual(parse('x 00:00'), once('x', '2026-11-09T00:00:00.000'));
	assert.deepEqual(parse('x 12am'), once('x', '2026-11-09T00:00:00.000'));
	assert.deepEqual(parse('x kl 00'), once('x', '2026-11-09T00:00:00.000'));
});

test('date-only phrases → 09:00', () => {
	assert.deepEqual(parse('report 2026-11-20'), once('report', '2026-11-20T09:00:00.000'));
	assert.deepEqual(parse('report on 2026-11-20 at 15:00'), once('report', '2026-11-20T15:00:00.000'));
	assert.deepEqual(parse('rapport den 20/11'), once('rapport', '2026-11-20T09:00:00.000'));
	assert.deepEqual(parse('dentist friday'), once('dentist', '2026-11-13T09:00:00.000'));
	assert.deepEqual(parse('tandläkare fredag'), once('tandläkare', '2026-11-13T09:00:00.000'));
});

test('dates are day/month and roll to the next such date', () => {
	// 3/12 is 3 December, not 12 March.
	assert.deepEqual(parse('party 3/12'), once('party', '2026-12-03T09:00:00.000'));
	// 1/11 has passed this year → next year.
	assert.deepEqual(parse('renew 1/11 10:00'), once('renew', '2027-11-01T10:00:00.000'));
	assert.deepEqual(parse('renew 1/11/2028'), once('renew', '2028-11-01T09:00:00.000'));
	// Invalid dates are not dates.
	assert.deepEqual(parse('x 31/11'), { error: 'no_time', text: 'x 31/11' });
	assert.deepEqual(parse('x 2026-02-30'), { error: 'no_time', text: 'x 2026-02-30' });
});

test('weekday names: full always, abbreviations only after on/next/every/på/nästa/varje or before a time', () => {
	assert.deepEqual(parse('sync on fri'), once('sync', '2026-11-13T09:00:00.000'));
	assert.deepEqual(parse('synk på fre'), once('synk', '2026-11-13T09:00:00.000'));
	assert.deepEqual(parse('deploy fri 16:00'), once('deploy', '2026-11-13T16:00:00.000'));
	assert.deepEqual(parse('möte ons kl 10'), once('möte', '2026-11-11T10:00:00.000'));
	assert.deepEqual(parse('möte ons 10'), once('möte', '2026-11-11T10:00:00.000'));
	// Not a weekday: abbreviation without a trigger word or time.
	assert.deepEqual(parse('buy sun cream'), { error: 'no_time', text: 'buy sun cream' });
	assert.deepEqual(parse('buy sun cream tomorrow'), once('buy sun cream', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('remind ons möte kl 10'), once('ons möte', '2026-11-08T22:00:00.000'));
	assert.deepEqual(parse('I sat down tomorrow'), once('I sat down', '2026-11-09T09:00:00.000'));
	// Same weekday as today: today while the time is ahead, else next week.
	assert.deepEqual(parse('walk sunday 18:00'), once('walk', '2026-11-08T18:00:00.000'));
	assert.deepEqual(parse('walk sunday'), once('walk', '2026-11-15T09:00:00.000'));
	assert.deepEqual(parse('promenad söndag'), once('promenad', '2026-11-15T09:00:00.000'));
});

test('next monday / nästa måndag is the coming Monday, never today', () => {
	assert.deepEqual(parse('plan next monday'), once('plan', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('planera nästa måndag'), once('planera', '2026-11-09T09:00:00.000'));
	// On Monday before 09:00 it is still next week.
	assert.deepEqual(parse('plan next monday', MON), once('plan', '2026-11-16T09:00:00.000'));
	assert.deepEqual(parse('planera nästa mån', MON), once('planera', '2026-11-16T09:00:00.000'));
	// Plain `monday` on a Monday before 09:00 is today.
	assert.deepEqual(parse('plan monday', MON), once('plan', '2026-11-09T09:00:00.000'));
});

test('repeat: every day / varje dag / daily', () => {
	const daily = (text, remindAt) => ({ text, remindAt, repeat: 'daily', weekdays: [], monthDay: null });
	assert.deepEqual(parse('water plants every day'), daily('water plants', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('water plants every day at 18'), daily('water plants', '2026-11-08T18:00:00.000'));
	assert.deepEqual(parse('vattna varje dag kl 8'), daily('vattna', '2026-11-09T08:00:00.000'));
	assert.deepEqual(parse('vattna dagligen 20:00'), daily('vattna', '2026-11-08T20:00:00.000'));
});

test('repeat: every weekday / varje vardag; first occurrence on a matching day', () => {
	const wk = (text, remindAt) => ({ text, remindAt, repeat: 'weekdays', weekdays: [], monthDay: null });
	// Typed on Saturday → Monday.
	assert.deepEqual(parse('standup every weekday 9', SAT), wk('standup', '2026-11-16T09:00:00.000'));
	assert.deepEqual(parse('standup varje vardag kl 9', SAT), wk('standup', '2026-11-16T09:00:00.000'));
	// Typed on Monday before the time → today.
	assert.deepEqual(parse('standup every weekday 9:15', MON), wk('standup', '2026-11-09T09:15:00.000'));
});

test('repeat: weekly on chosen days', () => {
	const weekly = (text, remindAt, weekdays) => ({ text, remindAt, repeat: 'weekly', weekdays, monthDay: null });
	assert.deepEqual(parse('gym every mon and thu 7:30'), weekly('gym', '2026-11-09T07:30:00.000', [1, 4]));
	assert.deepEqual(parse('gym varje mån och tor'), weekly('gym', '2026-11-09T09:00:00.000', [1, 4]));
	assert.deepEqual(parse('gym every thursday, monday at 7'), weekly('gym', '2026-11-09T07:00:00.000', [1, 4]));
	assert.deepEqual(parse('sync every fri', MON), weekly('sync', '2026-11-13T09:00:00.000', [5]));
	assert.deepEqual(parse('sync every week on fri'), weekly('sync', '2026-11-13T09:00:00.000', [5]));
	// `every week` alone → today's weekday (Sunday; 09:00 has passed → next Sunday).
	assert.deepEqual(parse('review every week'), weekly('review', '2026-11-15T09:00:00.000', [7]));
});

test('repeat: monthly phrases', () => {
	const monthly = (text, remindAt, monthDay) => ({ text, remindAt, repeat: 'monthly', weekdays: [], monthDay });
	assert.deepEqual(parse('pay rent every month on the 15th'), monthly('pay rent', '2026-11-15T09:00:00.000', 15));
	assert.deepEqual(parse('betala hyran varje månad den 15'), monthly('betala hyran', '2026-11-15T09:00:00.000', 15));
	assert.deepEqual(parse('pay rent every 15th'), monthly('pay rent', '2026-11-15T09:00:00.000', 15));
	assert.deepEqual(parse('betala hyran den 15 varje månad'), monthly('betala hyran', '2026-11-15T09:00:00.000', 15));
	assert.deepEqual(parse('betala hyran den 15:e varje månad kl 8'), monthly('betala hyran', '2026-11-15T08:00:00.000', 15));
	// Already passed this month → next month.
	assert.deepEqual(parse('invoice every 1st'), monthly('invoice', '2026-12-01T09:00:00.000', 1));
	// 31st in a 30-day month → its last day.
	assert.deepEqual(parse('close books every 31st'), monthly('close books', '2026-11-30T09:00:00.000', 31));
	// `every month` → today's day of month (8th; 09:00 passed → December).
	assert.deepEqual(parse('backup every month'), monthly('backup', '2026-12-08T09:00:00.000', 8));
	assert.deepEqual(parse('backup varje månad kl 16'), monthly('backup', '2026-11-08T16:00:00.000', 8));
});

test('repeat: bare hours after a repeat phrase are taken as typed', () => {
	const r = (text, remindAt, repeat, weekdays = [], monthDay = null) => ({ text, remindAt, repeat, weekdays, monthDay });
	// SUN is 14:30; no PM shift, no next-occurrence shift.
	assert.deepEqual(parse('standup every weekday at 7'), r('standup', '2026-11-09T07:00:00.000', 'weekdays'));
	assert.deepEqual(parse('standup varje vardag kl 7'), r('standup', '2026-11-09T07:00:00.000', 'weekdays'));
	assert.deepEqual(parse('gym every mon at 7'), r('gym', '2026-11-09T07:00:00.000', 'weekly', [1]));
	assert.deepEqual(parse('gym varje mån kl 6'), r('gym', '2026-11-09T06:00:00.000', 'weekly', [1]));
	// 03:00 today has passed → tomorrow 03:00 (not 15:00 today).
	assert.deepEqual(parse('vattna varje dag kl 3'), r('vattna', '2026-11-09T03:00:00.000', 'daily'));
	assert.deepEqual(parse('water daily at 5'), r('water', '2026-11-09T05:00:00.000', 'daily'));
	assert.deepEqual(parse('vattna dagligen kl 4'), r('vattna', '2026-11-09T04:00:00.000', 'daily'));
	assert.deepEqual(parse('pay rent every 25th at 9'), r('pay rent', '2026-11-25T09:00:00.000', 'monthly', [], 25));
	assert.deepEqual(parse('pay rent every month on the 15th at 3'), r('pay rent', '2026-11-15T03:00:00.000', 'monthly', [], 15));
	// Bare 12 after a repeat is noon.
	assert.deepEqual(parse('lunch every day at 12'), r('lunch', '2026-11-09T12:00:00.000', 'daily'));
	assert.deepEqual(parse('lunch every day at 12', MON), r('lunch', '2026-11-09T12:00:00.000', 'daily'));
	// Explicit forms stay literal.
	assert.deepEqual(parse('water every day at 3pm'), r('water', '2026-11-08T15:00:00.000', 'daily'));
	// Without a repeat the day-word PM rule still applies.
	assert.deepEqual(parse('gym mon at 7'), once('gym', '2026-11-09T19:00:00.000'));
	assert.deepEqual(parse('gym in 2 days at 3'), once('gym', '2026-11-10T15:00:00.000'));
});

test('text extraction drops remind/me and a leading to/att', () => {
	assert.deepEqual(parse('remind me to call Anna in 20 min'), once('call Anna', '2026-11-08T14:50:00.000'));
	assert.deepEqual(parse('to call Anna tomorrow'), once('call Anna', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('påminn mig att ringa Anna imorgon'), once('ringa Anna', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('att ringa Anna imorgon'), once('ringa Anna', '2026-11-09T09:00:00.000'));
	assert.deepEqual(parse('Call Anna, tomorrow at 10'), once('Call Anna', '2026-11-09T10:00:00.000'));
	// `to` in the middle stays.
	assert.deepEqual(parse('talk to Anna tomorrow'), once('talk to Anna', '2026-11-09T09:00:00.000'));
});

test('unparseable input', () => {
	assert.deepEqual(parse('call Anna'), { error: 'no_time', text: 'call Anna' });
	assert.deepEqual(parse('remind me to call Anna'), { error: 'no_time', text: 'call Anna' });
	assert.deepEqual(parse(''), { error: 'no_time', text: '' });
	assert.deepEqual(parse('tomorrow at 9'), { error: 'no_text', text: '' });
	assert.deepEqual(parse('remind me in 20 min'), { error: 'no_text', text: '' });
});

test('snoozeUntil', () => {
	assert.equal(snoozeUntil('20m', SUN), '2026-11-08T14:50:00.000');
	assert.equal(snoozeUntil('1h', SUN), '2026-11-08T15:30:00.000');
	assert.equal(snoozeUntil('3h', SUN), '2026-11-08T17:30:00.000');
	assert.equal(snoozeUntil('tomorrow', SUN), '2026-11-09T09:00:00.000');
	assert.equal(snoozeUntil('3h', new Date(2026, 11, 31, 22, 0)), '2027-01-01T01:00:00.000');
});

const row = (id, remind_at, state = 'pending', updated_at = '2026-11-01T00:00:00.000') => ({ id, remind_at, state, updated_at });

test('groupReminders buckets and sorts', () => {
	const list = [
		row(1, '2026-11-20T09:00:00.000'),
		row(2, '2026-11-09T09:00:00.000'),
		row(3, '2026-11-08T16:00:00.000'),
		row(4, '2026-11-08T10:00:00.000', 'fired'),
		row(5, '2026-11-07T10:00:00.000', 'fired'),
		row(6, '2026-11-08T15:00:00.000'),
		row(7, '2026-11-05T09:00:00.000', 'done', '2026-11-05T09:01:00.000'),
		row(8, '2026-11-06T09:00:00.000', 'done', '2026-11-07T09:01:00.000'),
		row(9, '2026-11-08T14:00:00.000'), // past but not yet fired
		row(10, '2026-11-12T09:00:00.000')
	];
	const g = groupReminders(list, SUN);
	const ids = (xs) => xs.map((r) => r.id);
	assert.deepEqual(ids(g.overdue), [5, 4]);
	assert.deepEqual(ids(g.today), [9, 6, 3]);
	assert.deepEqual(ids(g.tomorrow), [2]);
	assert.deepEqual(ids(g.later), [10, 1]);
	assert.deepEqual(ids(g.done), [8, 7]);
	assert.deepEqual(groupReminders([], SUN), { overdue: [], today: [], tomorrow: [], later: [], done: [] });
});

test('formatRemindAt', () => {
	assert.equal(formatRemindAt('2026-11-08T14:52:00.000', SUN), '14:52');
	assert.equal(formatRemindAt('2026-11-09T09:00:00.000', SUN), 'Tomorrow 09:00');
	assert.equal(formatRemindAt('2026-11-07T09:00:00.000', SUN), 'Yesterday 09:00');
	assert.equal(formatRemindAt('2026-11-13T16:00:00.000', SUN), 'Fri 16:00');
	assert.equal(formatRemindAt('2026-11-14T16:00:00.000', SUN), 'Sat 16:00');
	assert.equal(formatRemindAt('2026-11-15T09:00:00.000', SUN), '15 Nov 09:00');
	assert.equal(formatRemindAt('2026-11-20T09:00:00.000', SUN), '20 Nov 09:00');
	assert.equal(formatRemindAt('2026-11-02T09:00:00.000', SUN), '2 Nov 09:00');
	assert.equal(formatRemindAt('2027-01-04T09:00:00.000', SUN), '4 Jan 2027 09:00');
	assert.equal(formatRemindAt('garbage', SUN), 'garbage');
});

test('formatRepeat and ordinal', () => {
	assert.equal(formatRepeat({ repeat: 'none', weekdays: [], month_day: null }), '');
	assert.equal(formatRepeat({ repeat: 'daily', weekdays: [], month_day: null }), 'daily');
	assert.equal(formatRepeat({ repeat: 'weekdays', weekdays: [], month_day: null }), 'weekdays');
	assert.equal(formatRepeat({ repeat: 'weekly', weekdays: [4, 1], month_day: null }), 'Mon, Thu');
	assert.equal(formatRepeat({ repeat: 'monthly', weekdays: [], month_day: 15 }), 'monthly on the 15th');
	assert.equal(formatRepeat({ repeat: 'monthly', weekdays: [], monthDay: 1 }), 'monthly on the 1st');
	assert.deepEqual(
		[1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31].map(ordinal),
		['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '23rd', '31st']
	);
});

// ── Form and UI helpers ──────────────────────────────────────────────────────

test('quickTime: 20 min, 1 h, tomorrow 09:00 and the coming Monday 09:00', () => {
	assert.equal(quickTime('20m', SUN), '2026-11-08T14:50:00.000');
	assert.equal(quickTime('1h', SUN), '2026-11-08T15:30:00.000');
	assert.equal(quickTime('tomorrow', SUN), '2026-11-09T09:00:00.000');
	assert.equal(quickTime('monday', SUN), '2026-11-09T09:00:00.000');
	// On a Monday (even before 09:00) it is the next week's Monday.
	assert.equal(quickTime('monday', MON), '2026-11-16T09:00:00.000');
	assert.equal(quickTime('monday', SAT), '2026-11-16T09:00:00.000');
});

test('splitTimestamp gives date and HH:MM for the form inputs', () => {
	assert.deepEqual(splitTimestamp('2026-11-20T09:05:00.000'), { date: '2026-11-20', time: '09:05' });
});

test('isPast compares against now', () => {
	assert.equal(isPast('2026-11-08T14:30:00.000', SUN), true);
	assert.equal(isPast('2026-11-08T14:31:00.000', SUN), false);
	assert.equal(isPast('garbage', SUN), false);
});

const uiRow = (id, remind_at, state, todo_id = null) => ({ id, remind_at, state, todo_id, updated_at: remind_at });

test('activeForTodo and todoBells: fired first, then the next pending; done ignored', () => {
	const list = [
		uiRow(1, '2026-11-10T09:00:00.000', 'pending', 7),
		uiRow(2, '2026-11-09T09:00:00.000', 'pending', 7),
		uiRow(3, '2026-11-08T08:00:00.000', 'done', 7),
		uiRow(4, '2026-11-12T09:00:00.000', 'fired', 8),
		uiRow(5, '2026-11-08T09:00:00.000', 'pending', 8),
		uiRow(6, '2026-11-08T09:00:00.000', 'pending', null)
	];
	assert.deepEqual(activeForTodo(list, 7).map((r) => r.id), [2, 1]);
	assert.deepEqual(activeForTodo(list, 8).map((r) => r.id), [4, 5]);
	const bells = todoBells(list);
	assert.equal(bells.get(7).id, 2);
	assert.equal(bells.get(8).id, 4);
	assert.equal(bells.size, 2);
});

test('toastStack shows up to three fired, not dismissed, and counts the rest', () => {
	const list = [
		uiRow(1, '2026-11-08T09:00:00.000', 'fired'),
		uiRow(2, '2026-11-08T08:00:00.000', 'fired'),
		uiRow(3, '2026-11-08T10:00:00.000', 'fired'),
		uiRow(4, '2026-11-08T11:00:00.000', 'fired'),
		uiRow(5, '2026-11-08T12:00:00.000', 'fired'),
		uiRow(6, '2026-11-08T12:00:00.000', 'pending')
	];
	let stack = toastStack(list, new Set());
	assert.deepEqual(stack.shown.map((r) => r.id), [2, 1, 3]);
	assert.equal(stack.more, 2);
	stack = toastStack(list, new Set([toastKey(list[1]), toastKey(list[0])]));
	assert.deepEqual(stack.shown.map((r) => r.id), [3, 4, 5]);
	assert.equal(stack.more, 0);
	// A re-fired occurrence (new remind_at) is a new toast.
	stack = toastStack([uiRow(1, '2026-11-09T09:00:00.000', 'fired')], new Set([toastKey(list[0])]));
	assert.equal(stack.shown.length, 1);
});

test('formStart: one-off rows keep remind_at; recurring rows use time_of_day at the next occurrence', () => {
	const row = (over) => ({ remind_at: '2026-11-08T16:45:00.000', time_of_day: '09:00', repeat: 'none', weekdays: [], month_day: null, ...over });
	assert.deepEqual(formStart(row({}), SUN), { date: '2026-11-08', time: '16:45' });
	// Daily 09:00 snoozed to 16:45 today (SUN is 14:30): the anchor, on the next occurrence.
	assert.deepEqual(formStart(row({ repeat: 'daily' }), SUN), { date: '2026-11-09', time: '09:00' });
	assert.deepEqual(formStart(row({ repeat: 'daily', time_of_day: '18:00' }), SUN), { date: '2026-11-08', time: '18:00' });
	// Weekdays: Saturday → Monday.
	assert.deepEqual(formStart(row({ repeat: 'weekdays', remind_at: '2026-11-14T11:00:00.000' }), SAT), { date: '2026-11-16', time: '09:00' });
	// Weekly Mon+Thu at 09:00, fired Mon 09:00 and snoozed (MON is 08:00 → today still ahead).
	assert.deepEqual(formStart(row({ repeat: 'weekly', weekdays: [1, 4] }), MON), { date: '2026-11-09', time: '09:00' });
	assert.deepEqual(formStart(row({ repeat: 'weekly', weekdays: [1, 4] }), SUN), { date: '2026-11-09', time: '09:00' });
	// Monthly on the 31st clamps to the month's last day.
	assert.deepEqual(formStart(row({ repeat: 'monthly', month_day: 31 }), SUN), { date: '2026-11-30', time: '09:00' });
	// A malformed anchor falls back to remind_at.
	assert.deepEqual(formStart(row({ repeat: 'daily', time_of_day: '' }), SUN), { date: '2026-11-08', time: '16:45' });
});
