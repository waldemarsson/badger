// Every page, for ⌘K (the main window's and the floating one's). The nav bar groups them
// in src/routes/+layout.svelte; `as const` keeps `resolve()` type-checked.
export const navLinks = [
	{ href: '/', label: 'Today' },
	{ href: '/tasks', label: 'Tasks' },
	{ href: '/notes', label: 'Notes' },
	{ href: '/playground', label: 'Playground' },
	{ href: '/timesheet', label: 'Timesheet' },
	{ href: '/travel', label: 'Travel' },
	{ href: '/reports/weekly', label: 'Weekly report' },
	{ href: '/reports/monthly', label: 'Monthly report' },
	{ href: '/settings', label: 'Settings' }
] as const;

export type Href = (typeof navLinks)[number]['href'];
