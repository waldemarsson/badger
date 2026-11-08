<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { today } from '#lib/date-ranges.js';
	import { formatShortcut } from '#lib/palette.js';
	import { changes, palette, requestLogTime } from '#lib/palette-state.svelte.js';
	import {
		dailyTotals,
		formatDayLabel,
		formatHours,
		inProgress,
		isWeekend,
		needsAttention,
		progress,
		sumHours,
		todayEntries,
		weekDays
	} from '#lib/today.js';
	import { formatDue } from '#lib/todos.js';
	import type { TimeEntryRow, TodoRow } from '#lib/types.js';
	import TodoModal from '#lib/components/TodoModal.svelte';
	import Check from '@lucide/svelte/icons/check';
	import Clock from '@lucide/svelte/icons/clock';
	import Link2 from '@lucide/svelte/icons/link-2';
	import Plus from '@lucide/svelte/icons/plus';
	import CalendarDays from '@lucide/svelte/icons/calendar-days';
	import X from '@lucide/svelte/icons/x';

	const WORKDAYS = 5;

	let todayIso = $state(today());
	let entries = $state<TimeEntryRow[]>([]);
	let todos = $state<TodoRow[]>([]);
	let target = $state(8);
	let loaded = $state(false); // true once a load has succeeded; hero stats and sections wait for it
	let loadError = $state('');
	let loadSeq = 0;
	let actionError = $state('');
	let completing = $state<number[]>([]);
	let selected = $state<TodoRow | null>(null);

	/** Loads the week, todos and target. Overlapping calls are fine: only the latest applies. */
	async function load(): Promise<void> {
		const seq = ++loadSeq;
		const now = today();
		const days = weekDays(now);
		try {
			const [weekEntries, allTodos, dailyTarget] = await Promise.all([
				invoke<TimeEntryRow[]>('list_time_entries', { startDate: days[0], endDate: days[6] }),
				invoke<TodoRow[]>('list_todos'),
				invoke<number>('get_daily_target')
			]);
			if (seq !== loadSeq) return;
			todayIso = now;
			entries = weekEntries;
			todos = allTodos;
			target = dailyTarget;
			loadError = '';
			loaded = true;
		} catch (e) {
			if (seq !== loadSeq) return;
			loadError = String(e);
		}
	}

	/** The page may stay open past midnight: reload when it comes back on a new day. */
	function reloadIfNewDay(): void {
		if (document.visibilityState === 'visible' && today() !== todayIso) void load();
	}

	// ── Derived view ──────────────────────────────────────────────────────────────

	const days = $derived(dailyTotals(entries, weekDays(todayIso)));
	const todayHours = $derived(days.find((d) => d.date === todayIso)?.hours ?? 0);
	const weekHours = $derived(sumHours(entries));
	const weekTarget = $derived(target * WORKDAYS);
	const maxDayHours = $derived(Math.max(0, ...days.map((d) => d.hours)));

	const attention = $derived(needsAttention(todos, todayIso));
	const active = $derived(inProgress(todos, todayIso));
	const logged = $derived(todayEntries(entries, todayIso));
	const loggedHours = $derived(sumHours(logged));

	const dateHeading = $derived(formatDayLabel(todayIso));

	function weekdayShort(ymd: string): string {
		const [y, m, d] = ymd.split('-').map(Number);
		return new Date(y, m - 1, d).toLocaleDateString(undefined, { weekday: 'short' });
	}

	/** Bar height in % — against the daily target, or the busiest day when the target is off. */
	function barHeight(hours: number): number {
		const fraction = target > 0 ? progress(hours, target) : maxDayHours > 0 ? hours / maxDayHours : 0;
		return hours > 0 ? Math.max(6, fraction * 100) : 0;
	}

	function metTarget(date: string, hours: number): boolean {
		return target > 0 && !isWeekend(date) && hours >= target;
	}

	// ── Todo actions ──────────────────────────────────────────────────────────────

	async function complete(todo: TodoRow): Promise<void> {
		if (completing.includes(todo.id)) return;
		completing = [...completing, todo.id];
		actionError = '';
		try {
			await invoke('move_todo', { id: todo.id, status: 'done', index: 0 });
			todos = todos.filter((t) => t.id !== todo.id);
			void load();
		} catch (e) {
			actionError = String(e);
		} finally {
			completing = completing.filter((id) => id !== todo.id);
		}
	}

	function logTime(todo: TodoRow): void {
		if (todo.project_id !== null && todo.issue_title) {
			requestLogTime({ projectId: todo.project_id, issueId: todo.issue_id, issueTitle: todo.issue_title });
		} else {
			requestLogTime();
		}
	}

	function onRowKeydown(e: KeyboardEvent, todo: TodoRow): void {
		if (e.target !== e.currentTarget) return; // keys on the row's own buttons
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			selected = todo;
		}
	}

	const dueClasses = {
		overdue: 'bg-red-50 text-red-700 ring-red-200 dark:bg-red-500/10 dark:text-red-300 dark:ring-red-500/30',
		today: 'bg-amber-50 text-amber-700 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-300 dark:ring-amber-500/30',
		upcoming: 'bg-slate-50 text-slate-600 ring-slate-200 dark:bg-neutral-800/40 dark:text-neutral-400 dark:ring-neutral-700'
	};

	function dueKind(due: string): keyof typeof dueClasses {
		return due < todayIso ? 'overdue' : due === todayIso ? 'today' : 'upcoming';
	}

	onMount(() => {
		void load();
	});

	// Reload when the command palette logs time or adds a todo.
	let seenChanges = `${changes.timeEntries}:${changes.todos}`;
	$effect(() => {
		const current = `${changes.timeEntries}:${changes.todos}`;
		if (current === seenChanges) return;
		seenChanges = current;
		void load();
	});
</script>

{#snippet progressBar(hours: number, goal: number)}
	{@const fraction = progress(hours, goal)}
	<div class="today-bar-track mt-2" role="progressbar" aria-valuemin={0} aria-valuemax={goal} aria-valuenow={hours}>
		<div
			class="today-bar-fill {fraction >= 1 ? 'bg-emerald-500 dark:bg-emerald-400' : 'bg-sky-500 dark:bg-sky-400'}"
			style="width: {fraction * 100}%"
		></div>
	</div>
{/snippet}

{#snippet hoursStat(label: string, hours: number, goal: number)}
	<div>
		<div class="flex items-baseline justify-between gap-2">
			<span class="text-sm font-medium text-slate-500 dark:text-neutral-400">{label}</span>
			<span class="text-base font-semibold tabular-nums text-slate-900 dark:text-neutral-100">
				{formatHours(hours)}{#if goal > 0}<span class="font-normal text-slate-400 dark:text-neutral-500">{` / ${formatHours(goal)}`}</span>{/if}
				<span class="font-normal text-slate-400 dark:text-neutral-500">h</span>
			</span>
		</div>
		{#if goal > 0}
			{@render progressBar(hours, goal)}
		{/if}
	</div>
{/snippet}

{#snippet todoRow(t: TodoRow)}
	<div
		class="today-row group cursor-pointer"
		role="button"
		tabindex="0"
		aria-label="Open {t.title}"
		onclick={() => (selected = t)}
		onkeydown={(e) => onRowKeydown(e, t)}
	>
		<span
			class="h-2 w-2 shrink-0 rounded-full {t.status === 'in_progress' ? 'bg-sky-500 dark:bg-sky-400' : 'bg-slate-300 dark:bg-neutral-600'}"
			title={t.status === 'in_progress' ? 'In progress' : 'Not started'}
		></span>
		<div class="min-w-0 flex-1">
			<p class="truncate text-sm font-medium text-slate-800 dark:text-neutral-200" title={t.title}>{t.title}</p>
			{#if t.due_date || t.issue_title}
				<div class="mt-1 flex min-w-0 items-center gap-1.5">
					{#if t.due_date}
						{@const kind = dueKind(t.due_date)}
						<span class="kanban-badge shrink-0 {dueClasses[kind]}">
							<CalendarDays size={12} class="shrink-0" />
							{kind === 'today' ? 'Today' : kind === 'overdue' ? `Overdue · ${formatDue(t.due_date)}` : formatDue(t.due_date)}
						</span>
					{/if}
					{#if t.issue_title}
						<span
							class="kanban-badge min-w-0 bg-violet-50 text-violet-700 ring-violet-200 dark:bg-violet-500/10 dark:text-violet-300 dark:ring-violet-500/30"
							title="{t.customer_name} / {t.project_name} / {t.issue_title}"
						>
							<Link2 size={12} class="shrink-0" />
							<span class="truncate">{t.project_name} · {t.issue_title}</span>
						</span>
					{/if}
				</div>
			{/if}
		</div>
		<div class="flex shrink-0 items-center gap-1">
			<button
				type="button"
				class="btn-icon-ghost"
				title={t.issue_title ? `Log time on ${t.issue_title}` : 'Log time'}
				aria-label="Log time for {t.title}"
				onclick={(e) => {
					e.stopPropagation();
					logTime(t);
				}}
			>
				<Clock size={16} />
			</button>
			<button
				type="button"
				class="inline-flex items-center justify-center rounded-md p-1.5 text-slate-400 transition hover:bg-emerald-50 hover:text-emerald-600 focus:outline-none focus-visible:ring-2 focus-visible:ring-emerald-300 disabled:opacity-50 dark:text-neutral-500 dark:hover:bg-emerald-500/10 dark:hover:text-emerald-400 dark:focus-visible:ring-emerald-500/40"
				title="Mark done"
				aria-label="Mark {t.title} done"
				disabled={completing.includes(t.id)}
				onclick={(e) => {
					e.stopPropagation();
					void complete(t);
				}}
			>
				<Check size={16} />
			</button>
		</div>
	</div>
{/snippet}

{#snippet todoSection(title: string, dot: string, items: TodoRow[], emptyText: string)}
	<section class="today-section">
		<header class="today-section-header">
			<span class="h-2.5 w-2.5 rounded-full {dot}"></span>
			<h2 class="today-section-title">{title}</h2>
			{#if items.length}
				<span class="today-count ml-auto">{items.length}</span>
			{/if}
		</header>
		<!-- The list stays mounted so the last row's out:fade plays before the empty line fades in. -->
		<div class="divide-y divide-slate-100 dark:divide-neutral-800">
			{#each items as t (t.id)}
				<div animate:flip={{ duration: 200 }} out:fade={{ duration: 150 }}>
					{@render todoRow(t)}
				</div>
			{/each}
		</div>
		{#if items.length === 0}
			<p class="today-empty" in:fade={{ delay: 150, duration: 150 }}>{emptyText}</p>
		{/if}
	</section>
{/snippet}

<div class="page-shell">
	<div class="page-view">
		<h1 class="sr-only">Today</h1>

		{#if loadError}
			<div class="alert-panel alert-error">
				<p class="alert-error-title">Could not load today</p>
				<p class="alert-error-copy">{loadError}</p>
			</div>
		{/if}

		{#if loaded}
			<div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_24rem]">
				<div class="space-y-4">
					{#if actionError}
						<div
							class="flex items-center justify-between gap-3 rounded-xl bg-red-50 px-4 py-2 text-sm text-red-700 ring-1 ring-red-200 dark:bg-red-500/10 dark:text-red-300 dark:ring-red-500/30"
							role="alert"
							transition:fade={{ duration: 150 }}
						>
							{actionError}
							<button type="button" onclick={() => (actionError = '')} title="Dismiss" aria-label="Dismiss"><X size={14} /></button>
						</div>
					{/if}
					{@render todoSection('Needs attention', 'bg-red-500 dark:bg-red-400', attention, 'Nothing overdue or due today')}
					{@render todoSection('In progress', 'bg-sky-500 dark:bg-sky-400', active, 'Nothing in progress')}
				</div>

				<aside class="today-section">
					<header class="today-section-header">
						<span class="h-2.5 w-2.5 rounded-full bg-slate-400 dark:bg-neutral-500"></span>
						<h2 class="today-section-title">Hours <span class="font-normal normal-case tracking-normal text-slate-500 dark:text-neutral-400">· {dateHeading}</span></h2>
					</header>
					<div class="space-y-4 px-1 py-4">
						{@render hoursStat('Today', todayHours, target)}
						{@render hoursStat('This week', weekHours, weekTarget)}
						<div class="grid grid-cols-7 gap-1" aria-label="This week">
							{#each days as day (day.date)}
								{@const met = metTarget(day.date, day.hours)}
								{@const weekend = isWeekend(day.date)}
								<a
									href="{resolve('/timesheet')}?day={day.date}"
									class="today-day {day.date === todayIso ? 'today-day-today' : ''}"
									title="{day.date} — open in Timesheet"
								>
									<span
										class="text-xs font-semibold uppercase {day.date === todayIso
											? 'text-sky-600 dark:text-sky-400'
											: weekend
												? 'text-slate-400 dark:text-neutral-500'
												: 'text-slate-500 dark:text-neutral-400'}"
									>{weekdayShort(day.date)}</span>
									<div class="flex h-16 w-2.5 items-end overflow-hidden rounded-full bg-slate-100 dark:bg-neutral-800">
										<div
											class="w-full rounded-full transition-[height] duration-200 {met
												? 'bg-emerald-500 dark:bg-emerald-400'
												: weekend
													? 'bg-slate-300 dark:bg-neutral-600'
													: 'bg-slate-400 dark:bg-neutral-400'}"
											style="height: {barHeight(day.hours)}%"
										></div>
									</div>
									<span
										class="text-sm font-semibold tabular-nums {met
											? 'text-emerald-600 dark:text-emerald-400'
											: day.hours > 0
												? 'text-slate-900 dark:text-neutral-100'
												: 'text-slate-400 dark:text-neutral-500'}"
									>{day.hours > 0 ? formatHours(day.hours) : '—'}</span>
								</a>
							{/each}
						</div>
					</div>

					<header class="today-section-header border-t">
						<span class="h-2.5 w-2.5 rounded-full bg-emerald-500 dark:bg-emerald-400"></span>
						<h2 class="today-section-title">Logged today</h2>
						<span class="ml-auto text-sm font-semibold tabular-nums text-slate-900 dark:text-neutral-100">{formatHours(loggedHours)} h</span>
					</header>
					{#if logged.length === 0}
						<p class="today-empty">Nothing logged yet today</p>
					{:else}
						<div class="divide-y divide-slate-100 dark:divide-neutral-800">
							{#each logged as entry (entry.id)}
								<a href="{resolve('/timesheet')}?day={todayIso}" class="today-row" title="{entry.customer_name} / {entry.project_name} · {entry.issue_title} — open in Timesheet">
									<div class="min-w-0 flex-1">
										<p class="truncate text-sm font-medium text-slate-800 dark:text-neutral-200">{entry.issue_title}</p>
										<p class="truncate text-xs text-slate-500 dark:text-neutral-400">{entry.customer_name} / {entry.project_name}</p>
									</div>
									<span class="shrink-0 text-sm font-semibold tabular-nums text-slate-900 dark:text-neutral-100">{formatHours(entry.hours)} h</span>
								</a>
							{/each}
						</div>
					{/if}
					<button
						type="button"
						class="flex w-full items-center gap-2 border-t border-slate-100 px-1 py-2.5 text-sm text-slate-500 transition hover:bg-slate-50 hover:text-slate-800 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-sky-400 dark:border-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200"
						onclick={() => requestLogTime()}
					>
						<Plus size={16} />
						Log time
						<kbd class="palette-kbd ml-auto">{formatShortcut(palette.shortcut)}</kbd>
					</button>
				</aside>
			</div>
		{/if}
	</div>
</div>

<svelte:window onfocus={reloadIfNewDay} />
<svelte:document onvisibilitychange={reloadIfNewDay} />

{#if selected}
	<TodoModal todo={selected} onclose={() => (selected = null)} onchange={load} />
{/if}
