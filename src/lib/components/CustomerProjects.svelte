<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';
	import { fade, scale } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import type { Customer, Project } from '#lib/types.js';
	import NameInput from '#lib/components/NameInput.svelte';
	import { byName, plural } from '#lib/names.js';
	import Star from '@lucide/svelte/icons/star';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Trash2 from '@lucide/svelte/icons/trash';
	import Plus from '@lucide/svelte/icons/plus';

	let customers = $state<Customer[]>([]);
	let projects = $state<Record<number, Project[]>>({});
	let loaded = $state(false);
	let loadError = $state('');

	// UI keys: `c:<id>` for a customer header, `p:<id>` for a project row.
	let editing = $state<string | null>(null);
	let confirming = $state<string | null>(null);
	let errors = $state<Record<string, string>>({});
	let addingCustomer = $state(false);
	let addingProjectFor = $state<number | null>(null);

	onMount(load);

	async function load() {
		loaded = false;
		try {
			const list = byName(await invoke<Customer[]>('list_customers'));
			const lists = await Promise.all(
				list.map((c) => invoke<Project[]>('list_projects', { customerId: c.id }))
			);
			projects = Object.fromEntries(list.map((c, i) => [c.id, byName(lists[i])]));
			customers = list;
			loadError = '';
		} catch (e) {
			loadError = String(e);
		} finally {
			loaded = true;
		}
	}

	// Resets per-row UI state before starting a new interaction on `key`.
	function begin(key: string) {
		delete errors[key];
		editing = null;
		confirming = null;
		addingProjectFor = null;
	}

	function startEdit(key: string) {
		begin(key);
		editing = key;
	}

	function askDelete(key: string) {
		begin(key);
		confirming = key;
	}

	// Runs a save from a NameInput. The error is also kept in `errors[key]` so it
	// survives the editor being closed by another action (shown only once it is).
	async function tracked(key: string, fn: () => Promise<void>) {
		try {
			await fn();
			delete errors[key];
		} catch (e) {
			errors[key] = String(e);
			throw e;
		}
	}

	function focusOnMount(node: HTMLElement) {
		node.focus();
	}

	function onConfirmKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') confirming = null;
	}

	// ── Customers ───────────────────────────────────────────────────────────
	async function createCustomer(name: string) {
		const c = await invoke<Customer>('create_customer', { name });
		projects[c.id] = [];
		customers = byName([...customers, c]);
		addingCustomer = false;
	}

	const renameCustomer = (id: number, name: string) =>
		tracked(`c:${id}`, async () => {
			const updated = await invoke<Customer>('update_customer', { id, name });
			customers = byName(customers.map((c) => (c.id === id ? updated : c)));
			if (editing === `c:${id}`) editing = null;
		});

	async function setDefaultCustomer(c: Customer) {
		const key = `c:${c.id}`;
		begin(key);
		const previous = customers;
		customers = customers.map((x) => ({ ...x, use_as_default: x.id === c.id }));
		try {
			await invoke<void>('set_default_customer', { id: c.id });
		} catch (e) {
			customers = previous;
			errors[key] = String(e);
		}
	}

	async function deleteCustomer(c: Customer) {
		const key = `c:${c.id}`;
		confirming = null;
		try {
			await invoke<void>('delete_customer', { id: c.id });
			customers = customers.filter((x) => x.id !== c.id);
			delete projects[c.id];
		} catch (e) {
			errors[key] = String(e);
		}
	}

	// ── Projects ────────────────────────────────────────────────────────────
	function startAddProject(customerId: number) {
		begin(`a:${customerId}`);
		addingProjectFor = customerId;
	}

	const createProject = (customerId: number, name: string) =>
		tracked(`a:${customerId}`, async () => {
			const p = await invoke<Project>('create_project', { customerId, name });
			projects[customerId] = byName([...(projects[customerId] ?? []), p]);
			if (addingProjectFor === customerId) addingProjectFor = null;
		});

	const renameProject = (p: Project, name: string) =>
		tracked(`p:${p.id}`, async () => {
			const updated = await invoke<Project>('update_project', { id: p.id, name });
			projects[p.customer_id] = byName(
				projects[p.customer_id].map((x) => (x.id === p.id ? updated : x))
			);
			if (editing === `p:${p.id}`) editing = null;
		});

	async function setDefaultProject(p: Project) {
		const key = `p:${p.id}`;
		begin(key);
		const previous = projects[p.customer_id];
		projects[p.customer_id] = previous.map((x) => ({ ...x, use_as_default: x.id === p.id }));
		try {
			await invoke<void>('set_default_project', { projectId: p.id });
		} catch (e) {
			projects[p.customer_id] = previous;
			errors[key] = String(e);
		}
	}

	async function deleteProject(p: Project) {
		const key = `p:${p.id}`;
		confirming = null;
		try {
			await invoke<void>('delete_project', { id: p.id });
			projects[p.customer_id] = projects[p.customer_id].filter((x) => x.id !== p.id);
		} catch (e) {
			errors[key] = String(e);
		}
	}
</script>

{#snippet defaultStar(isDefault: boolean, title: string)}
	<span class="flex w-3.5 shrink-0 justify-center text-amber-500 dark:text-amber-400" {title}>
		{#if isDefault}<Star size={14} fill="currentColor" />{/if}
	</span>
{/snippet}

{#snippet actions(
	noun: string,
	name: string,
	isDefault: boolean,
	onDefault: () => void,
	onEdit: () => void,
	onDelete: () => void
)}
	<div class="row-actions">
		{#if !isDefault}
			<button
				class="btn-icon-ghost hover:text-amber-500 dark:hover:text-amber-400"
				title="Set as default {noun}"
				aria-label="Set {name} as default {noun}"
				onclick={onDefault}><Star size={14} /></button
			>
		{/if}
		<button class="btn-icon-ghost" title="Rename" aria-label="Rename {name}" onclick={onEdit}
			><Pencil size={14} /></button
		>
		<button class="btn-icon-ghost-danger" title="Delete" aria-label="Delete {name}" onclick={onDelete}
			><Trash2 size={14} /></button
		>
	</div>
{/snippet}

{#snippet confirmDelete(name: string, onConfirm: () => void)}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="flex shrink-0 items-center gap-1.5" onkeydown={onConfirmKeydown} in:fade={{ duration: 150 }}>
		<span class="text-xs font-medium text-red-700 dark:text-red-300">Delete?</span>
		<button class="btn-danger-sm" aria-label="Confirm delete {name}" use:focusOnMount onclick={onConfirm}
			>Delete</button
		>
		<button class="btn-secondary-sm" onclick={() => (confirming = null)}>Cancel</button>
	</div>
{/snippet}

<div class="space-y-4">
	<div class="flex items-center justify-between gap-3">
		<p class="text-sm text-slate-500 dark:text-neutral-400">
			{loadError ? '' : loaded ? plural(customers.length, 'customer') : 'Loading…'}
		</p>
		<button
			class="btn-primary gap-1.5"
			onclick={() => {
				begin('');
				addingCustomer = true;
			}}
			disabled={addingCustomer}
		>
			<Plus size={16} /> Customer
		</button>
	</div>

	{#if loadError}
		<div class="stack-error flex items-center justify-between gap-3">
			<span>Could not load customers: {loadError}</span>
			<button class="btn-secondary-sm" onclick={load} disabled={!loaded}>Retry</button>
		</div>
	{/if}

	{#if loaded && !loadError && customers.length === 0 && !addingCustomer}
		<div class="empty-state" in:fade={{ duration: 150 }}>
			<p class="font-semibold text-slate-700 dark:text-neutral-300">No customers yet</p>
			<p>Add your first customer to start tracking time.</p>
		</div>
	{/if}

	<div class="grid grid-cols-1 items-start gap-4 lg:grid-cols-2 xl:grid-cols-3">
		{#if addingCustomer}
			<div class="entity-card px-4 py-3" in:scale={{ duration: 150, start: 0.97 }}>
				<NameInput
					label="New customer name"
					placeholder="Customer name…"
					onsave={createCustomer}
					oncancel={() => (addingCustomer = false)}
				/>
			</div>
		{/if}

		{#each customers as c (c.id)}
			{@const key = `c:${c.id}`}
			{@const list = projects[c.id] ?? []}
			<section
				class="entity-card"
				aria-label={c.name}
				animate:flip={{ duration: 200 }}
				in:scale={{ duration: 150, start: 0.97 }}
				out:fade={{ duration: 150 }}
			>
				<header
					class="group flex items-center gap-2 rounded-t-2xl border-b border-slate-100 dark:border-neutral-800 px-4 py-3 transition {confirming ===
					key
						? 'bg-red-50 dark:bg-red-500/10'
						: ''}"
				>
					{@render defaultStar(c.use_as_default, 'Default customer')}
					{#if editing === key}
						<NameInput
							initial={c.name}
							label="Customer name"
							onsave={(name) => renameCustomer(c.id, name)}
							oncancel={() => {
								delete errors[key];
								editing = null;
							}}
						/>
					{:else}
						<div class="min-w-0 flex-1">
							<h3
								class="truncate font-semibold text-slate-900 dark:text-neutral-100"
								title="Double-click to rename"
								ondblclick={() => startEdit(key)}
							>
								{c.name}
							</h3>
							<p class="text-xs text-slate-500 dark:text-neutral-400">{plural(list.length, 'project')}</p>
						</div>
						{#if confirming === key}
							{@render confirmDelete(c.name, () => deleteCustomer(c))}
						{:else}
							{@render actions(
								'customer',
								c.name,
								c.use_as_default,
								() => setDefaultCustomer(c),
								() => startEdit(key),
								() => askDelete(key)
							)}
						{/if}
					{/if}
				</header>
				{#if errors[key] && editing !== key}
					<p class="form-error px-4 pt-2" transition:fade={{ duration: 150 }}>{errors[key]}</p>
				{/if}

				<ul class="flex flex-col gap-0.5 p-2">
					{#each list as p (p.id)}
						{@const pkey = `p:${p.id}`}
						<li animate:flip={{ duration: 200 }} transition:fade={{ duration: 150 }}>
							<div class="group entity-row {confirming === pkey ? 'bg-red-50 dark:bg-red-500/10 hover:bg-red-50 dark:hover:bg-red-500/10' : ''}">
								{@render defaultStar(p.use_as_default, 'Default project')}
								{#if editing === pkey}
									<NameInput
										initial={p.name}
										label="Project name"
										onsave={(name) => renameProject(p, name)}
										oncancel={() => {
											delete errors[pkey];
											editing = null;
										}}
									/>
								{:else}
									<!-- svelte-ignore a11y_no_static_element_interactions -->
									<span
										class="min-w-0 flex-1 truncate text-sm text-slate-700 dark:text-neutral-300"
										title="Double-click to rename"
										ondblclick={() => startEdit(pkey)}>{p.name}</span
									>
									{#if confirming === pkey}
										{@render confirmDelete(p.name, () => deleteProject(p))}
									{:else}
										{@render actions(
											'project',
											p.name,
											p.use_as_default,
											() => setDefaultProject(p),
											() => startEdit(pkey),
											() => askDelete(pkey)
										)}
									{/if}
								{/if}
							</div>
							{#if errors[pkey] && editing !== pkey}
								<p class="form-error px-2 pb-1 pl-8" transition:fade={{ duration: 150 }}>
									{errors[pkey]}
								</p>
							{/if}
						</li>
					{/each}

					{#if list.length === 0 && addingProjectFor !== c.id}
						<li class="px-2 py-1.5 pl-8 text-sm text-slate-400 dark:text-neutral-500">No projects yet</li>
					{/if}

					<li>
						{#if addingProjectFor === c.id}
							<div class="entity-row">
								{@render defaultStar(false, '')}
								<NameInput
									label="New project name for {c.name}"
									placeholder="Project name…"
									onsave={(name) => createProject(c.id, name)}
									oncancel={() => {
										delete errors[`a:${c.id}`];
										addingProjectFor = null;
									}}
								/>
							</div>
						{:else}
							<button
								class="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-sm text-slate-500 dark:text-neutral-400 transition hover:bg-slate-50 dark:hover:bg-neutral-800 hover:text-slate-900 dark:hover:text-neutral-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-slate-300 dark:focus-visible:ring-neutral-600"
								onclick={() => startAddProject(c.id)}
							>
								<span class="flex w-3.5 shrink-0 justify-center"><Plus size={14} /></span>
								Add project
							</button>
						{/if}
						{#if errors[`a:${c.id}`] && addingProjectFor !== c.id}
							<p class="form-error px-2 pb-1 pl-8" transition:fade={{ duration: 150 }}>
								{errors[`a:${c.id}`]}
							</p>
						{/if}
					</li>
				</ul>
			</section>
		{/each}
	</div>
</div>
