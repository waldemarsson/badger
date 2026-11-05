<script lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { onMount } from 'svelte';
import { today } from '$lib/date-ranges';
import type { Customer, Project, Issue, TodoRow } from '$lib/types';
import Combobox from '$lib/components/Combobox.svelte';
import Pencil from 'lucide-svelte/icons/pencil';
import Trash2 from 'lucide-svelte/icons/trash-2';
import Check from 'lucide-svelte/icons/check';
import X from 'lucide-svelte/icons/x';
import Link from 'lucide-svelte/icons/link';
import Unlink from 'lucide-svelte/icons/unlink';

let todos = $state<TodoRow[]>([]);
let showDone = $state(false);
let error = $state('');

// NEW TODO
let newTitle = $state('');
let newDueDate = $state('');
let newNotes = $state('');
let newError = $state('');

// EDIT
let editId = $state<number | null>(null);
let editTitle = $state('');
let editDueDate = $state('');
let editNotes = $state('');
let editError = $state('');

// LINK
let linkId = $state<number | null>(null);
let customers = $state<Customer[]>([]);
let linkCustomerId = $state('');
let linkProjectId = $state('');
let linkIssue = $state('');
let linkProjects = $state<Project[]>([]);
let linkIssues = $state<Issue[]>([]);
let linkError = $state('');

const todayStr = today();

function isOverdue(t: TodoRow): boolean {
return !t.done && t.due_date !== null && t.due_date < todayStr;
}

async function loadTodos(): Promise<void> {
try {
todos = await invoke<TodoRow[]>('list_todos', { includeDone: showDone });
} catch (e) {
error = String(e);
}
}

async function addTodo(): Promise<void> {
newError = '';
if (!newTitle.trim()) {
newError = 'Title is required';
return;
}
try {
await invoke('create_todo', {
title: newTitle,
notes: newNotes || null,
dueDate: newDueDate || null,
issueId: null
});
newTitle = '';
newDueDate = '';
newNotes = '';
await loadTodos();
} catch (e) {
newError = String(e);
}
}

async function toggleDone(t: TodoRow): Promise<void> {
try {
await invoke('set_todo_done', { id: t.id, done: !t.done });
await loadTodos();
} catch (e) {
error = String(e);
}
}

async function deleteTodo(t: TodoRow): Promise<void> {
if (!confirm(`Delete todo "${t.title}"?`)) return;
try {
await invoke('delete_todo', { id: t.id });
await loadTodos();
} catch (e) {
error = String(e);
}
}

function startEdit(t: TodoRow): void {
linkId = null;
editId = t.id;
editTitle = t.title;
editDueDate = t.due_date ?? '';
editNotes = t.notes ?? '';
editError = '';
}

async function saveEdit(t: TodoRow): Promise<void> {
editError = '';
if (!editTitle.trim()) {
editError = 'Title is required';
return;
}
try {
await invoke('update_todo', {
id: t.id,
title: editTitle,
notes: editNotes || null,
dueDate: editDueDate || null,
issueId: t.issue_id
});
editId = null;
await loadTodos();
} catch (e) {
editError = String(e);
}
}

async function unlink(t: TodoRow): Promise<void> {
try {
await invoke('update_todo', {
id: t.id,
title: t.title,
notes: t.notes,
dueDate: t.due_date,
issueId: null
});
await loadTodos();
} catch (e) {
error = String(e);
}
}

async function startLink(t: TodoRow): Promise<void> {
editId = null;
linkId = t.id;
linkError = '';
linkCustomerId = '';
linkProjectId = '';
linkIssue = t.title; // default new-issue title; pick from list to link an existing one
linkProjects = [];
linkIssues = [];
try {
customers = await invoke<Customer[]>('list_customers');
const defaultCustomer = customers.find((c) => c.use_as_default);
if (defaultCustomer) {
linkCustomerId = String(defaultCustomer.id);
await onLinkCustomerChange(linkCustomerId);
const defaultProject = linkProjects.find((p) => p.use_as_default);
if (defaultProject) {
linkProjectId = String(defaultProject.id);
await onLinkProjectChange(linkProjectId);
}
}
} catch (e) {
linkError = String(e);
}
}

async function onLinkCustomerChange(v: string): Promise<void> {
linkProjectId = '';
linkProjects = [];
linkIssues = [];
if (v) linkProjects = await invoke<Project[]>('list_projects', { customerId: parseInt(v) });
}

async function onLinkProjectChange(v: string): Promise<void> {
linkIssues = [];
if (v) linkIssues = await invoke<Issue[]>('list_issues', { projectId: parseInt(v) });
}

async function saveLink(t: TodoRow): Promise<void> {
linkError = '';
if (!linkProjectId) {
linkError = 'Pick a customer and project';
return;
}
if (!linkIssue.trim()) {
linkError = 'Issue is required';
return;
}
try {
// Reuses an existing issue with this title (or external id), otherwise creates it.
await invoke('create_issue_for_todo', {
todoId: t.id,
projectId: parseInt(linkProjectId),
issueTitle: linkIssue
});
linkId = null;
await loadTodos();
} catch (e) {
linkError = String(e);
}
}

onMount(loadTodos);
</script>

<div class="page-shell">
<div class="page-view">

<section class="page-hero">
<p class="page-hero-kicker">TimeTracker</p>
<h1 class="page-hero-title">Todos</h1>
<p class="page-hero-copy">Standalone todos, optionally linked to an issue.</p>
</section>

<div class="table-card">
<div class="table-toolbar">
<span class="table-title">Todos</span>
<label class="flex items-center gap-2 text-sm">
<input type="checkbox" bind:checked={showDone} onchange={loadTodos} />
Show done
</label>
</div>

{#if error}
<div class="stack-error mx-4 my-2">{error}</div>
{/if}

<div class="table-scroll-region">
<table class="table-base">
<thead class="table-head">
<tr>
<th class="table-heading w-10">Done</th>
<th class="table-heading">Todo</th>
<th class="table-heading w-36">Due</th>
<th class="table-heading w-64">Issue</th>
<th class="table-heading-right w-28">Actions</th>
</tr>
</thead>
<tbody>

<!-- New todo row -->
<tr class="table-row-new">
<td></td>
<td>
<input
class="form-input w-full"
placeholder="New todo…"
bind:value={newTitle}
onkeydown={(e) => e.key === 'Enter' && addTodo()}
/>
<input
class="form-input mt-1 w-full"
placeholder="Notes…"
bind:value={newNotes}
onkeydown={(e) => e.key === 'Enter' && addTodo()}
/>
{#if newError}<span class="form-error">{newError}</span>{/if}
</td>
<td><input type="date" class="form-input w-full" bind:value={newDueDate} /></td>
<td class="table-cell-muted">Link after creating</td>
<td>
<div class="flex justify-end gap-1">
<button onclick={addTodo} title="Add todo" class="btn-icon-confirm"><Check size={14} /></button>
</div>
</td>
</tr>

{#each todos as t (t.id)}
{#if editId === t.id}
<tr class="table-row-editing">
<td></td>
<td>
<input
class="form-input w-full"
bind:value={editTitle}
onkeydown={(e) => e.key === 'Enter' && saveEdit(t)}
/>
<input class="form-input mt-1 w-full" placeholder="Notes…" bind:value={editNotes} />
{#if editError}<span class="form-error">{editError}</span>{/if}
</td>
<td><input type="date" class="form-input w-full" bind:value={editDueDate} /></td>
<td class="table-cell-muted">
{t.issue_title ? `${t.customer_name} / ${t.project_name} / ${t.issue_title}` : '—'}
</td>
<td>
<div class="flex justify-end gap-1">
<button onclick={() => saveEdit(t)} title="Save" class="btn-icon-confirm"><Check size={14} /></button>
<button onclick={() => (editId = null)} title="Cancel" class="btn-icon-danger"><X size={14} /></button>
</div>
</td>
</tr>
{:else}
<tr class="table-row">
<td class="table-cell">
<input type="checkbox" checked={t.done} onchange={() => toggleDone(t)} title="Mark done" />
</td>
<td class="table-cell">
<div class:line-through={t.done} class:text-slate-400={t.done}>{t.title}</div>
{#if t.notes}<div class="table-cell-muted text-xs">{t.notes}</div>{/if}
</td>
<td class="table-cell" class:text-red-600={isOverdue(t)} class:font-semibold={isOverdue(t)}>
{t.due_date ?? '—'}
</td>
<td class="table-cell">
{#if t.issue_title}
<span class="table-cell-muted">{t.customer_name} / {t.project_name} /</span> {t.issue_title}
{:else}
<span class="table-cell-muted">—</span>
{/if}
</td>
<td class="table-cell">
<div class="flex justify-end gap-1">
<button onclick={() => startEdit(t)} title="Edit" class="btn-icon-secondary"><Pencil size={14} /></button>
{#if t.issue_id !== null}
<button onclick={() => unlink(t)} title="Unlink issue" class="btn-icon-secondary"><Unlink size={14} /></button>
{:else}
<button onclick={() => startLink(t)} title="Link issue" class="btn-icon-secondary"><Link size={14} /></button>
{/if}
<button onclick={() => deleteTodo(t)} title="Delete" class="btn-icon-danger"><Trash2 size={14} /></button>
</div>
</td>
</tr>
{/if}

{#if linkId === t.id}
<tr class="table-row-editing">
<td></td>
<td colspan="3">
<div class="grid grid-cols-1 gap-2 md:grid-cols-3">
<Combobox
items={customers.map((c) => ({ value: c.id, label: c.name }))}
bind:value={linkCustomerId}
placeholder="Customer…"
onchange={onLinkCustomerChange}
/>
<Combobox
items={linkProjects.map((p) => ({ value: p.id, label: p.name }))}
bind:value={linkProjectId}
placeholder="Project…"
disabled={!linkCustomerId}
onchange={onLinkProjectChange}
/>
<Combobox
items={linkIssues.map((i) => ({ value: i.title, label: i.title }))}
bind:value={linkIssue}
placeholder="Existing or new issue…"
allowNew={true}
disabled={!linkProjectId}
/>
</div>
{#if linkError}<span class="form-error">{linkError}</span>{/if}
</td>
<td>
<div class="flex justify-end gap-1">
<button onclick={() => saveLink(t)} title="Link" class="btn-icon-confirm"><Check size={14} /></button>
<button onclick={() => (linkId = null)} title="Cancel" class="btn-icon-danger"><X size={14} /></button>
</div>
</td>
</tr>
{/if}
{:else}
<tr><td colspan="5" class="table-cell table-cell-muted">No todos.</td></tr>
{/each}

</tbody>
</table>
</div>
</div>

</div>
</div>
