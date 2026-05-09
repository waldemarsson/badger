import type { Customer, Issue, Project, TimeEntryRow } from './types';

type DateRange = {
	start: string;
	end: string;
};

type TimesheetStartupDependencies = {
	listCustomers: () => Promise<Customer[]>;
	listProjects: (customerId: number) => Promise<Project[]>;
	listIssues: (projectId: number) => Promise<Issue[]>;
	listTimeEntries: (startDate: string, endDate: string) => Promise<TimeEntryRow[]>;
	getDateRange: () => DateRange;
};

export type InitialTimesheetData = {
	customers: Customer[];
	projects: Project[];
	issues: Issue[];
	entries: TimeEntryRow[];
	newCustomerId: string;
	newProjectId: string;
};

export async function loadInitialTimesheetData(
	deps: TimesheetStartupDependencies
): Promise<InitialTimesheetData> {
	const customers = await deps.listCustomers();
	let projects: Project[] = [];
	let issues: Issue[] = [];
	let newCustomerId = '';
	let newProjectId = '';

	const defaultCustomer = customers.find((customer) => customer.use_as_default) ?? null;
	if (defaultCustomer) {
		newCustomerId = String(defaultCustomer.id);
		projects = await deps.listProjects(defaultCustomer.id);

		const defaultProject = projects.find((project) => project.use_as_default) ?? null;
		if (defaultProject) {
			newProjectId = String(defaultProject.id);
			issues = await deps.listIssues(defaultProject.id);
		}
	}

	const { start, end } = deps.getDateRange();
	const entries = start && end ? await deps.listTimeEntries(start, end) : [];

	return {
		customers,
		projects,
		issues,
		entries,
		newCustomerId,
		newProjectId
	};
}
