<script lang="ts">
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import type { Reminder } from '#lib/types.js';
	import ReminderForm from '#lib/components/ReminderForm.svelte';
	import Bell from '@lucide/svelte/icons/bell';
	import X from '@lucide/svelte/icons/x';

	// "Remind me…" for a note (context menu): the reminder form in a small modal.
	let {
		noteId,
		title,
		onsaved,
		onclose
	}: {
		noteId: number;
		/** Note title: the default text and the link chip. */
		title: string;
		onsaved: (r: Reminder) => void;
		onclose: () => void;
	} = $props();

	function onkeydown(e: KeyboardEvent): void {
		// The form handles its own Esc (cancel, or closing an open picker list) first.
		if (e.key === 'Escape' && !e.defaultPrevented) {
			e.preventDefault();
			onclose();
		}
	}
</script>

<svelte:window {onkeydown} />

<div
	class="fixed inset-0 z-40 flex items-start justify-center bg-slate-950/40 p-4 pt-[14vh] backdrop-blur-[2px] dark:bg-black/60"
	transition:fade={{ duration: 150 }}
	onclick={(e) => e.target === e.currentTarget && onclose()}
	role="presentation"
>
	<div
		class="w-full max-w-lg rounded-2xl bg-white shadow-2xl ring-1 ring-slate-900/10 dark:bg-neutral-900 dark:ring-neutral-800"
		transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
		role="dialog"
		aria-modal="true"
		aria-label="Remind me about {title}"
	>
		<div class="flex items-center gap-2 border-b border-slate-100 px-5 pb-3 pt-4 dark:border-neutral-800">
			<Bell size={16} class="text-slate-400 dark:text-neutral-500" />
			<h2 class="min-w-0 flex-1 truncate text-sm font-semibold">Remind me</h2>
			<button type="button" class="btn-icon-ghost" onclick={onclose} title="Close (Esc)" aria-label="Close"><X size={16} /></button>
		</div>
		<div class="px-5 pb-5 pt-4">
			<ReminderForm text={title} {noteId} linkTitle={title} bare {onsaved} oncancel={onclose} />
		</div>
	</div>
</div>
