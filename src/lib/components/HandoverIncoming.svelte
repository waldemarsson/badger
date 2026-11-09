<script lang="ts">
	// The incoming handover, near the bottom of Today's main column (just above today's editor): the newest earlier handover once its next
	// working day has come. Read-only, but task checkboxes tick and save; ←/→ browse every
	// past handover. State and the save queue are shared with today's editor (HandoverToday).
	import { onMount, tick, untrack } from 'svelte';
	import { fade } from 'svelte/transition';
	import { registerFlushHandler } from '#lib/flush-guard.js';
	import { handoverKey } from '#lib/handovers.js';
	import type { HandoverCards } from '#lib/handover-cards.svelte.js';
	import { formatDayLabel } from '#lib/today.js';
	import NoteEditor from '#lib/components/NoteEditor.svelte';
	import HandoverError from '#lib/components/HandoverError.svelte';
	import HandoverStatus from '#lib/components/HandoverStatus.svelte';
	import Check from '@lucide/svelte/icons/check';
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import FilePlus from '@lucide/svelte/icons/file-plus';
	import X from '@lucide/svelte/icons/x';

	let { cards }: { cards: HandoverCards } = $props();

	const shown = $derived(cards.shown);
	const shownKey = $derived(shown ? handoverKey(shown.date) : null);

	// Shows the selected past handover read-only (checkboxes still tick).
	$effect(() => {
		const ed = cards.pastEditor;
		const h = shown;
		if (!ed || !h) return;
		const key = handoverKey(h.date);
		const body = h.body;
		untrack(() => {
			void tick().then(() => {
				if (cards.links) ed.setLinks(cards.links);
				ed.setEditable(false);
				ed.open(key, body);
			});
		});
	});

	// History opened from today's card below: bring this card into view.
	let wasOpen = untrack(() => cards.historyOpen);
	$effect(() => {
		const open = cards.historyOpen;
		if (open && !wasOpen) void tick().then(() => cards.pastSection?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }));
		wasOpen = open;
	});

	function onKeydown(e: KeyboardEvent): void {
		if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
		if (e.target instanceof Element && e.target.closest('.cm-editor')) return;
		if (e.key === 'ArrowLeft') {
			e.preventDefault();
			cards.older();
		} else if (e.key === 'ArrowRight') {
			e.preventDefault();
			cards.newer();
		} else if (e.key === 'Escape' && cards.historyOpen) {
			e.preventDefault();
			cards.browsing = null;
		}
	}

	function onFocusout(e: FocusEvent): void {
		const next = e.relatedTarget;
		if (!(next instanceof Node) || !cards.pastSection?.contains(next)) void cards.flush();
	}

	onMount(() => {
		const unregister = registerFlushHandler(() => cards.flush());
		return () => {
			unregister();
			cards.flushOnUnmount();
		};
	});
</script>

<svelte:window onblur={() => void cards.flush()} />

{#if shown || cards.historyOpen}
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<section
		bind:this={cards.pastSection}
		class="today-section"
		aria-label={cards.historyOpen ? 'Past handovers' : 'Handover'}
		onkeydown={onKeydown}
		onfocusout={onFocusout}
		transition:fade={{ duration: 150 }}
	>
		<header class="today-section-header">
			<span class="h-2.5 w-2.5 shrink-0 rounded-full bg-sky-500 dark:bg-sky-400"></span>
			<h2 class="today-section-title min-w-0 truncate">
				{cards.historyOpen ? 'Past handovers' : 'Handover'}
				{#if shown}
					<span class="font-normal normal-case tracking-normal text-slate-500 dark:text-neutral-400">· {cards.historyOpen ? '' : 'from '}{formatDayLabel(shown.date)}</span>
				{/if}
			</h2>
			{#if cards.historyOpen && shown?.dismissed_at}
				<span class="kanban-badge shrink-0 bg-slate-50 text-slate-500 ring-slate-200 dark:bg-neutral-800/40 dark:text-neutral-400 dark:ring-neutral-700">Dismissed</span>
			{/if}
			<HandoverStatus status={shownKey === null ? 'saved' : cards.status(shownKey)} showSaved={false} />
			<div class="-my-1 ml-auto flex shrink-0 items-center gap-0.5">
				{#if cards.historyOpen && cards.past.length}
					<span class="mr-1 text-xs tabular-nums text-slate-400 dark:text-neutral-500">{cards.shownIndex + 1} of {cards.past.length}</span>
				{/if}
				{#if shown}
					{@const date = shown.date}
					<button
						type="button"
						class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40"
						title="Save as note"
						aria-label="Save handover from {formatDayLabel(date)} as note"
						disabled={cards.savingNote !== null}
						onclick={() => void cards.saveAsNote(date, 'past')}
					>
						<FilePlus size={16} />
					</button>
				{/if}
				<button
					type="button"
					class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40"
					title="Older (←)"
					aria-label="Older handover"
					disabled={cards.shownIndex + 1 >= cards.past.length}
					onclick={() => cards.older()}
				>
					<ChevronLeft size={16} />
				</button>
				<button
					type="button"
					class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40"
					title="Newer (→)"
					aria-label="Newer handover"
					disabled={cards.browsing === null || cards.browsing === 0}
					onclick={() => cards.newer()}
				>
					<ChevronRight size={16} />
				</button>
				<span class="mx-1 h-4 w-px bg-slate-200 dark:bg-neutral-800" aria-hidden="true"></span>
				{#if cards.historyOpen}
					<button type="button" class="btn-icon-ghost" title="Close history (Esc)" aria-label="Close history" onclick={() => (cards.browsing = null)}>
						<X size={16} />
					</button>
				{:else}
					<button type="button" class="handover-dismiss" disabled={cards.dismissing} onclick={() => void cards.dismiss()}>
						<Check size={14} />
						Dismiss
					</button>
				{/if}
			</div>
		</header>
		{#if shown}
			<div class="handover-incoming">
				<NoteEditor
					bind:this={cards.pastEditor}
					onchange={(key, body) => cards.onPastChange(key, body)}
					onfollow={(link) => void cards.follow(link, 'past')}
					followScope="notes"
					class="handover-editor"
				/>
			</div>
		{:else}
			<p class="today-empty">No past handovers yet. Write one below and it shows here on your next working day.</p>
		{/if}
		{#if shownKey !== null && cards.error(shownKey)}
			<HandoverError message="Couldn't save: {cards.error(shownKey)}" onretry={() => void cards.flush()} />
		{/if}
		{#if cards.pastError}
			<HandoverError message={cards.pastError} ondismiss={() => (cards.pastError = '')} />
		{/if}
	</section>
{/if}
