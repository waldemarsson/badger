<script lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { onMount, untrack } from 'svelte';
import { page } from '$app/state';
import { getMonthBounds, getWeekBounds, today } from '#lib/date-ranges.js';
import { parseDayParam } from '#lib/today.js';
import { loadInitialTimesheetData } from '#lib/timesheet-startup.js';
import type { Customer, Project, Issue, TimeEntryRow } from '#lib/types.js';
import Combobox from '#lib/components/Combobox.svelte';
import { changes } from '#lib/palette-state.svelte.js';
import Pencil from '@lucide/svelte/icons/pencil';
import Trash2 from '@lucide/svelte/icons/trash';
import Check from '@lucide/svelte/icons/check';
import X from '@lucide/svelte/icons/x';

// DATA
let customers = $state<Customer[]>([]);
let projects = $state<Project[]>([]);
let issues = $state<Issue[]>([]);
let entries = $state<TimeEntryRow[]>([]);
let entriesLoading = $state(true);
let canReloadEntries = false;

// DATE FILTER — `?day=YYYY-MM-DD` (from Today's week strip) opens the Single Day filter.
let initialDay = parseDayParam(page.url.searchParams.get('day'));
let dateFilter = $state(initialDay ? 'single_day' : 'this_month');
let singleDay = $state(initialDay ?? today());
let customStart = $state('');
let customEnd = $state('');

// NEW ENTRY ROW (always visible)
let newDate = $state('');
let newCustomerId = $state('');
let newProjectId = $state('');
let newIssueIdentifier = $state('');
let newHours = $state('');
let newNotes = $state('');
let newErrors = $state<Record<string, string>>({});
let newSaving = $state(false);
let newSaveError = $state('');

// EDIT ROW
let editId = $state<number | null>(null);
let editDate = $state('');
let editCustomerId = $state('');
let editProjectId = $state('');
let editIssueIdentifier = $state('');
let editHours = $state('');
let editNotes = $state('');
let editErrors = $state<Record<string, string>>({});
let editSaving = $state(false);
let editSaveError = $state('');
let editProjects = $state<Project[]>([]);
let editIssues = $state<Issue[]>([]);

// DELETE ERROR
let deleteError = $state('');

function formatHours(h: number): string {
const r = Math.round(h * 10000) / 10000;
return Number.isInteger(r) ? String(Math.round(r)) : String(r);
}

function getDateRange(): { start: string; end: string } {
if (dateFilter === 'this_week') return getWeekBounds(0);
if (dateFilter === 'last_week') return getWeekBounds(-1);
if (dateFilter === 'this_month') return getMonthBounds(0);
if (dateFilter === 'last_month') return getMonthBounds(-1);
if (dateFilter === 'single_day') return { start: singleDay, end: singleDay };
return { start: customStart, end: customEnd };
}

async function loadEntries(): Promise<void> {
const { start, end } = getDateRange();
if (!start || !end) return;
entries = await invoke<TimeEntryRow[]>('list_time_entries', { startDate: start, endDate: end });
}

async function loadProjects(customerId: number): Promise<Project[]> {
return invoke<Project[]>('list_projects', { customerId });
}

async function loadIssues(projectId: number): Promise<Issue[]> {
return invoke<Issue[]>('list_issues', { projectId });
}

async function onNewCustomerChange(v: string): Promise<void> {
newProjectId = '';
newIssueIdentifier = '';
projects = [];
issues = [];
if (v) projects = await loadProjects(parseInt(v));
}

async function onNewProjectChange(v: string): Promise<void> {
newIssueIdentifier = '';
issues = [];
if (v) issues = await loadIssues(parseInt(v));
}

async function resetNewRow(): Promise<void> {
newDate = today();
newErrors = {};
newSaveError = '';
newHours = '';
newNotes = '';
newCustomerId = '';
newProjectId = '';
newIssueIdentifier = '';
projects = [];
issues = [];
const defaultCustomer = customers.find((c) => c.use_as_default) ?? null;
if (defaultCustomer) {
newCustomerId = String(defaultCustomer.id);
projects = await loadProjects(defaultCustomer.id);
const defaultProject = projects.find((p) => p.use_as_default) ?? null;
if (defaultProject) {
newProjectId = String(defaultProject.id);
issues = await loadIssues(defaultProject.id);
}
}
}

function validateEntry(
date: string,
customerId: string,
projectId: string,
issueIdentifier: string,
hours: string
): Record<string, string> {
const errs: Record<string, string> = {};
if (!date) errs.date = 'Required';
if (!customerId) errs.customer = 'Required';
if (!projectId) errs.project = 'Required';
if (!issueIdentifier.trim()) errs.issue = 'Required';
const h = parseFloat(hours);
if (!hours) errs.hours = 'Required';
else if (isNaN(h) || h <= 0) errs.hours = 'Must be > 0';
else if (!Number.isInteger(h)) errs.hours = 'Must be a whole hour';
return errs;
}

async function saveEntry(): Promise<void> {
newErrors = validateEntry(newDate, newCustomerId, newProjectId, newIssueIdentifier, newHours);
if (Object.keys(newErrors).length > 0) return;
newSaving = true;
newSaveError = '';
try {
await invoke<TimeEntryRow>('create_time_entry', {
projectId: parseInt(newProjectId),
issueIdentifier: newIssueIdentifier.trim(),
date: newDate,
hours: parseFloat(newHours),
notes: newNotes.trim() || null
});
await loadEntries();
await resetNewRow();
} catch {
newSaveError = 'Failed to save.';
} finally {
newSaving = false;
}
}

async function startEdit(entry: TimeEntryRow): Promise<void> {
if (editId !== null) discardEdit();
editId = entry.id;
editDate = entry.date;
editCustomerId = String(entry.customer_id);
editProjectId = String(entry.project_id);
editIssueIdentifier = entry.issue_title;
editHours = String(entry.hours);
editNotes = entry.notes ?? '';
editErrors = {};
editSaveError = '';
editProjects = await loadProjects(entry.customer_id);
editIssues = await loadIssues(entry.project_id);
}

function discardEdit(): void {
editId = null;
editErrors = {};
editSaveError = '';
}

async function onEditCustomerChange(v: string): Promise<void> {
editProjectId = '';
editIssueIdentifier = '';
editProjects = [];
editIssues = [];
if (v) editProjects = await loadProjects(parseInt(v));
}

async function onEditProjectChange(v: string): Promise<void> {
editIssueIdentifier = '';
editIssues = [];
if (v) editIssues = await loadIssues(parseInt(v));
}

async function updateEntry(): Promise<void> {
editErrors = validateEntry(editDate, editCustomerId, editProjectId, editIssueIdentifier, editHours);
if (Object.keys(editErrors).length > 0) return;
editSaving = true;
editSaveError = '';
try {
const updated = await invoke<TimeEntryRow>('update_time_entry', {
id: editId,
projectId: parseInt(editProjectId),
issueIdentifier: editIssueIdentifier.trim(),
date: editDate,
hours: parseFloat(editHours),
notes: editNotes.trim() || null
});
entries = entries.map((e) => (e.id === updated.id ? updated : e));
editId = null;
await loadEntries();
} catch {
editSaveError = 'Failed to save.';
} finally {
editSaving = false;
}
}

async function deleteEntry(id: number): Promise<void> {
deleteError = '';
try {
await invoke('delete_time_entry', { id });
entries = entries.filter((e) => e.id !== id);
} catch {
deleteError = 'Failed to delete entry.';
}
}

onMount(async () => {
entriesLoading = true;
const initial = await loadInitialTimesheetData({
listCustomers: () => invoke<Customer[]>('list_customers'),
listProjects: (customerId) => loadProjects(customerId),
listIssues: (projectId) => loadIssues(projectId),
listTimeEntries: (startDate, endDate) =>
invoke<TimeEntryRow[]>('list_time_entries', { startDate, endDate }),
getDateRange
});

customers = initial.customers;
projects = initial.projects;
issues = initial.issues;
entries = initial.entries;
newDate = today();
newErrors = {};
newSaveError = '';
newHours = '';
newNotes = '';
newCustomerId = initial.newCustomerId;
newProjectId = initial.newProjectId;
newIssueIdentifier = '';
entriesLoading = false;
canReloadEntries = true;
});

$effect(() => {
void dateFilter;
void singleDay;
void customStart;
void customEnd;
void changes.timeEntries;
if (!canReloadEntries) return;
entriesLoading = true;
void loadEntries().finally(() => {
entriesLoading = false;
});
});

// The palette may have created an issue in the selected project; refresh the picker list.
let seenEntryChanges = changes.timeEntries;
$effect(() => {
if (changes.timeEntries === seenEntryChanges) return;
seenEntryChanges = changes.timeEntries;
const projectId = parseInt(untrack(() => newProjectId));
if (projectId) void loadIssues(projectId).then((list) => (issues = list));
});

// Same-route navigation to another `?day=` (the page stays mounted) switches the filter too.
let seenDayParam = page.url.searchParams.get('day');
$effect(() => {
const param = page.url.searchParams.get('day');
if (param === seenDayParam) return;
seenDayParam = param;
const day = parseDayParam(param);
if (day) {
singleDay = day;
dateFilter = 'single_day';
}
});

let totalHours = $derived(entries.reduce((sum, e) => sum + e.hours, 0));
</script>

<div class="page-shell">
<div class="page-view">

<h1 class="sr-only">Timesheet</h1>

<!-- Main table card -->
<section class="table-card">

<!-- Toolbar -->
<div class="table-toolbar flex items-center justify-between">
<div class="toolbar-stat">
<span class="toolbar-stat-label">Total</span>
<span class="toolbar-stat-value">{formatHours(totalHours)} <span class="toolbar-stat-muted">h</span></span>
</div>
<div class="flex items-center gap-2">
<select bind:value={dateFilter} class="form-select w-40">
<option value="this_week">This Week</option>
<option value="last_week">Last Week</option>
<option value="this_month">This Month</option>
<option value="last_month">Last Month</option>
<option value="single_day">Single Day</option>
<option value="custom">Custom Range…</option>
</select>
{#if dateFilter === 'single_day'}
<input type="date" bind:value={singleDay} class="form-input w-36" aria-label="Day" />
{/if}
{#if dateFilter === 'custom'}
<input type="date" bind:value={customStart} class="form-input w-36" />
<input type="date" bind:value={customEnd} class="form-input w-36" />
{/if}
</div>
</div>

{#if deleteError}
<div class="stack-error mx-4 my-2">{deleteError}</div>
{/if}

<!-- Table: Date | Customer | Project | Issue | Notes | Hours | Actions -->
<div class="table-scroll-region">
<table class="table-base">
<thead class="table-head">
<tr>
<th class="table-heading w-28">Date</th>
<th class="table-heading w-44">Customer</th>
<th class="table-heading w-44">Project</th>
<th class="table-heading w-44">Issue</th>
<th class="table-heading">Notes</th>
<th class="table-heading w-28 text-right">Hours</th>
<th class="table-heading-right w-16">Actions</th>
</tr>
</thead>
<tbody>

<!-- Always-visible new entry row -->
<tr class="table-row-new">
<td>
<input type="date" bind:value={newDate} class="form-input w-full" />
{#if newErrors.date}<span class="form-error">{newErrors.date}</span>{/if}
</td>
<td>
<Combobox
items={customers.map((c) => ({ value: c.id, label: c.name }))}
bind:value={newCustomerId}
placeholder="Customer…"
onchange={onNewCustomerChange}
/>
{#if newErrors.customer}<span class="form-error">{newErrors.customer}</span>{/if}
</td>
<td>
<Combobox
items={projects.map((p) => ({ value: p.id, label: p.name }))}
bind:value={newProjectId}
placeholder="Project…"
disabled={!newCustomerId}
onchange={onNewProjectChange}
/>
{#if newErrors.project}<span class="form-error">{newErrors.project}</span>{/if}
</td>
<td>
<Combobox
items={issues.map((i) => ({ value: i.title, label: i.title }))}
bind:value={newIssueIdentifier}
placeholder="Issue…"
allowNew={true}
disabled={!newProjectId}
/>
{#if newErrors.issue}<span class="form-error">{newErrors.issue}</span>{/if}
</td>
<td>
<input type="text" bind:value={newNotes} placeholder="Notes…" class="form-input w-full" />
</td>
<td>
<input type="number" step="1" min="1" max="24" bind:value={newHours} placeholder="0" class="form-input w-full text-right" />
{#if newErrors.hours}<span class="form-error text-right block">{newErrors.hours}</span>{/if}
</td>
<td>
<div class="flex justify-end gap-1">
<button onclick={saveEntry} disabled={newSaving} title="Save Entry" class="btn-icon-confirm">
<Check size={14} />
</button>
<button onclick={resetNewRow} title="Discard" class="btn-icon-danger">
<X size={14} />
</button>
</div>
{#if newSaveError}<div class="stack-error mt-1 text-xs">{newSaveError}</div>{/if}
</td>
</tr>

{#if entriesLoading}
<tr>
<td colspan="7">
<div class="empty-state">
<p class="font-semibold">Loading entries…</p>
<p>Please wait while Badger loads your timesheet.</p>
</div>
</td>
</tr>
{:else}
{#each entries as entry (entry.id)}
{#if editId === entry.id}
<tr class="table-row-editing">
<td>
<input type="date" bind:value={editDate} class="form-input w-full" />
{#if editErrors.date}<span class="form-error">{editErrors.date}</span>{/if}
</td>
<td>
<Combobox
items={customers.map((c) => ({ value: c.id, label: c.name }))}
bind:value={editCustomerId}
placeholder="Customer…"
onchange={onEditCustomerChange}
/>
{#if editErrors.customer}<span class="form-error">{editErrors.customer}</span>{/if}
</td>
<td>
<Combobox
items={editProjects.map((p) => ({ value: p.id, label: p.name }))}
bind:value={editProjectId}
placeholder="Project…"
disabled={!editCustomerId}
onchange={onEditProjectChange}
/>
{#if editErrors.project}<span class="form-error">{editErrors.project}</span>{/if}
</td>
<td>
<Combobox
items={editIssues.map((i) => ({ value: i.title, label: i.title }))}
bind:value={editIssueIdentifier}
placeholder="Issue…"
allowNew={true}
disabled={!editProjectId}
/>
{#if editErrors.issue}<span class="form-error">{editErrors.issue}</span>{/if}
</td>
<td>
<input type="text" bind:value={editNotes} placeholder="Notes…" class="form-input w-full" />
</td>
<td>
<input type="number" step="1" min="1" max="24" bind:value={editHours} placeholder="0" class="form-input w-full text-right" />
{#if editErrors.hours}<span class="form-error text-right block">{editErrors.hours}</span>{/if}
</td>
<td>
<div class="flex justify-end gap-1">
<button onclick={updateEntry} disabled={editSaving} title="Save Changes" class="btn-icon-confirm">
<Check size={14} />
</button>
<button onclick={discardEdit} title="Discard" class="btn-icon-danger">
<X size={14} />
</button>
</div>
{#if editSaveError}<div class="stack-error mt-1 text-xs">{editSaveError}</div>{/if}
</td>
</tr>
{:else}
<tr class="table-row-history">
<td class="table-cell-strong w-28">{entry.date}</td>
<td class="table-cell w-44">{entry.customer_name}</td>
<td class="table-cell w-44">{entry.project_name}</td>
<td class="table-cell w-44">{entry.issue_title}</td>
<td class="table-cell-muted max-w-xs truncate">{entry.notes ?? ''}</td>
<td class="table-cell w-28 text-right">{formatHours(entry.hours)}</td>
<td class="table-cell w-16">
<div class="flex justify-end gap-1">
<button onclick={() => startEdit(entry)} title="Edit" class="btn-icon-secondary">
<Pencil size={14} />
</button>
<button onclick={() => deleteEntry(entry.id)} title="Delete" class="btn-icon-danger">
<Trash2 size={14} />
</button>
</div>
</td>
</tr>
{/if}
{/each}

{#if entries.length === 0}
<tr>
<td colspan="7">
<div class="empty-state">
<p class="font-semibold">No entries</p>
<p>No time entries for this period.</p>
</div>
</td>
</tr>
{/if}
{/if}

</tbody>
</table>
</div>

</section>
</div>
</div>
