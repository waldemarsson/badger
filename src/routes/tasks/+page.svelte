<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { onMount, tick, untrack } from 'svelte';
	import { fade, slide } from 'svelte/transition';
	import type { LinkTargets, Task } from '#lib/types.js';
	import {
		SCOPES,
		completedTasks,
		groupTasks,
		inScope,
		moreDone,
		nextFocusId,
		planChips,
		planError,
		planInput,
		planQuickAdd,
		scopeCounts,
		scopePhrase,
		scopeTasks,
		type GroupKey,
		type IssueTarget
	} from '#lib/tasks.js';
	import { parseReminder } from '#lib/time-phrases.js';
	import {
		clock,
		completeTask,
		completing,
		createTask,
		loadAllDone,
		openRequest,
		refreshTasks,
		reopenTask,
		shownTasks,
		taskView,
		tasks,
		toastArea
	} from '#lib/tasks-state.svelte.js';
	import { appleSync } from '#lib/apple-sync-state.svelte.js';
	import { appleSyncPill, formatLastSync } from '#lib/apple-sync.js';
	import { groupLeave, keepRowFocus, motionMs, panelReveal, rowLeave } from '#lib/task-transitions.js';
	import TaskRow from '#lib/components/TaskRow.svelte';
	import TaskPanel from '#lib/components/TaskPanel.svelte';
	import TaskChips from '#lib/components/TaskChips.svelte';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Plus from '@lucide/svelte/icons/plus';

	const now = $derived(new Date(clock.ms));
	const shown = $derived(shownTasks(tasks.list, completing.rows));
	// The scope filters before grouping; Completed shows only under All.
	const scope = $derived(SCOPES.find((s) => s.key === taskView.scope) ?? SCOPES[0]);
	const groups = $derived(groupTasks(scopeTasks(shown, scope.key, now), now));
	const done = $derived(completedTasks(shown, now));
	const doneCount = $derived(done.thisWeek.length + done.older.length);
	// Counts follow the rows: a task mid-completion still counts until it slides out.
	const counts = $derived(scopeCounts(shown, now));

	// Full class names for Tailwind's content scan.
	const DOTS: Record<GroupKey, string> = {
		overdue: 'task-dot-overdue',
		today: 'task-dot-today',
		tomorrow: 'task-dot-tomorrow',
		week: 'task-dot-week',
		later: 'task-dot-later',
		none: 'task-dot-none'
	};

	let selectedId = $state<number | null>(null);
	const selected = $derived(selectedId === null ? null : (tasks.list.find((t) => t.id === selectedId) ?? null));
	let showDone = $state(false);
	let showOlder = $state(false);
	const hasMoreDone = $derived(moreDone(tasks.list, tasks.doneLimit));
	let actionError = $state('');
	let listEl = $state<HTMLElement>();
	let panelEl = $state<HTMLElement>();
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

	/**
	 * Selects a task and, when asked, scrolls it into view: a task outside the scope switches
	 * to All, and Completed unfolds if it's there.
	 */
	function select(id: number, reveal = false): void {
		selectedId = id;
		if (!reveal) return;
		const t = tasks.list.find((x) => x.id === id);
		if (t && !inScope(t, taskView.scope, now)) taskView.scope = 'all';
		if (t?.done) {
			showDone = true;
			if (!done.thisWeek.some((x) => x.id === id)) showOlderDone();
		}
		void tick().then(() => listEl?.querySelector(`[data-task-row="${id}"]`)?.scrollIntoView({ block: 'nearest', behavior: motionMs(1) ? 'smooth' : 'auto' }));
	}

	/** Closes the panel; focus that was in it goes back to the task's row. */
	function closePanel(): void {
		const id = selectedId;
		const hadFocus = !!panelEl?.contains(document.activeElement);
		selectedId = null;
		if (hadFocus) void tick().then(() => listEl?.querySelector<HTMLElement>(`[data-task-row="${id}"] .task-row-button`)?.focus());
	}

	/** Clicking the selected row again closes the panel. */
	function toggleSelect(id: number): void {
		if (id === selectedId) closePanel();
		else select(id);
	}

	/**
	 * Esc closes the panel unless something nearer used it: an open menu, a panel field, the
	 * quick-add draft (they prevent the default), or another text field.
	 */
	function onWindowKeydown(e: KeyboardEvent): void {
		if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing || selectedId === null) return;
		const t = e.target;
		const field = t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement || (t instanceof HTMLElement && t.isContentEditable);
		if (field && t !== quickInput) return;
		e.preventDefault();
		closePanel();
	}

	function showOlderDone(): void {
		showOlder = true;
		void loadAllDone();
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
		menus
		onselect={() => toggleSelect(t.id)}
		ontoggle={() => void toggle(t)}
		ondetails={() => select(t.id)}
		ondelete={() => {
			if (selectedId === t.id) selectedId = null;
		}}
		onerror={(message) => (actionError = message)}
		keepfocus={() => keepRowFocus(listEl, t.id, () => quickInput)}
	/>
{/snippet}

<svelte:window onkeydown={onWindowKeydown} />

<!-- The panel is a column only while a task is selected; otherwise the list spans the width. -->
<div class="lg:flex lg:min-h-[calc(100vh-4.25rem)]">
	<main class="min-w-0 space-y-3.5 px-3 pt-4 sm:px-4 lg:flex-1 lg:px-6" style="padding-bottom: {Math.max(40, toastArea.height + 24)}px">
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
			<div class="task-scope" role="group" aria-label="Show">
				{#each SCOPES as s (s.key)}
					{@const on = taskView.scope === s.key}
					{@const dot = s.key === 'today' && counts.attention > 0}
					<button
						type="button"
						class="task-scope-option"
						class:task-scope-option-active={on}
						aria-pressed={on}
						aria-label="{s.label}, {counts[s.key]}{dot ? `, ${counts.attention} overdue` : ''}"
						onclick={() => (taskView.scope = s.key)}
					>
						{#if dot}
							<span class="task-scope-dot" title="{counts.attention} overdue"></span>
						{/if}
						{s.label}<span class="task-scope-count">{counts[s.key]}</span>
					</button>
				{/each}
			</div>
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

		<div bind:this={listEl}>
			<!-- Switching scope swaps the list at once (keyed), so rows and groups don't slide. -->
			{#key scope.key}
				<div class="space-y-1" in:fade={{ duration: motionMs(120) }}>
					{#if tasks.loaded && tasks.list.length > 0 && groups.length === 0}
						{#if scope.key === 'all'}
							<p class="empty-state" in:fade={{ delay: motionMs(250), duration: motionMs(150) }}>{scope.empty}</p>
						{:else}
							<div class="empty-state" in:fade={{ delay: motionMs(250), duration: motionMs(150) }}>
								<p class="font-medium text-slate-700 dark:text-neutral-200">{scope.empty}</p>
								<p class="mt-1">Quick-add phrases like “{scopePhrase(scope.key, now)}” land here.</p>
							</div>
						{/if}
					{/if}
					{#each groups as g (g.key)}
						<section class="flex flex-col gap-0.5" aria-labelledby="task-group-{g.key}" out:groupLeave>
							<h2 id="task-group-{g.key}" class="task-group-header">
								<span class="task-dot {DOTS[g.key]}"></span>{g.label}<span class="task-group-count">{g.items.length}</span>
							</h2>
							{#each g.items as t (t.id)}
								<div class="task-row-item" out:rowLeave>
									{@render row(t)}
								</div>
							{/each}
						</section>
					{/each}

					{#if scope.key !== 'all'}
						{#if doneCount && groups.length}
							<p class="task-help px-3 pt-3">Completed tasks show under All.</p>
						{/if}
					{:else if doneCount}
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
								<div class="flex flex-col gap-0.5" transition:slide={{ duration: motionMs(200) }}>
									{#each done.thisWeek as t (t.id)}
										<div class="task-row-item" out:rowLeave>{@render row(t)}</div>
									{:else}
										<p class="task-help px-3 py-2">Nothing completed this week yet.</p>
									{/each}
									{#if done.older.length || hasMoreDone}
										{#if showOlder}
											<div class="task-rows-more flex flex-col gap-0.5" transition:slide={{ duration: motionMs(200) }}>
												{#each done.older as t (t.id)}
													<div class="task-row-item" out:rowLeave>{@render row(t)}</div>
												{/each}
											</div>
										{:else}
											<button type="button" class="btn-secondary-sm ml-3 mt-2" onclick={showOlderDone}>Show older ({done.older.length}{hasMoreDone ? '+' : ''})</button>
										{/if}
									{/if}
								</div>
							{/if}
						</section>
					{/if}
				</div>
			{/key}
		</div>
	</main>

	{#if selected}
		{@const s = selected}
		<aside bind:this={panelEl} class="task-panel lg:w-[27.5rem] lg:shrink-0" aria-label="Task details" transition:panelReveal>
			<!-- Bottom padding clears the reminder toasts (bottom right), so the panel's end stays reachable. -->
			<div class="px-6 pt-5 lg:sticky lg:top-0 lg:max-h-screen lg:w-[calc(27.5rem-1px)] lg:overflow-y-auto" style="padding-bottom: {toastArea.height + 32}px">
				<TaskPanel
					task={completing.rows[s.id]?.task ?? s}
					{now}
					notes={targets.notes}
					ticking={completing.rows[s.id]?.phase === 'tick'}
					ontoggle={() => void toggle(s)}
					onclose={closePanel}
				/>
			</div>
		</aside>
	{/if}
</div>
