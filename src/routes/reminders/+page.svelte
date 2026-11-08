<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { tick, untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade, slide } from 'svelte/transition';
	import type { Reminder } from '#lib/types.js';
	import { groupReminders } from '#lib/reminders.js';
	import { formatShortcut } from '#lib/palette.js';
	import { palette } from '#lib/palette-state.svelte.js';
	import { clock, highlightRequest, reminders, toastArea } from '#lib/reminders-state.svelte.js';
	import ReminderForm from '#lib/components/ReminderForm.svelte';
	import ReminderRow from '#lib/components/ReminderRow.svelte';
	import Bell from '@lucide/svelte/icons/bell';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Plus from '@lucide/svelte/icons/plus';

	const FLASH_MS = 1600;

	let adding = $state(false);
	let addText = $state('');
	let formKey = $state(0);
	let editingId = $state<number | null>(null);
	let doneOpen = $state(false);
	let flashId = $state<number | null>(null);

	const now = $derived(new Date(clock.ms));
	const groups = $derived(groupReminders(reminders.list, now));
	const activeCount = $derived(groups.overdue.length + groups.today.length + groups.tomorrow.length + groups.later.length);
	const upcoming = $derived(groups.tomorrow.length + groups.later.length);

	function startAdd(text = ''): void {
		editingId = null;
		addText = text;
		formKey += 1;
		adding = true;
	}

	// `?new=1[&text=…]` (Today card, ⌘K without a recognised time) opens the form once.
	$effect(() => {
		const params = page.url.searchParams;
		if (params.get('new') !== '1') return;
		const text = params.get('text') ?? '';
		untrack(() => startAdd(text));
		void goto(resolve('/reminders'), { replace: true, reset: false });
	});

	// Notification click: scroll to the row and flash it.
	let flashTimer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		if (!highlightRequest.pending || !reminders.loaded) return;
		const id = highlightRequest.id;
		highlightRequest.pending = false;
		if (id === null) {
			window.scrollTo({ top: 0, behavior: 'smooth' });
			return;
		}
		if (groups.done.some((r) => r.id === id)) doneOpen = true;
		flashId = null;
		void tick().then(() => {
			flashId = id;
			document.querySelector(`[data-reminder-id="${id}"]`)?.scrollIntoView({ block: 'center', behavior: 'smooth' });
			clearTimeout(flashTimer);
			flashTimer = setTimeout(() => (flashId = null), FLASH_MS);
		});
	});

	$effect(() => () => clearTimeout(flashTimer));
</script>

{#snippet rows(items: Reminder[])}
	<div class="divide-y divide-slate-100 dark:divide-neutral-800">
		{#each items as r (r.id)}
			<div animate:flip={{ duration: 200 }} out:fade={{ duration: 150 }}>
				{#if editingId === r.id}
					<div class="py-2" in:fade={{ duration: 120 }}>
						<ReminderForm reminder={r} onsaved={() => (editingId = null)} oncancel={() => (editingId = null)} />
					</div>
				{:else}
					<ReminderRow {r} {now} flash={flashId === r.id} onedit={() => ((editingId = r.id), (adding = false))} />
				{/if}
			</div>
		{/each}
	</div>
{/snippet}

{#snippet section(title: string, dot: string, items: Reminder[])}
	{#if items.length}
		<section class="today-section" transition:fade={{ duration: 150 }}>
			<header class="today-section-header">
				<span class="h-2.5 w-2.5 rounded-full {dot}"></span>
				<h2 class="today-section-title">{title}</h2>
				<span class="today-count ml-auto">{items.length}</span>
			</header>
			{@render rows(items)}
		</section>
	{/if}
{/snippet}

<div class="page-shell">
	<!-- Room below the last rows while toasts show, so they don't cover its actions. -->
	<div class="page-view" style:padding-bottom={toastArea.height ? `calc(${toastArea.height}px + 2rem)` : null}>
		<h1 class="sr-only">Reminders</h1>

		<div class="table-toolbar flex flex-wrap items-center justify-between gap-3 px-1">
			<div class="toolbar-stats">
				<span class="toolbar-stat">
					<span class="toolbar-stat-label">Overdue</span>
					<span class="toolbar-stat-value {groups.overdue.length ? 'font-semibold text-red-600 dark:text-red-400' : ''}">{groups.overdue.length}</span>
				</span>
				<span class="toolbar-stat">
					<span class="toolbar-stat-label">Today</span>
					<span class="toolbar-stat-value">{groups.today.length}</span>
				</span>
				<span class="toolbar-stat">
					<span class="toolbar-stat-label">Upcoming</span>
					<span class="toolbar-stat-value">{upcoming}</span>
				</span>
			</div>
			<div class="flex items-center gap-3">
				<span class="hidden text-xs text-slate-400 sm:inline dark:text-neutral-500">
					or <kbd class="palette-kbd">{formatShortcut(palette.shortcut)}</kbd> <span class="font-mono">remind call Anna in 20 min</span>
				</span>
				<button type="button" class="btn-secondary-sm gap-1" onclick={() => startAdd()}>
					<Plus size={14} /> Reminder
				</button>
			</div>
		</div>

		{#if adding}
			<div transition:slide={{ duration: 150 }}>
				{#key formKey}
					<ReminderForm text={addText} onsaved={() => (adding = false)} oncancel={() => (adding = false)} />
				{/key}
			</div>
		{/if}

		{#if reminders.error}
			<div class="stack-error" role="alert">Could not load reminders: {reminders.error}</div>
		{/if}

		{#if reminders.loaded}
			{#if activeCount === 0 && !adding}
				<div class="empty-state flex flex-col items-center gap-2 py-10" in:fade={{ duration: 150 }}>
					<Bell size={22} class="text-slate-300 dark:text-neutral-600" />
					<p class="font-medium text-slate-700 dark:text-neutral-300">No reminders</p>
					<p class="text-xs">
						Press <kbd class="palette-kbd">{formatShortcut(palette.shortcut)}</kbd> and type
						<span class="font-mono text-slate-600 dark:text-neutral-300">remind send invoice tomorrow 9</span>, or
					</p>
					<button type="button" class="btn-secondary-sm mt-1 gap-1" onclick={() => startAdd()}><Plus size={14} /> New reminder</button>
				</div>
			{/if}

			{@render section('Overdue', 'bg-red-500 dark:bg-red-400', groups.overdue)}
			{@render section('Today', 'bg-amber-500 dark:bg-amber-400', groups.today)}
			{@render section('Tomorrow', 'bg-sky-500 dark:bg-sky-400', groups.tomorrow)}
			{@render section('Later', 'bg-slate-400 dark:bg-neutral-500', groups.later)}

			{#if groups.done.length}
				<section class="today-section">
					<button
						type="button"
						class="today-section-header w-full text-left transition-colors hover:bg-slate-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-sky-400 dark:hover:bg-neutral-800/60"
						aria-expanded={doneOpen}
						onclick={() => (doneOpen = !doneOpen)}
					>
						<ChevronRight size={14} class="shrink-0 text-slate-400 transition-transform duration-150 dark:text-neutral-500 {doneOpen ? 'rotate-90' : ''}" />
						<span class="h-2.5 w-2.5 rounded-full bg-emerald-500 dark:bg-emerald-400"></span>
						<h2 class="today-section-title">Done</h2>
						<span class="today-count ml-auto">{groups.done.length}</span>
					</button>
					{#if doneOpen}
						<div transition:slide={{ duration: 150 }}>
							{@render rows(groups.done)}
							{#if groups.done.length >= 50}
								<p class="today-empty text-xs">Showing the 50 most recent</p>
							{/if}
						</div>
					{/if}
				</section>
			{/if}
		{/if}
	</div>
</div>
