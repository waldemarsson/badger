<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { onMount, tick, untrack } from 'svelte';
	import { fade, slide } from 'svelte/transition';
	import type { LinkTargets, Task } from '#lib/types.js';
	import {
		completedTasks,
		dueTodayCount,
		groupTasks,
		nextFocusId,
		overdueCount,
		planChips,
		planError,
		planInput,
		planQuickAdd,
		type GroupKey,
		type IssueTarget
	} from '#lib/tasks.js';
	import { parseReminder } from '#lib/time-phrases.js';
	import {
		clock,
		completeTask,
		completing,
		createTask,
		openRequest,
		refreshTasks,
		reopenTask,
		shownTasks,
		tasks,
		toastArea
	} from '#lib/tasks-state.svelte.js';
	import { appleSync } from '#lib/apple-sync-state.svelte.js';
	import { appleSyncPill, formatLastSync } from '#lib/apple-sync.js';
	import { groupLeave, motionMs, rowLeave } from '#lib/task-transitions.js';
	import TaskRow from '#lib/components/TaskRow.svelte';
	import TaskPanel from '#lib/components/TaskPanel.svelte';
	import TaskChips from '#lib/components/TaskChips.svelte';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Plus from '@lucide/svelte/icons/plus';

	const now = $derived(new Date(clock.ms));
	const shown = $derived(shownTasks(tasks.list, completing.rows));
	const groups = $derived(groupTasks(shown, now));
	const done = $derived(completedTasks(shown, now));
	// Totals follow the rows: a task mid-completion still counts until it slides out.
	const open = $derived(shown.filter((t) => !t.done));
	const overdue = $derived(overdueCount(shown, now));
	const dueToday = $derived(dueTodayCount(shown, now));

	// Full class names for Tailwind's content scan.
	const DOTS: Record<GroupKey, string> = {
		overdue: 'task-dot-overdue',
		today: 'task-dot-today',
		upcoming: 'task-dot-upcoming',
		none: 'task-dot-none'
	};

	let selectedId = $state<number | null>(null);
	const selected = $derived(selectedId === null ? null : (tasks.list.find((t) => t.id === selectedId) ?? null));
	let showDone = $state(false);
	let showOlder = $state(false);
	let actionError = $state('');
	let listEl = $state<HTMLElement>();
	let quickInput = $state<HTMLInputElement>();

	// ── Quick add ──────────────────────────────────────────────────────────────

	let draft = $state('');
	let quickError = $state('');
	let adding = $state(false);
	let targets = $state<{ issues: IssueTarget[]; notes: { id: number; title: string }[] }>({ issues: [], notes: [] });
	const plan = $derived(draft.trim() ? planQuickAdd(draft, now, targets.issues, parseReminder) : null);
	const preview = $derived(plan ? planChips(plan, now) : []);

	async function loadTargets(): Promise<void> {
		try {
			const t = await invoke<LinkTargets>('get_link_targets');
			targets = { issues: t.issues, notes: t.notes };
		} catch {
			// #tags just don't match; the panel's note picker stays empty.
		}
	}

	async function addDraft(): Promise<void> {
		if (adding) return;
		const p = planQuickAdd(draft, new Date(), targets.issues, parseReminder);
		quickError = planError(p);
		if (quickError) return;
		adding = true;
		try {
			const { input, reminders } = planInput(p);
			const t = await createTask(input, reminders);
			draft = '';
			select(t.id, true);
		} catch (e) {
			quickError = String(e);
		} finally {
			adding = false;
		}
	}

	function onDraftKeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		if (e.key === 'Enter') {
			e.preventDefault();
			void addDraft();
		} else if (e.key === 'Escape' && draft) {
			e.preventDefault();
			draft = '';
			quickError = '';
		}
	}

	// ── Selection ──────────────────────────────────────────────────────────────

	/** Selects a task and, when asked, scrolls it into view (unfolding Completed if it's there). */
	function select(id: number, reveal = false): void {
		selectedId = id;
		if (!reveal) return;
		const t = tasks.list.find((x) => x.id === id);
		if (t?.done) {
			showDone = true;
			if (!done.thisWeek.some((x) => x.id === id)) showOlder = true;
		}
		void tick().then(() => listEl?.querySelector(`[data-task-row="${id}"]`)?.scrollIntoView({ block: 'nearest', behavior: motionMs(1) ? 'smooth' : 'auto' }));
	}

	// Notification clicks and ⌘K ask through `openRequest`; links use `?id=` (and `?new=1`).
	$effect(() => {
		if (!openRequest.pending || !tasks.loaded) return;
		openRequest.pending = false;
		const id = openRequest.id;
		if (id !== null) untrack(() => select(id, true));
	});

	$effect(() => {
		const id = Number(page.url.searchParams.get('id'));
		const isNew = page.url.searchParams.has('new');
		if (!tasks.loaded || (!id && !isNew)) return;
		if (id) untrack(() => select(id, true));
		if (isNew) void tick().then(() => quickInput?.focus());
		void goto(resolve('/tasks'), { replace: true, reset: false });
	});

	// ── Completing ─────────────────────────────────────────────────────────────

	/** Visible open rows in order, without the ones already leaving. */
	function openOrder(): number[] {
		return groups.flatMap((g) => g.items.map((t) => t.id));
	}

	/** Focus was on the completed row (or the panel's check): move it to the next row's check. */
	function moveFocus(id: number, order: number[]): void {
		const active = document.activeElement;
		const row = listEl?.querySelector(`[data-task-row="${id}"]`);
		const fromRow = !!row && row.contains(active);
		if (!fromRow) return;
		const next = nextFocusId(order, id);
		const target = next === null ? quickInput : listEl?.querySelector<HTMLElement>(`[data-task-row="${next}"] [data-task-check]`);
		target?.focus();
	}

	async function toggle(t: Task): Promise<void> {
		actionError = '';
		if (t.done) {
			try {
				await reopenTask(t.id);
			} catch (e) {
				actionError = String(e);
			}
			return;
		}
		const order = openOrder();
		actionError = await completeTask(t, () => moveFocus(t.id, order));
	}

	// The Completed count bumps (450 ms) after each completion made while this page is open.
	let bumpSeen = completing.bump;
	let bumping = $state(false);
	$effect(() => {
		if (completing.bump === bumpSeen) return;
		bumpSeen = completing.bump;
		bumping = true;
		const timer = setTimeout(() => (bumping = false), 450);
		return () => clearTimeout(timer);
	});

	// ── Sync status beside the totals ──────────────────────────────────────────

	const sync = $derived.by(() => {
		const status = appleSync.status;
		if (!status || !appleSync.on) return null;
		const pill = appleSyncPill(status);
		if (pill.tone === 'ok') return { dot: 'bg-emerald-500 dark:bg-emerald-400', text: `Synced with Apple Reminders · ${formatLastSync(status.last_sync_at, now).toLowerCase()}` };
		if (pill.tone === 'error') return { dot: 'bg-red-500 dark:bg-red-400', text: 'Apple Reminders sync failed — see Settings' };
		if (pill.tone === 'warn') return { dot: 'bg-amber-500 dark:bg-amber-400', text: `Apple Reminders sync: ${pill.label.toLowerCase()} — see Settings` };
		return { dot: 'bg-slate-400 dark:bg-neutral-500', text: 'Syncing with Apple Reminders…' };
	});

	onMount(() => {
		void loadTargets();
		// The layout keeps the list current from `tasks-changed` (deleted or renamed projects
		// and customers emit it too); this is just a cheap safety re-read on arrival.
		void refreshTasks();
	});
</script>

{#snippet row(t: Task)}
	<TaskRow
		task={t}
		{now}
		selected={t.id === selectedId}
		ticking={completing.rows[t.id]?.phase === 'tick'}
		syncOn={appleSync.on}
		onselect={() => select(t.id)}
		ontoggle={() => void toggle(t)}
	/>
{/snippet}

<div class="lg:grid lg:min-h-[calc(100vh-4.25rem)] lg:grid-cols-[minmax(0,1fr)_minmax(22rem,27.5rem)]">
	<main class="min-w-0 space-y-3.5 px-3 pt-4 sm:px-4 lg:px-6" style="padding-bottom: {Math.max(40, toastArea.height + 24)}px">
		<h1 class="sr-only">Tasks</h1>

		<div class="space-y-2">
			<label for="task-quick-add" class="sr-only">New task</label>
			<div class="task-quickadd">
				<Plus size={16} class="shrink-0 text-slate-500 dark:text-neutral-400" aria-hidden="true" />
				<input
					id="task-quick-add"
					bind:this={quickInput}
					class="task-quickadd-input"
					placeholder="Add a task — try “Call Anna fri 14:00 #Contoso”"
					autocomplete="off"
					maxlength={600}
					bind:value={draft}
					oninput={() => (quickError = '')}
					onkeydown={onDraftKeydown}
					aria-describedby="task-quick-preview"
				/>
				<kbd class="task-kbd">⏎</kbd>
			</div>
			<div id="task-quick-preview" aria-live="polite">
				{#if plan}
					<div class="flex flex-wrap items-center gap-1.5 pl-3.5 text-xs text-slate-500 dark:text-neutral-400" transition:fade={{ duration: 150 }}>
						<span>Adds</span>
						<span class="max-w-[24rem] truncate font-semibold text-slate-700 dark:text-neutral-200">{plan.title ? `“${plan.title}”` : '…'}</span>
						<TaskChips chips={preview} />
						<span class="ml-auto">Enter adds · Esc clears</span>
					</div>
				{/if}
				{#if quickError}
					<p class="form-error mt-1 pl-3.5 text-xs" role="alert">{quickError}</p>
				{/if}
			</div>
		</div>

		<div class="task-toolbar">
			<span class="toolbar-stat text-sm"><span class="toolbar-stat-label">Open</span><span class="font-medium tabular-nums text-slate-700 dark:text-neutral-200">{open.length}</span></span>
			<span class="toolbar-stat text-sm"><span class="toolbar-stat-label">Overdue</span><span class="font-medium tabular-nums text-red-700 dark:text-red-300">{overdue}</span></span>
			<span class="toolbar-stat text-sm"><span class="toolbar-stat-label">Due today</span><span class="font-medium tabular-nums text-amber-700 dark:text-amber-300">{dueToday}</span></span>
			{#if sync}
				<a href={resolve('/settings')} class="ml-auto inline-flex items-center gap-1.5 rounded-md text-xs text-slate-500 transition hover:text-slate-800 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 dark:text-neutral-400 dark:hover:text-neutral-200">
					<span class="h-2 w-2 rounded-full {sync.dot}"></span>{sync.text}
				</a>
			{/if}
		</div>

		{#if actionError}
			<p class="stack-error" role="alert" transition:fade={{ duration: 150 }}>{actionError}</p>
		{/if}

		{#if tasks.error && !tasks.loaded}
			<div class="alert-panel alert-error">
				<p class="alert-error-title">Could not load tasks</p>
				<p class="alert-error-copy">{tasks.error}</p>
			</div>
		{:else if tasks.error}
			<p class="stack-error" role="alert" transition:fade={{ duration: 150 }}>Could not refresh tasks: {tasks.error}</p>
		{/if}

		{#if tasks.loaded && tasks.list.length === 0}
			<div class="empty-state" in:fade={{ duration: 150 }}>
				<p class="font-medium text-slate-700 dark:text-neutral-200">No tasks</p>
				<p class="mt-1">Type one above. Add a day or a time (“tomorrow 9”) and it gets a due date and a reminder.</p>
			</div>
		{/if}

		<div bind:this={listEl} class="space-y-1">
			{#if tasks.loaded && tasks.list.length > 0 && groups.length === 0}
				<p class="empty-state" in:fade={{ delay: 250, duration: 150 }}>Nothing open. Nice.</p>
			{/if}
			{#each groups as g (g.key)}
				<section class="flex flex-col gap-0.5" aria-labelledby="task-group-{g.key}" out:groupLeave>
					<h2 id="task-group-{g.key}" class="task-group-header">
						<span class="task-dot {DOTS[g.key]}"></span>{g.label}<span class="task-group-count">{g.items.length}</span>
					</h2>
					{#each g.items as t (t.id)}
						<div out:rowLeave>
							{@render row(t)}
						</div>
					{/each}
				</section>
			{/each}

			{#if done.thisWeek.length || done.older.length}
				<section class="flex flex-col gap-0.5">
					<h2 class="m-0">
						<button type="button" class="task-group-header" aria-expanded={showDone} onclick={() => (showDone = !showDone)}>
							{#if showDone}<ChevronDown size={14} aria-hidden="true" />{:else}<ChevronRight size={14} aria-hidden="true" />{/if}
							<span class="task-dot task-dot-done"></span>Completed this week
							{#key completing.bump}
								<span class="task-group-count" class:task-group-count-bump={bumping}>{done.thisWeek.length}</span>
							{/key}
						</button>
					</h2>
					{#if showDone}
						<div transition:slide={{ duration: motionMs(200) }}>
							{#each done.thisWeek as t (t.id)}
								<div out:rowLeave>{@render row(t)}</div>
							{:else}
								<p class="task-help px-3 py-2">Nothing completed this week yet.</p>
							{/each}
							{#if done.older.length}
								{#if showOlder}
									<div transition:slide={{ duration: motionMs(200) }}>
										{#each done.older as t (t.id)}
											<div out:rowLeave>{@render row(t)}</div>
										{/each}
									</div>
								{:else}
									<button type="button" class="btn-secondary-sm ml-3 mt-2" onclick={() => (showOlder = true)}>Show older ({done.older.length})</button>
								{/if}
							{/if}
						</div>
					{/if}
				</section>
			{/if}
		</div>
	</main>

	<aside class="task-panel" aria-label="Task details">
		<!-- Bottom padding clears the reminder toasts (bottom right), so the panel's end stays reachable. -->
		<div class="px-6 pt-5 lg:sticky lg:top-0 lg:max-h-screen lg:overflow-y-auto" style="padding-bottom: {toastArea.height + 32}px">
			{#if selected}
				{@const s = selected}
				<TaskPanel
					task={completing.rows[s.id]?.task ?? s}
					{now}
					notes={targets.notes}
					ticking={completing.rows[s.id]?.phase === 'tick'}
					ontoggle={() => void toggle(s)}
					onclose={() => (selectedId = null)}
				/>
			{:else}
				<div class="empty-state mt-10">Select a task to edit it</div>
			{/if}
		</div>
	</aside>
</div>
