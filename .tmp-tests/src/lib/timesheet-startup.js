export async function loadInitialTimesheetData(deps) {
    const customers = await deps.listCustomers();
    let projects = [];
    let issues = [];
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
