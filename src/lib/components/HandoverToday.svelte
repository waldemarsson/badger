<script lang="ts">
	// Today's handover editor at the bottom of Today's main column, under the incoming card:
	// a note to your future self, shown in that card on the next working day. Autosaves through the queue it
	// shares with the incoming card (HandoverIncoming); ⌘K "Write handover" focuses it.
	import { onMount, untrack } from 'svelte';
	import { registerFlushHandler } from '#lib/flush-guard.js';
	import { nextWorkingDay } from '#lib/handovers.js';
	import type { HandoverCards } from '#lib/handover-cards.svelte.js';
	import { handoverRequest, takeHandoverFocus } from '#lib/handover-state.svelte.js';
	import { formatDayLabel } from '#lib/today.js';
	import NoteEditor from '#lib/components/NoteEditor.svelte';
	import HandoverError from '#lib/components/HandoverError.svelte';
	import HandoverStatus from '#lib/components/HandoverStatus.svelte';
	import FilePlus from '@lucide/svelte/icons/file-plus';
	import RotateCcwClock from '@lucide/svelte/icons/rotate-ccw-clock';

	let { cards }: { cards: HandoverCards } = $props();

	const saveError = $derived(cards.error(cards.todayKey));

	// ⌘K "Write handover": take a request made before Today mounted, or any new one.
	$effect(() => {
		void handoverRequest.id;
		untrack(() => {
			if (takeHandoverFocus()) void cards.focusToday();
		});
	});

	function onFocusout(e: FocusEvent): void {
		const next = e.relatedTarget;
		if (!(next instanceof Node) || !cards.todaySection?.contains(next)) void cards.flush();
	}

	onMount(() => {
		void cards.refreshLinks();
		const unregister = registerFlushHandler(() => cards.flush());
		return () => {
			unregister();
			cards.flushOnUnmount();
		};
	});
</script>

<svelte:window onblur={() => void cards.flush()} />

<section bind:this={cards.todaySection} class="today-section" aria-label="Today's handover" onfocusout={onFocusout}>
	<header class="today-section-header">
		<span class="h-2.5 w-2.5 shrink-0 rounded-full bg-slate-400 dark:bg-neutral-500"></span>
		<h2 class="today-section-title min-w-0 truncate">
			Today's handover
			<span class="font-normal normal-case tracking-normal text-slate-500 dark:text-neutral-400">· for {formatDayLabel(nextWorkingDay(cards.editorDate))}</span>
		</h2>
		<HandoverStatus status={cards.status(cards.todayKey)} showSaved={!cards.blank} />
		<div class="-my-1 ml-auto flex shrink-0 items-center gap-0.5">
			<button
				type="button"
				class="btn-icon-ghost {cards.historyOpen ? 'text-sky-600 dark:text-sky-400' : ''}"
				title={cards.historyOpen ? 'Close history' : 'Past handovers'}
				aria-label={cards.historyOpen ? 'Close past handovers' : 'Show past handovers'}
				aria-pressed={cards.historyOpen}
				onclick={() => cards.toggleHistory()}
			>
				<RotateCcwClock size={16} />
			</button>
			<button
				type="button"
				class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40"
				title="Save as note"
				aria-label="Save today's handover as note"
				disabled={cards.blank || cards.savingNote !== null || !cards.editorReady}
				onclick={() => void cards.saveAsNote(cards.editorDate, 'today')}
			>
				<FilePlus size={16} />
			</button>
		</div>
	</header>
	{#if cards.loadError}
		<HandoverError message="Couldn't load handovers: {cards.loadError}" onretry={() => cards.retry()} />
	{/if}
	<!-- Hidden, not unmounted, until the stored text is in: it keeps the undo history. -->
	<div class="handover-today {cards.editorReady && !cards.loadError ? '' : 'hidden'}">
		<NoteEditor
			bind:this={cards.todayEditor}
			onchange={(key, body) => cards.onTodayChange(key, body)}
			onfollow={(link) => void cards.follow(link, 'today')}
			followScope="notes"
			class="handover-editor handover-editor-today"
		/>
	</div>
	<p class="px-1 pt-1.5 text-xs text-slate-400 dark:text-neutral-500">
		A note to your future self, shown just above on your next working day. Not in search.
	</p>
	{#if saveError}
		<HandoverError message="Couldn't save: {saveError}. Your text is kept and saving retries as you type." onretry={() => void cards.flush()} />
	{/if}
	{#if cards.todayError}
		<HandoverError message={cards.todayError} ondismiss={() => (cards.todayError = '')} />
	{/if}
</section>
