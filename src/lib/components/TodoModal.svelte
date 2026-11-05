<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import type { Customer, Issue, Project, TodoRow, TodoStatus } from '#lib/types.js';
	import { COLUMNS } from '#lib/todos.js';
	import Combobox from '#lib/components/Combobox.svelte';
	import X from 'lucide-svelte/icons/x';
	import Link2 from 'lucide-svelte/icons/link-2';
	import Unlink from 'lucide-svelte/icons/unlink';
	import Trash2 from 'lucide-svelte/icons/trash-2';

	let {
		todo,
		onclose,
		onchange
	}: { todo: TodoRow; onclose: () => void; onchange: () => Promise<void> } = $props();

	// The modal is mounted per card, so the form starts from the initial prop value.
	// svelte-ignore state_referenced_locally
	const initial = todo;
	// Refreshed after link/unlink so the issue section stays current.
	let current = $state<TodoRow>({ ...initial });
	let title = $state(initial.title);
	let notes = $state(initial.notes ?? '');
	let dueDate = $state(initial.due_date ?? '');
	let status = $state<TodoStatus>(initial.status);
	let error = $state('');
	let saving = $state(false);
	let confirmDelete = $state(false);

	// Issue picker
	let linking = $state(false);
	let customers = $state<Customer[]>([]);
	let projects = $state<Project[]>([]);
	let issues = $state<Issue[]>([]);
	let customerId = $state('');
	let projectId = $state('');
	let issueTitle = $state('');

	async function save(): Promise<void> {
		error = '';
		if (!title.trim()) {
			error = 'Title is required';
			return;
		}
		saving = true;
		try {
			await invoke('update_todo', {
				id: current.id,
				title,
				notes: notes || null,
				dueDate: dueDate || null,
				issueId: current.issue_id
			});
			if (status !== current.status) {
				// Moved via the form → append to the bottom of the new column.
				await invoke('move_todo', { id: current.id, status, index: 2147483647 });
			}
			await onchange();
			onclose();
		} catch (e) {
			error = String(e);
		} finally {
			saving = false;
		}
	}

	async function remove(): Promise<void> {
		try {
			await invoke('delete_todo', { id: current.id });
			await onchange();
			onclose();
		} catch (e) {
			error = String(e);
		}
	}

	async function unlink(): Promise<void> {
		try {
			current = await invoke<TodoRow>('update_todo', {
				id: current.id,
				title: current.title,
				notes: current.notes,
				dueDate: current.due_date,
				issueId: null
			});
			await onchange();
		} catch (e) {
			error = String(e);
		}
	}

	async function startLink(): Promise<void> {
		linking = true;
		issueTitle = title; // default new-issue title; pick from the list to reuse an existing one
		try {
			customers = await invoke<Customer[]>('list_customers');
			const c = customers.find((x) => x.use_as_default);
			if (c) {
				customerId = String(c.id);
				await onCustomerChange(customerId);
				const p = projects.find((x) => x.use_as_default);
				if (p) {
					projectId = String(p.id);
					await onProjectChange(projectId);
				}
			}
		} catch (e) {
			error = String(e);
		}
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
		error = '';
		if (!projectId) {
			error = 'Pick a customer and project';
			return;
		}
		if (!issueTitle.trim()) {
			error = 'Issue is required';
			return;
		}
		try {
			// Reuses an existing issue with this title, otherwise creates it.
			current = await invoke<TodoRow>('create_issue_for_todo', {
				todoId: current.id,
				projectId: parseInt(projectId),
				issueTitle
			});
			linking = false;
			await onchange();
		} catch (e) {
			error = String(e);
		}
	}

	function onkeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') onclose();
		if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) save();
	}
</script>

<svelte:window {onkeydown} />

<div
	class="fixed inset-0 z-40 flex items-start justify-center overflow-y-auto bg-slate-950/40 dark:bg-black/60 p-4 pt-[10vh] backdrop-blur-[2px]"
	transition:fade={{ duration: 150 }}
	onclick={(e) => e.target === e.currentTarget && onclose()}
	role="presentation"
>
	<div
		class="w-full max-w-2xl rounded-2xl bg-white dark:bg-neutral-900 shadow-2xl ring-1 ring-slate-900/10 dark:ring-neutral-800"
		transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
		role="dialog"
		aria-modal="true"
		aria-label="Edit todo"
	>
		<div class="flex items-start gap-3 border-b border-slate-100 dark:border-neutral-800 px-6 pb-4 pt-5">
			<input
				class="flex-1 rounded-lg border-0 px-0 text-lg font-semibold text-slate-900 dark:text-neutral-100 dark:bg-transparent outline-none placeholder:text-slate-300 dark:placeholder:text-neutral-600 focus:ring-0"
				bind:value={title}
				placeholder="Title"
			/>
			<button class="rounded-lg p-1.5 text-slate-400 dark:text-neutral-500 transition hover:bg-slate-100 dark:hover:bg-neutral-800 hover:text-slate-700 dark:hover:text-neutral-300" onclick={onclose} title="Close (Esc)">
				<X size={18} />
			</button>
		</div>

		<div class="grid gap-6 px-6 py-5 sm:grid-cols-[1fr_14rem]">
			<label class="block">
				<span class="form-label">Notes</span>
				<textarea class="form-textarea mt-1.5 min-h-[9rem]" bind:value={notes} placeholder="Add more detail…"></textarea>
			</label>

			<div class="space-y-4">
				<label class="block">
					<span class="form-label">Status</span>
					<select class="form-select mt-1.5 w-full" bind:value={status}>
						{#each COLUMNS as col (col.status)}
							<option value={col.status}>{col.label}</option>
						{/each}
					</select>
				</label>
				<label class="block">
					<span class="form-label">Due date</span>
					<input type="date" class="form-input mt-1.5" bind:value={dueDate} />
				</label>
				<div>
					<span class="form-label">Issue</span>
					{#if current.issue_id !== null}
						<div class="mt-1.5 flex items-start gap-2 rounded-xl bg-violet-50 dark:bg-violet-500/10 px-3 py-2 ring-1 ring-violet-200 dark:ring-violet-500/30">
							<div class="min-w-0 flex-1 text-xs">
								<p class="truncate text-violet-500 dark:text-violet-400">{current.customer_name} / {current.project_name}</p>
								<p class="truncate font-medium text-violet-800 dark:text-violet-300">{current.issue_title}</p>
							</div>
							<button class="rounded-md p-1 text-violet-500 dark:text-violet-400 transition hover:bg-violet-100 dark:hover:bg-violet-500/15 hover:text-violet-800 dark:hover:text-violet-300" onclick={unlink} title="Unlink issue">
								<Unlink size={14} />
							</button>
						</div>
					{:else if !linking}
						<button class="btn-secondary-sm mt-1.5 gap-1.5" onclick={startLink}>
							<Link2 size={13} /> Link issue
						</button>
					{/if}
				</div>
			</div>
		</div>

		{#if linking && current.issue_id === null}
			<div class="mx-6 mb-5 space-y-3 rounded-xl bg-slate-50 dark:bg-neutral-800/40 p-4 ring-1 ring-slate-200 dark:ring-neutral-800" transition:fade={{ duration: 120 }}>
				<p class="text-xs text-slate-500 dark:text-neutral-400">Pick an existing issue, or type a new title to create one.</p>
				<div class="grid gap-2 sm:grid-cols-3">
					<Combobox
						items={customers.map((c) => ({ value: c.id, label: c.name }))}
						bind:value={customerId}
						placeholder="Customer…"
						onchange={onCustomerChange}
					/>
					<Combobox
						items={projects.map((p) => ({ value: p.id, label: p.name }))}
						bind:value={projectId}
						placeholder="Project…"
						disabled={!customerId}
						onchange={onProjectChange}
					/>
					<Combobox
						items={issues.map((i) => ({ value: i.title, label: i.title }))}
						bind:value={issueTitle}
						placeholder="Issue…"
						allowNew={true}
						disabled={!projectId}
					/>
				</div>
				<div class="flex justify-end gap-2">
					<button class="btn-secondary-sm" onclick={() => (linking = false)}>Cancel</button>
					<button class="btn-secondary-sm" onclick={link}>Link</button>
				</div>
			</div>
		{/if}

		{#if error}
			<p class="mx-6 mb-4 rounded-lg bg-red-50 dark:bg-red-500/10 px-3 py-2 text-sm text-red-700 dark:text-red-300 ring-1 ring-red-200 dark:ring-red-500/30">{error}</p>
		{/if}

		<div class="flex items-center justify-between gap-2 border-t border-slate-100 dark:border-neutral-800 px-6 py-4">
			{#if confirmDelete}
				<div class="flex items-center gap-2">
					<span class="text-sm text-red-700 dark:text-red-300">Delete this card?</span>
					<button class="btn-danger-sm" onclick={remove}>Delete</button>
					<button class="btn-secondary-sm" onclick={() => (confirmDelete = false)}>Keep</button>
				</div>
			{:else}
				<button class="inline-flex items-center gap-1.5 rounded-lg px-2 py-1 text-sm text-slate-400 dark:text-neutral-500 transition hover:bg-red-50 dark:hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400" onclick={() => (confirmDelete = true)}>
					<Trash2 size={14} /> Delete
				</button>
			{/if}
			<div class="flex gap-2">
				<button class="btn-secondary" onclick={onclose}>Cancel</button>
				<button class="btn-primary" onclick={save} disabled={saving}>Save</button>
			</div>
		</div>
	</div>
</div>
