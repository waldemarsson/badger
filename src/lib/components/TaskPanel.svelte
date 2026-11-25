<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { resolve } from '$app/paths';
	import { tick } from 'svelte';
	import { fade } from 'svelte/transition';
	import type { Customer, Issue, Project, Task, TaskInput, TaskRepeat, TaskReminder, TaskReminderInput } from '#lib/types.js';
	import {
		MAX_REMINDERS,
		customReminderStart,
		dueEdit,
		formatDay,
		needsAttention,
		reminderLabel,
		reminderPresets,
		reminderStatus,
		taskInput
	} from '#lib/tasks.js';
	import {
		addTaskReminder,
		createIssueForTask,
		deleteTask,
		deleteTaskReminder,
		updateTask,
		updateTaskReminder
	} from '#lib/tasks-state.svelte.js';
	import { appleSync } from '#lib/apple-sync-state.svelte.js';
	import { taskMacOnlyControl } from '#lib/apple-sync.js';
	import { requestLogTime } from '#lib/palette-state.svelte.js';
	import Combobox from '#lib/components/Combobox.svelte';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import TaskCheck from '#lib/components/TaskCheck.svelte';
	import Bell from '@lucide/svelte/icons/bell';
	import BellRing from '@lucide/svelte/icons/bell-ring';
	import BellOff from '@lucide/svelte/icons/bell-off';
	import Clock from '@lucide/svelte/icons/clock';
	import FileText from '@lucide/svelte/icons/file-text';
	import Link2 from '@lucide/svelte/icons/link-2';
	import Plus from '@lucide/svelte/icons/plus';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import X from '@lucide/svelte/icons/x';

	// The details side panel on /tasks: every field saves on its own (change/blur), errors
	// show under the field that caused them, and the task re-renders from the shared list.
	let {
		task,
		now,
		notes,
		ticking = false,
		ontoggle,
		onclose
	}: {
		task: Task;
		now: Date;
		/** Notes to link (`get_link_targets`). */
		notes: { id: number; title: string }[];
		ticking?: boolean;
		ontoggle: () => void;
		onclose: () => void;
	} = $props();

	type Field = 'title' | 'notes' | 'due' | 'reminders' | 'repeat' | 'issue' | 'note' | 'mac' | 'delete';
	let error = $state<{ at: Field; message: string } | null>(null);
	let confirmDelete = $state(false);

	// Title and notes: local while being edited, otherwise they follow the task (sync, undo).
	let title = $state('');
	let notesText = $state('');
	let editingTitle = false;
	let editingNotes = false;
	$effect(() => {
		const t = task.title;
		if (!editingTitle) title = t;
	});
	$effect(() => {
		const n = task.notes ?? '';
		if (!editingNotes) notesText = n;
	});

	// Another task opened: start clean.
	let shownId = 0;
	$effect(() => {
		if (task.id === shownId) return;
		shownId = task.id;
		error = null;
		confirmDelete = false;
		menu = null;
		linking = false;
	});

	async function save(at: Field, patch: Partial<TaskInput>): Promise<boolean> {
		error = null;
		try {
			await updateTask(task.id, taskInput(task, patch));
			return true;
		} catch (e) {
			error = { at, message: String(e) };
			return false;
		}
	}

	async function run(at: Field, action: () => Promise<unknown>): Promise<void> {
		error = null;
		try {
			await action();
		} catch (e) {
			error = { at, message: String(e) };
		}
	}

	async function commitTitle(): Promise<void> {
		editingTitle = false;
		const next = title.trim();
		if (next === task.title) return;
		if (!next) {
			error = { at: 'title', message: 'Title is required' };
			title = task.title;
			return;
		}
		if (!(await save('title', { title: next }))) title = task.title;
	}

	function onTitleKeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		if (e.key === 'Enter') {
			e.preventDefault();
			(e.currentTarget as HTMLInputElement).blur();
		} else if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			title = task.title;
			editingTitle = false;
			(e.currentTarget as HTMLInputElement).blur();
		}
	}

	async function commitNotes(): Promise<void> {
		editingNotes = false;
		if (notesText === (task.notes ?? '')) return;
		await save('notes', { notes: notesText.trim() ? notesText : null });
	}

	// ── Due and repeat ──────────────────────────────────────────────────────────

	function onDue(date: string | null, time: string | null): void {
		const patch = dueEdit(date, time, now);
		// Weekly/monthly rules defaulted from the old date follow the new one.
		if (patch.dueDate && patch.dueDate !== task.due_date) {
			if (task.repeat === 'monthly') patch.monthDay = null;
			if (task.repeat === 'weekly' && task.weekdays.length <= 1) patch.weekdays = [];
		}
		void save('due', patch);
	}

	/** Weekly and monthly start from the due date's weekday / day of month (Rust fills them in). */
	function onRepeat(value: TaskRepeat): void {
		void save('repeat', { repeat: value, weekdays: [], monthDay: null });
	}

	function toggleWeekday(day: number): void {
		const days = task.weekdays.includes(day) ? task.weekdays.filter((d) => d !== day) : [...task.weekdays, day];
		void save('repeat', { weekdays: days.sort((a, b) => a - b) });
	}

	const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
	const REPEATS: { value: TaskRepeat; label: string }[] = [
		{ value: 'none', label: 'Never' },
		{ value: 'daily', label: 'Daily' },
		{ value: 'weekdays', label: 'Weekdays' },
		{ value: 'weekly', label: 'Weekly' },
		{ value: 'monthly', label: 'Monthly' }
	];

	// ── Reminders ───────────────────────────────────────────────────────────────

	let menu = $state<{ x: number; y: number; h: number; button: HTMLElement } | null>(null);
	let remindersEl = $state<HTMLElement>();
	const full = $derived(task.reminders.length >= MAX_REMINDERS);

	function openMenu(button: HTMLElement): void {
		const rect = button.getBoundingClientRect();
		menu = { x: rect.left, y: rect.bottom + 6, h: rect.height + 6, button };
	}

	function closeMenu(refocus: boolean): void {
		const button = menu?.button;
		menu = null;
		if (refocus && button?.isConnected) button.focus();
	}

	// ContextMenu calls onclose before an item's run, so items must not read `menu`.
	const menuItems = $derived<MenuItem[]>([
		...reminderPresets(task, now).map((p) => ({
			label: p.label,
			hint: p.hint,
			disabled: p.disabled,
			run: () => void addReminder({ offsetMinutes: p.offset })
		})),
		{ label: 'Custom…', hint: 'Pick a date and time', separator: true, disabled: full, run: () => void addReminder({ at: customReminderStart(task, new Date()) }) }
	]);

	async function addReminder(reminder: TaskReminderInput): Promise<void> {
		const before = new Set(task.reminders.map((r) => r.id));
		await run('reminders', async () => {
			const saved = await addTaskReminder(task.id, reminder);
			if (reminder.at === undefined) return;
			const added = saved.reminders.find((r) => !before.has(r.id));
			await tick();
			if (added) remindersEl?.querySelector<HTMLInputElement>(`[data-reminder-input="${added.id}"]`)?.focus();
		});
	}

	function onCustomAt(r: TaskReminder, value: string): void {
		if (!value || value === r.at?.slice(0, 16)) return;
		void run('reminders', () => updateTaskReminder(r.id, { at: value }));
	}

	// ── Issue ───────────────────────────────────────────────────────────────────

	let linking = $state(false);
	let customers = $state<Customer[]>([]);
	let projects = $state<Project[]>([]);
	let issues = $state<Issue[]>([]);
	let customerId = $state('');
	let projectId = $state('');
	let issueTitle = $state('');

	async function startLink(): Promise<void> {
		linking = true;
		issueTitle = task.title; // default new-issue title; pick from the list to reuse one
		await run('issue', async () => {
			customers = await invoke<Customer[]>('list_customers');
			const c = customers.find((x) => x.use_as_default);
			if (!c) return;
			customerId = String(c.id);
			await onCustomerChange(customerId);
			const p = projects.find((x) => x.use_as_default);
			if (!p) return;
			projectId = String(p.id);
			await onProjectChange(projectId);
		});
	}

	async function onCustomerChange(v: string): Promise<void> {
		projectId = '';
		projects = [];
		issues = [];
		if (v) projects = await invoke<Project[]>('list_projects', { customerId: parseInt(v) });
	}

	async function onProjectChange(v: string): Promise<void> {
		issues = [];
		if (v) issues = await invoke<Issue[]>('list_issues', { projectId: parseInt(v) });
	}

	async function link(): Promise<void> {
		if (!projectId) {
			error = { at: 'issue', message: 'Pick a customer and project' };
			return;
		}
		if (!issueTitle.trim()) {
			error = { at: 'issue', message: 'Issue is required' };
			return;
		}
		// Reuses an existing issue with this title, otherwise creates it.
		await run('issue', async () => {
			await createIssueForTask(task.id, parseInt(projectId), issueTitle);
			linking = false;
		});
	}

	function logTime(): void {
		if (task.project_id !== null && task.issue_title) {
			requestLogTime({ projectId: task.project_id, issueId: task.issue_id, issueTitle: task.issue_title });
		}
	}

	// ── Note, Mac only, delete ──────────────────────────────────────────────────

	let noteValue = $state('');
	const noteItems = $derived(notes.map((n) => ({ value: n.id, label: n.title })));

	function onNotePick(v: string): void {
		const id = parseInt(v);
		if (!Number.isFinite(id)) return;
		noteValue = '';
		void save('note', { noteId: id });
	}

	const macControl = $derived(taskMacOnlyControl(appleSync.on, task.repeat));

	async function remove(): Promise<void> {
		await run('delete', async () => {
			await deleteTask(task.id);
			onclose();
		});
	}

	/** Esc leaves a field first (blurs it), then closes the panel. */
	function onPanelKeydown(e: KeyboardEvent): void {
		if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing || menu) return;
		e.preventDefault();
		const target = e.target;
		if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) {
			target.blur();
			return;
		}
		onclose();
	}
</script>

{#snippet fieldError(at: Field)}
	{#if error?.at === at}
		<p class="form-error text-xs" role="alert" transition:fade={{ duration: 150 }}>{error.message}</p>
	{/if}
{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="flex flex-col gap-5" onkeydown={onPanelKeydown}>
	<div class="flex items-center justify-between">
		<span class="task-eyebrow">Task</span>
		<button type="button" class="btn-icon-ghost" aria-label="Close details" title="Close (Esc)" onclick={onclose}><X size={16} /></button>
	</div>

	<div class="space-y-1.5">
		<div class="flex items-start gap-3">
			<div class="mt-0.5">
				<TaskCheck
					done={task.done}
					attention={needsAttention(task, now)}
					{ticking}
					label={task.done ? `Mark ${task.title} as not done` : `Complete ${task.title}`}
					onclick={ontoggle}
				/>
			</div>
			<input
				class="task-title-input"
				aria-label="Title"
				maxlength={500}
				bind:value={title}
				onfocus={() => (editingTitle = true)}
				onblur={() => void commitTitle()}
				onkeydown={onTitleKeydown}
			/>
		</div>
		{@render fieldError('title')}
	</div>

	<div class="flex flex-col gap-1.5">
		<label class="form-label" for="task-notes">Notes</label>
		<textarea
			id="task-notes"
			class="form-textarea"
			rows="3"
			placeholder="Add notes"
			bind:value={notesText}
			onfocus={() => (editingNotes = true)}
			onblur={() => void commitNotes()}
		></textarea>
		{@render fieldError('notes')}
	</div>

	<div class="flex flex-col gap-1.5">
		<span class="form-label">Due</span>
		<div class="grid grid-cols-2 gap-2">
			<input
				type="date"
				class="form-input"
				aria-label="Due date"
				value={task.due_date ?? ''}
				onchange={(e) => onDue(e.currentTarget.value || null, task.due_time)}
			/>
			<div class="relative">
				<input
					type="time"
					class="form-input"
					aria-label="Due time (optional)"
					value={task.due_time ?? ''}
					onchange={(e) => onDue(task.due_date, e.currentTarget.value || null)}
				/>
			</div>
		</div>
		<div class="flex items-start justify-between gap-3">
			<p class="task-help">The due date alone never notifies. Without a time, reminders count from 09:00.</p>
			{#if task.due_date}
				<button type="button" class="btn-secondary-sm shrink-0" onclick={() => onDue(null, null)}>Clear</button>
			{/if}
		</div>
		{@render fieldError('due')}
	</div>

	<div class="flex flex-col gap-1.5" bind:this={remindersEl}>
		<span class="form-label">Reminders</span>
		{#each task.reminders as r (r.id)}
			{@const status = reminderStatus(r, task.done, now)}
			<div
				class="task-reminder"
				class:task-reminder-sent={status.tone === 'sent'}
				class:task-reminder-muted={status.tone === 'dismissed' || status.tone === 'dormant'}
				transition:fade={{ duration: 150 }}
			>
				{#if status.tone === 'sent'}
					<BellRing size={14} class="shrink-0" aria-hidden="true" />
				{:else if status.tone === 'dormant' || status.tone === 'dismissed'}
					<BellOff size={14} class="shrink-0" aria-hidden="true" />
				{:else}
					<Bell size={14} class="shrink-0" aria-hidden="true" />
				{/if}
				<span class="task-reminder-label">{reminderLabel(r)}</span>
				{#if r.at !== null}
					<input
						type="datetime-local"
						class="task-reminder-input"
						aria-label="Reminder time"
						data-reminder-input={r.id}
						value={r.at.slice(0, 16)}
						onchange={(e) => onCustomAt(r, e.currentTarget.value)}
					/>
				{:else}
					<span class="task-reminder-when">{status.when}</span>
				{/if}
				<button type="button" class="btn-icon-ghost p-1" aria-label="Remove reminder {reminderLabel(r)}" onclick={() => void run('reminders', () => deleteTaskReminder(r.id))}>
					<X size={14} />
				</button>
			</div>
		{:else}
			<p class="task-help">No reminders yet.</p>
		{/each}
		<div class="flex items-center gap-2">
			<button
				type="button"
				class="btn-secondary-sm gap-1.5"
				aria-haspopup="menu"
				aria-expanded={menu !== null}
				disabled={full}
				onclick={(e) => (menu ? closeMenu(false) : openMenu(e.currentTarget))}
			>
				<Plus size={14} /> Add reminder
			</button>
			{#if full}
				<span class="task-help">At most {MAX_REMINDERS} reminders</span>
			{/if}
		</div>
		{@render fieldError('reminders')}
	</div>

	<div class="flex flex-col gap-1.5">
		<label class="form-label" for="task-repeat">Repeat</label>
		<select
			id="task-repeat"
			class="form-select"
			value={task.repeat}
			disabled={!task.due_date}
			onchange={(e) => onRepeat(e.currentTarget.value as TaskRepeat)}
		>
			{#each REPEATS as r (r.value)}
				<option value={r.value}>{r.label}</option>
			{/each}
		</select>
		{#if task.repeat === 'weekly'}
			<div class="flex flex-wrap gap-1" role="group" aria-label="Repeat on">
				{#each WEEKDAYS as day, i (day)}
					<button
						type="button"
						class="task-day"
						class:task-day-on={task.weekdays.includes(i + 1)}
						aria-pressed={task.weekdays.includes(i + 1)}
						onclick={() => toggleWeekday(i + 1)}
					>{day}</button>
				{/each}
			</div>
		{/if}
		<p class="task-help">
			{#if !task.due_date}
				Set a due date to repeat.
			{:else if task.repeat !== 'none'}
				Completing moves the due to the next date; the task stays open.
			{:else}
				A one-off task.
			{/if}
		</p>
		{@render fieldError('repeat')}
	</div>

	<div class="flex flex-col gap-1.5">
		<span class="form-label">Issue</span>
		{#if task.issue_id !== null}
			<div class="task-link-field">
				<span class="task-chip task-chip-violet min-w-0" title="{task.customer_name} / {task.project_name} / {task.issue_title}">
					<Link2 size={12} class="shrink-0" /><span class="truncate">{task.project_name} · {task.issue_title}</span>
				</span>
				<span class="flex shrink-0 items-center">
					<button type="button" class="btn-icon-ghost p-1" aria-label="Log time on {task.issue_title}" title="Log time" onclick={logTime}><Clock size={14} /></button>
					<button type="button" class="btn-icon-ghost p-1" aria-label="Unlink issue" title="Unlink issue" onclick={() => void save('issue', { issueId: null })}><X size={14} /></button>
				</span>
			</div>
		{:else if linking}
			<div class="space-y-2 rounded-xl bg-white p-3 ring-1 ring-slate-200 dark:bg-neutral-900 dark:ring-neutral-800" transition:fade={{ duration: 150 }}>
				<p class="task-help">Pick an existing issue, or type a new title to create one.</p>
				<Combobox items={customers.map((c) => ({ value: c.id, label: c.name }))} bind:value={customerId} placeholder="Customer…" onchange={(v) => void onCustomerChange(v)} />
				<Combobox items={projects.map((p) => ({ value: p.id, label: p.name }))} bind:value={projectId} placeholder="Project…" disabled={!customerId} onchange={(v) => void onProjectChange(v)} />
				<Combobox items={issues.map((i) => ({ value: i.title, label: i.title }))} bind:value={issueTitle} placeholder="Issue…" allowNew={true} disabled={!projectId} />
				<div class="flex justify-end gap-2">
					<button type="button" class="btn-secondary-sm" onclick={() => (linking = false)}>Cancel</button>
					<button type="button" class="btn-secondary-sm" onclick={() => void link()}>Link</button>
				</div>
			</div>
		{:else}
			<button type="button" class="btn-secondary-sm gap-1.5 self-start" onclick={() => void startLink()}><Link2 size={13} /> Link issue</button>
		{/if}
		{@render fieldError('issue')}
	</div>

	<div class="flex flex-col gap-1.5">
		<span class="form-label">Note</span>
		{#if task.note_id !== null}
			<div class="task-link-field">
				<a href="{resolve('/notes')}?id={task.note_id}" class="task-chip task-chip-slate min-w-0 hover:ring-slate-300 dark:hover:ring-neutral-700">
					<FileText size={12} class="shrink-0" /><span class="truncate">{task.note_title}</span>
				</a>
				<button type="button" class="btn-icon-ghost p-1" aria-label="Unlink note" title="Unlink note" onclick={() => void save('note', { noteId: null })}><X size={14} /></button>
			</div>
		{:else}
			<Combobox items={noteItems} bind:value={noteValue} placeholder="Link a note…" picker={true} onchange={onNotePick} />
		{/if}
		{@render fieldError('note')}
	</div>

	{#if macControl !== 'hidden'}
		<div class="flex items-center justify-between gap-4" transition:fade={{ duration: 150 }}>
			<span>
				<span class="block text-sm font-medium text-slate-950 dark:text-neutral-100" id="task-mac-label">Keep off phone</span>
				<span class="task-help block">{macControl === 'repeating' ? 'Repeating tasks stay on the Mac' : 'Stays out of Apple Reminders'}</span>
			</span>
			<button
				type="button"
				role="switch"
				class="task-switch"
				class:task-switch-on={task.mac_only || macControl === 'repeating'}
				aria-checked={task.mac_only || macControl === 'repeating'}
				aria-labelledby="task-mac-label"
				disabled={macControl === 'repeating'}
				onclick={() => void save('mac', { macOnly: !task.mac_only })}
			><span class="task-switch-knob"></span></button>
		</div>
		{@render fieldError('mac')}
	{/if}

	<div class="flex flex-wrap items-center justify-between gap-3 border-t border-slate-200 pt-4 dark:border-neutral-800">
		<span class="text-xs text-slate-500 dark:text-neutral-400">Created {formatDay(task.created_at.slice(0, 10), now)}</span>
		{#if confirmDelete}
			<span class="inline-flex items-center gap-2" transition:fade={{ duration: 150 }}>
				<span class="text-xs text-red-700 dark:text-red-400">Delete this task?</span>
				<button type="button" class="btn-danger-sm" onclick={() => void remove()}>Delete</button>
				<button type="button" class="btn-secondary-sm" onclick={() => (confirmDelete = false)}>Cancel</button>
			</span>
		{:else}
			<button type="button" class="btn-danger-sm gap-1.5" onclick={() => (confirmDelete = true)}><Trash2 size={14} /> Delete</button>
		{/if}
	</div>
	{@render fieldError('delete')}
</div>

{#if menu}
	<ContextMenu x={menu.x} y={menu.y} anchorHeight={menu.h} label="Add reminder" items={menuItems} onclose={closeMenu} />
{/if}
