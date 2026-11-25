<script lang="ts">
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import type { Task } from '#lib/types.js';
	import { snoozeUntil, type SnoozeChoice } from '#lib/time-phrases.js';
	import { customReminderStart, MAX_TITLE } from '#lib/tasks.js';
	import { createTask } from '#lib/tasks-state.svelte.js';
	import { backdropDismiss } from '#lib/backdrop.js';
	import Bell from '@lucide/svelte/icons/bell';
	import FileText from '@lucide/svelte/icons/file-text';
	import X from '@lucide/svelte/icons/x';

	// "Remind me…" for a note (context menu): creates a task linked to the note with one
	// custom reminder. The task shows on /tasks; deleting the note only unlinks it.
	let {
		noteId,
		title: noteTitle,
		onsaved,
		onclose
	}: {
		noteId: number;
		/** Note title: the default task title and the link chip. */
		title: string;
		onsaved: (t: Task) => void;
		onclose: () => void;
	} = $props();

	const backdrop = backdropDismiss(() => onclose());

	// Starts from the note title once; editing it is the user's.
	// svelte-ignore state_referenced_locally
	let title = $state(noteTitle);
	let at = $state(customReminderStart({ due_date: null, due_time: null }, new Date()));
	let error = $state('');
	let saving = $state(false);

	const QUICK: { choice: SnoozeChoice; label: string }[] = [
		{ choice: '20m', label: 'In 20 min' },
		{ choice: '1h', label: 'In 1 hour' },
		{ choice: '3h', label: 'In 3 hours' },
		{ choice: 'tomorrow', label: 'Tomorrow 09:00' }
	];

	async function save(): Promise<void> {
		if (saving) return;
		error = '';
		if (!title.trim()) {
			error = 'Title is required';
			return;
		}
		if (!at) {
			error = 'Pick when to remind';
			return;
		}
		saving = true;
		try {
			onsaved(await createTask({ title: title.trim(), noteId }, [{ at }]));
		} catch (e) {
			error = String(e);
		} finally {
			saving = false;
		}
	}

	function onkeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		if (e.key === 'Escape' && !e.defaultPrevented) {
			e.preventDefault();
			onclose();
		} else if (e.key === 'Enter' && e.target instanceof HTMLInputElement) {
			e.preventDefault();
			void save();
		}
	}

	function autofocus(node: HTMLInputElement): void {
		node.focus();
		node.select();
	}
</script>

<svelte:window {onkeydown} />

<div
	class="fixed inset-0 z-40 flex items-start justify-center bg-slate-950/40 p-4 pt-[14vh] backdrop-blur-[2px] dark:bg-black/60"
	transition:fade={{ duration: 150 }}
	{...backdrop}
	role="presentation"
>
	<div
		class="w-full max-w-lg rounded-2xl bg-white shadow-2xl ring-1 ring-slate-900/10 dark:bg-neutral-900 dark:ring-neutral-800"
		transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
		role="dialog"
		aria-modal="true"
		aria-label="Remind me about {noteTitle}"
	>
		<div class="flex items-center gap-2 border-b border-slate-100 px-5 pb-3 pt-4 dark:border-neutral-800">
			<Bell size={16} class="text-slate-400 dark:text-neutral-500" />
			<h2 class="min-w-0 flex-1 truncate text-sm font-semibold">Remind me</h2>
			<button type="button" class="btn-icon-ghost" onclick={onclose} title="Close (Esc)" aria-label="Close"><X size={16} /></button>
		</div>
		<div class="space-y-4 px-5 pb-5 pt-4">
			<label class="block space-y-1.5">
				<span class="form-label">Task</span>
				<input class="form-input" maxlength={MAX_TITLE} bind:value={title} use:autofocus />
			</label>
			<div class="space-y-1.5">
				<label class="form-label" for="remind-at">When</label>
				<input id="remind-at" type="datetime-local" class="form-input" bind:value={at} />
				<div class="flex flex-wrap gap-1.5">
					{#each QUICK as q (q.choice)}
						<button type="button" class="btn-secondary-sm" onclick={() => (at = snoozeUntil(q.choice, new Date()).slice(0, 16))}>{q.label}</button>
					{/each}
				</div>
			</div>
			<p class="task-help flex items-center gap-1.5">
				<FileText size={12} class="shrink-0" /><span class="min-w-0 truncate">Linked to “{noteTitle}” · shows on Tasks</span>
			</p>
			{#if error}
				<p class="form-error" role="alert">{error}</p>
			{/if}
			<div class="flex justify-end gap-2">
				<button type="button" class="btn-secondary" onclick={onclose}>Cancel</button>
				<button type="button" class="btn-primary" disabled={saving} onclick={() => void save()}>Remind me</button>
			</div>
		</div>
	</div>
</div>
