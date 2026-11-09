import assert from 'node:assert/strict';
import test from 'node:test';

const { nextWorkingDay, selectIncoming, handoverKey, keyDate, isBlank } = await import('../src/lib/handovers.ts');

// 2026-11-05 is a Thursday; 2026-11-09 a Monday.
const THU = '2026-11-05';
const FRI = '2026-11-06';
const SAT = '2026-11-07';
const SUN = '2026-11-08';
const MON = '2026-11-09';
const TUE = '2026-11-10';

const h = (date, dismissed = false) => ({
	date,
	body: `from ${date}`,
	dismissed_at: dismissed ? `${date}T18:00:00.000` : null,
	created_at: `${date}T17:00:00.000`,
	updated_at: `${date}T17:00:00.000`
});

test('nextWorkingDay skips the weekend', () => {
	assert.equal(nextWorkingDay(MON), TUE);
	assert.equal(nextWorkingDay(THU), FRI);
	assert.equal(nextWorkingDay(FRI), MON);
	assert.equal(nextWorkingDay(SAT), MON);
	assert.equal(nextWorkingDay(SUN), MON);
});

test('nextWorkingDay crosses month and year ends', () => {
	assert.equal(nextWorkingDay('2026-12-31'), '2027-01-01'); // Thu → Fri
	assert.equal(nextWorkingDay('2027-01-01'), '2027-01-04'); // Fri → Mon
	assert.equal(nextWorkingDay('2028-02-28'), '2028-02-29'); // leap day (Mon → Tue)
});

test('a Thursday handover shows on Friday', () => {
	assert.equal(selectIncoming([h(THU)], THU), null, "today's own handover is never incoming");
	assert.equal(selectIncoming([h(THU)], FRI)?.date, THU);
});

test('a Friday or weekend handover is hidden on the weekend and shows on Monday', () => {
	for (const written of [FRI, SAT, SUN]) {
		for (const viewed of [SAT, SUN]) {
			if (viewed <= written) continue;
			assert.equal(selectIncoming([h(written)], viewed), null, `${written} viewed ${viewed}`);
		}
		assert.equal(selectIncoming([h(written)], MON)?.date, written, `${written} on Monday`);
		assert.equal(selectIncoming([h(written)], TUE)?.date, written, 'stays until dismissed');
	}
});

test('selectIncoming: none', () => {
	assert.equal(selectIncoming([], MON), null);
	assert.equal(selectIncoming([h(MON)], MON), null, 'only today');
	assert.equal(selectIncoming([h(TUE)], MON), null, 'only future');
});

test('selectIncoming: the newest past handover dismissed means none (no fallback)', () => {
	assert.equal(selectIncoming([h(THU), h(FRI, true)], MON), null);
	assert.equal(selectIncoming([h(FRI, true), h(THU)], MON), null, 'input order does not matter');
});

test('selectIncoming: the newest past handover not yet due means none (no fallback)', () => {
	// Thu + Fri written, viewed Saturday: Fri is newest and not due until Monday.
	assert.equal(selectIncoming([h(THU), h(FRI)], SAT), null);
	assert.equal(selectIncoming([h(THU), h(FRI)], MON)?.date, FRI);
});

test('selectIncoming: due shows the newest past one only', () => {
	const list = [h('2026-10-29'), h(THU), h('2026-10-30', true), h(MON)];
	assert.equal(selectIncoming(list, TUE)?.date, MON);
	assert.equal(selectIncoming(list, MON)?.date, THU, "today's own is skipped, the newest older one shows");
	// A week off: the newest one shows long after its next working day.
	assert.equal(selectIncoming([h('2026-10-01'), h(THU)], '2026-11-20')?.date, THU);
});

test('handoverKey is a stable number per date', () => {
	assert.equal(handoverKey(MON), 20261109);
	assert.equal(handoverKey('2027-01-01'), 20270101);
	assert.notEqual(handoverKey(MON), handoverKey(TUE));
	assert.ok(Number.isInteger(handoverKey(MON)) && handoverKey(MON) > 0);
});

test('keyDate turns a key back into its date', () => {
	for (const date of [MON, '2027-01-01', '2028-02-29', '0999-12-31']) assert.equal(keyDate(handoverKey(date)), date);
});

test('isBlank treats whitespace as empty', () => {
	assert.equal(isBlank(''), true);
	assert.equal(isBlank(' \n\t'), true);
	assert.equal(isBlank('- [ ] x'), false);
});
