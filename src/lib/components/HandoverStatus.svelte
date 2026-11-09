<script lang="ts">
	// A handover card's autosave state, next to its title (like the scratchpad's).
	import Check from '@lucide/svelte/icons/check';
	import type { SaveStatus } from '#lib/note-save-queue.js';

	let { status, showSaved }: { status: SaveStatus; showSaved: boolean } = $props();
</script>

<span class="note-save-state" aria-live="polite">
	{#if status === 'error'}
		<span class="text-red-600 dark:text-red-400">Not saved</span>
	{:else if status === 'saving' || status === 'pending'}
		<span class="h-1.5 w-1.5 animate-pulse rounded-full bg-sky-500 dark:bg-sky-400"></span>
		Saving…
	{:else if showSaved}
		<Check size={13} class="text-emerald-500 dark:text-emerald-400" />
		Saved
	{/if}
</span>
