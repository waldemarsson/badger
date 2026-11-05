import assert from 'node:assert/strict';
import test from 'node:test';

const { loadInitialTimesheetData } = await import(
	'../.tmp-tests/src/lib/timesheet-startup.js'
);

test('loadInitialTimesheetData loads initial entries once and resolves defaults', async () => {
	const calls = [];

	const result = await loadInitialTimesheetData({
		listCustomers: async () => [{ id: 1, name: 'Acme', use_as_default: true }],
		listProjects: async (customerId) => {
			calls.push(['listProjects', customerId]);
			return [{ id: 10, customer_id: customerId, name: 'Web', use_as_default: true }];
		},
		listIssues: async (projectId) => {
			calls.push(['listIssues', projectId]);
			return [{ id: 100, project_id: projectId, title: 'Issue' }];
		},
		listTimeEntries: async (startDate, endDate) => {
			calls.push(['listTimeEntries', startDate, endDate]);
			return [
				{
					id: 200,
					date: startDate,
					hours: 1,
					notes: null,
					issue_id: 100,
					issue_title: 'Issue',
					project_id: 10,
					project_name: 'Web',
					customer_id: 1,
					customer_name: 'Acme'
				}
			];
		},
		getDateRange: () => ({ start: '2026-05-01', end: '2026-05-31' })
	});

	assert.equal(result.newCustomerId, '1');
	assert.equal(result.newProjectId, '10');
	assert.equal(result.entries.length, 1);
	assert.deepEqual(
		calls.filter(([name]) => name === 'listTimeEntries'),
		[['listTimeEntries', '2026-05-01', '2026-05-31']]
	);
});

test('loadInitialTimesheetData skips the entries load when the date range is incomplete', async () => {
	let entryCalls = 0;

	const result = await loadInitialTimesheetData({
		listCustomers: async () => [],
		listProjects: async () => [],
		listIssues: async () => [],
		listTimeEntries: async () => {
			entryCalls += 1;
			return [];
		},
		getDateRange: () => ({ start: '', end: '' })
	});

	assert.equal(entryCalls, 0);
	assert.deepEqual(result.entries, []);
});
