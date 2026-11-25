<script lang="ts">
	import { fly } from 'svelte/transition';
	import { undo, undoCompletion } from '#lib/tasks-state.svelte.js';
	import { motionMs } from '#lib/task-transitions.js';
	import CircleCheck from '@lucide/svelte/icons/circle-check';

	// "Completed “X” · Undo" for 5 s after a completion, on every page (bottom centre, clear of
	// the reminder toasts). The region is live, so screen readers hear the completion and that
	// Undo is there; ⌘Z (outside text fields) undoes too.

	$effect(() => {
		const u = undo.pending;
		if (!u) return;
		const timer = setTimeout(() => {
			if (undo.pending === u) {
				undo.pending = null;
				undo.error = '';
			}
		}, Math.max(0, u.expires - Date.now()));
		return () => clearTimeout(timer);
	});

	function editable(el: Element | null): boolean {
		return el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
	}

	function onkeydown(e: KeyboardEvent): void {
		if (!undo.pending || e.key.toLowerCase() !== 'z' || !(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return;
		if (editable(document.activeElement)) return;
		e.preventDefault();
		void undoCompletion();
	}
</script>

<svelte:window {onkeydown} />

<div class="pointer-events-none fixed inset-x-0 bottom-4 z-[60] flex justify-center px-4" role="status" aria-live="polite">
	{#if undo.pending}
		{#key undo.pending.taskId}
			<div class="task-undo" transition:fly={{ y: 8, duration: motionMs(180) }}>
				<CircleCheck size={16} class="shrink-0 text-emerald-400 dark:text-emerald-600" aria-hidden="true" />
				<span class="min-w-0 truncate">{undo.error || undo.pending.message}</span>
				<button type="button" class="task-undo-button" onclick={() => void undoCompletion()} title="Undo (⌘Z)">Undo</button>
			</div>
		{/key}
	{/if}
</div>
