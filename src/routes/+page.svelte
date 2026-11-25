<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { fade } from 'svelte/transition';
	import { today } from '#lib/date-ranges.js';
	import { formatShortcut } from '#lib/palette.js';
	import { changes, palette, requestLogTime } from '#lib/palette-state.svelte.js';
	import {
		dailyTotals,
		formatDayLabel,
		formatHours,
		isWeekend,
		progress,
		sumHours,
		todayEntries,
		weekDays
	} from '#lib/today.js';
	import type { Task, TimeEntryRow } from '#lib/types.js';
	import { todayTasks } from '#lib/tasks.js';
	import { clock, completeTask, completing, shownTasks, tasks } from '#lib/tasks-state.svelte.js';
	import { appleSync } from '#lib/apple-sync-state.svelte.js';
	import { rowLeave } from '#lib/task-transitions.js';
	import TaskRow from '#lib/components/TaskRow.svelte';
	import HandoverIncoming from '#lib/components/HandoverIncoming.svelte';
	import HandoverToday from '#lib/components/HandoverToday.svelte';
	import { HandoverCards } from '#lib/handover-cards.svelte.js';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import ArrowRight from '@lucide/svelte/icons/arrow-right';
	import Clock from '@lucide/svelte/icons/clock';

	const WORKDAYS = 5;

	let todayIso = $state(today());
	/** The handovers' local date: follows the clock even when a load fails. */
	let handoverDay = $state(today());
	/** Both handover cards share one list and save queue; they don't wait for `loaded`. */
	const handovers = new HandoverCards(today());
	$effect(() => handovers.setDay(handoverDay));
	let entries = $state<TimeEntryRow[]>([]);
	let target = $state(8);
	let loaded = $state(false); // true once a load has succeeded; hero stats and sections wait for it
	let settled = $state(false); // true once a load has succeeded or failed (the handover cards wait for it)
	let loadError = $state('');
	let loadSeq = 0;
	let actionError = $state('');

	/** Loads the week and target (tasks come from the shared list). Overlapping calls are fine: only the latest applies. */
	async function load(): Promise<void> {
		const seq = ++loadSeq;
		const now = today();
		const days = weekDays(now);
		try {
			const [weekEntries, dailyTarget] = await Promise.all([
				invoke<TimeEntryRow[]>('list_time_entries', { startDate: days[0], endDate: days[6] }),
				invoke<number>('get_daily_target')
			]);
			if (seq !== loadSeq) return;
			todayIso = now;
			entries = weekEntries;
			target = dailyTarget;
			loadError = '';
			loaded = true;
			settle();
		} catch (e) {
			if (seq !== loadSeq) return;
			loadError = String(e);
			settle();
		}
	}

	/** Shows the handover cards in their place (below the sections) once those rendered or failed. */
	function settle(): void {
		if (settled) return;
		settled = true;
		handovers.pageSettled();
	}

	/** The page may stay open past midnight: reload when it comes back on a new day. */
	function reloadIfNewDay(): void {
		if (document.visibilityState !== 'visible') return;
		const now = today();
		if (now !== handoverDay) handoverDay = now;
		if (now !== todayIso) void load();
	}

	// ── Derived view ──────────────────────────────────────────────────────────────

	const days = $derived(dailyTotals(entries, weekDays(todayIso)));
	const todayHours = $derived(days.find((d) => d.date === todayIso)?.hours ?? 0);
	const weekHours = $derived(sumHours(entries));
	const weekTarget = $derived(target * WORKDAYS);
	const maxDayHours = $derived(Math.max(0, ...days.map((d) => d.hours)));

	const logged = $derived(todayEntries(entries, todayIso));
	const loggedHours = $derived(sumHours(logged));

	const dateHeading = $derived(formatDayLabel(todayIso));

	// Tasks card: overdue (incl. sent reminders) and due today, with the same row and check as /tasks.
	const now = $derived(new Date(clock.ms));
	const dueTasks = $derived(todayTasks(shownTasks(tasks.list, completing.rows), now));

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

	// ── Task actions ──────────────────────────────────────────────────────────────

	async function complete(t: Task): Promise<void> {
		actionError = await completeTask(t);
	}

	/** Opens Log time at the hours step for the task's issue. */
	function logTime(t: Task): void {
		if (t.project_id !== null && t.issue_title) {
			requestLogTime({ projectId: t.project_id, issueId: t.issue_id, issueTitle: t.issue_title });
		}
	}

	function openTask(t: Task): void {
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
		void goto(`${resolve('/tasks')}?id=${t.id}`);
	}

	onMount(() => {
		void load();
	});

	// Reload when the command palette logs time.
	let seenChanges = changes.timeEntries;
	$effect(() => {
		const current = changes.timeEntries;
		if (current === seenChanges) return;
		seenChanges = current;
		void load();
	});
</script>

{#snippet logTimeAction(t: Task)}
	<button type="button" class="btn-icon-ghost" title="Log time on {t.issue_title}" aria-label="Log time for {t.title}" onclick={() => logTime(t)}>
		<Clock size={16} />
	</button>
{/snippet}

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

<div class="page-shell">
	<div class="page-view">
		<h1 class="sr-only">Today</h1>

		{#if loadError}
			<div class="alert-panel alert-error">
				<p class="alert-error-title">Could not load today</p>
				<p class="alert-error-copy">{loadError}</p>
			</div>
		{/if}

		<div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_24rem]">
			<div class="space-y-4">
				{#if loaded}
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
					<section class="today-section" aria-labelledby="today-tasks">
						<header class="today-section-header">
							<span class="h-2.5 w-2.5 rounded-full {dueTasks.some((t) => t.due_date !== null && t.due_date < todayIso) ? 'bg-red-500 dark:bg-red-400' : 'bg-amber-500 dark:bg-amber-400'}"></span>
							<h2 id="today-tasks" class="today-section-title">Tasks</h2>
							{#if dueTasks.length}
								<span class="today-count">{dueTasks.length}</span>
							{/if}
							<a href="{resolve('/tasks')}?new=1" class="btn-icon-ghost ml-auto -my-1" title="New task" aria-label="New task"><Plus size={15} /></a>
							<a href={resolve('/tasks')} class="-my-1 inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-xs font-medium text-slate-500 transition hover:bg-slate-100 hover:text-slate-800 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-200">
								All <ArrowRight size={12} />
							</a>
						</header>
						<!-- The list stays mounted so the last row's slide-out plays before the empty line fades in. -->
						<div class="py-1">
							{#each dueTasks as t (t.id)}
								<div out:rowLeave>
									<TaskRow
										task={t}
										{now}
										ticking={completing.rows[t.id]?.phase === 'tick'}
										syncOn={appleSync.on}
										onselect={() => openTask(t)}
										ontoggle={() => void complete(t)}
										actions={t.project_id !== null && t.issue_title ? logTimeAction : undefined}
									/>
								</div>
							{/each}
						</div>
						{#if tasks.loaded && dueTasks.length === 0}
							<p class="today-empty" in:fade={{ delay: 250, duration: 150 }}>Nothing overdue or due today</p>
						{:else if tasks.error && !tasks.loaded}
							<p class="form-error px-1 py-3">Could not load tasks: {tasks.error}</p>
						{/if}
					</section>
				{/if}
				<!-- Handovers sit below the sections. Mounted at once (they load on their own) and shown
				     once the page's load settled, success or failure, so they don't jump. -->
				<div class="space-y-4" hidden={!settled}>
					<HandoverIncoming cards={handovers} />
					<HandoverToday cards={handovers} />
				</div>
			</div>

			{#if loaded}
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
			{/if}
		</div>
	</div>
</div>

<svelte:window onfocus={reloadIfNewDay} />
<svelte:document onvisibilitychange={reloadIfNewDay} />

