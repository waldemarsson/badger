import assert from 'node:assert/strict';
import test from 'node:test';

const { formatNumber, tripDateLabel, tripAmountLine, tripTotals, monthReportText } = await import(
	'../src/lib/travel.ts'
);

const trip = (id, date, from, to, km, amount) => ({ id, date, from_place: from, to_place: to, km, amount });

const ROADMAP_EXAMPLE = `October

OP
Frånvaro: 50 h

Roxtec
MyRoxtec: 40 h
PublicWeb: 20 h

Total: 110 h

Travel:

5/10 Malmö - Karlskrona
500 km - 1300 kr

10/10 Malmö - Karlskrona
500 km - 1300 kr`;

test('monthReportText reproduces the roadmap example exactly', () => {
	const customers = [
		{ name: 'OP', projects: [{ name: 'Frånvaro', hours: 50 }] },
		{ name: 'Roxtec', projects: [{ name: 'MyRoxtec', hours: 40 }, { name: 'PublicWeb', hours: 20 }] },
	];
	const trips = [
		trip(1, '2026-10-05', 'Malmö', 'Karlskrona', 500, 1300),
		trip(2, '2026-10-10', 'Malmö', 'Karlskrona', 500, 1300),
	];
	assert.equal(monthReportText(10, customers, trips), ROADMAP_EXAMPLE);
});

test('monthReportText drops missing km/amount parts', () => {
	const text = monthReportText(1, [], [
		trip(1, '2026-01-02', 'A', 'B', 12.5, null),
		trip(2, '2026-01-03', 'A', 'B', null, 99.5),
		trip(3, '2026-01-04', 'A', 'B', null, null),
	]);
	assert.equal(text, 'January\n\nTravel:\n\n2/1 A - B\n12.5 km\n\n3/1 A - B\n99.5 kr\n\n4/1 A - B');
});

test('monthReportText omits the travel block when there are no trips', () => {
	const text = monthReportText(12, [{ name: 'C', projects: [{ name: 'P', hours: 7.25 }] }], []);
	assert.equal(text, 'December\n\nC\nP: 7.25 h\n\nTotal: 7.25 h');
});

test('monthReportText returns null for an empty month', () => {
	assert.equal(monthReportText(3, [], []), null);
});

test('formatNumber drops trailing zeros and float noise', () => {
	assert.equal(formatNumber(1300), '1300');
	assert.equal(formatNumber(99.5), '99.5');
	assert.equal(formatNumber(0.1 + 0.2), '0.3');
	assert.equal(formatNumber(12345.6), '12345.6');
});

test('tripDateLabel is day/month without padding', () => {
	assert.equal(tripDateLabel('2026-10-05'), '5/10');
	assert.equal(tripDateLabel('2026-01-31'), '31/1');
});

test('tripAmountLine and tripTotals skip missing values', () => {
	assert.equal(tripAmountLine({ km: 500, amount: 0 }), '500 km - 0 kr');
	assert.equal(tripAmountLine({ km: null, amount: null }), '');
	assert.deepEqual(tripTotals([{ km: 500, amount: null }, { km: null, amount: 1300 }, { km: 2.5, amount: 100 }]), {
		km: 502.5,
		amount: 1400,
	});
});
