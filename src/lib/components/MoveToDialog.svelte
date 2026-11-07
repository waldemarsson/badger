<script lang="ts">
	import { onMount } from 'svelte';
	import { scale, fade } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { targetFolderId, type MoveTarget } from '#lib/folders.js';
	import Combobox from '#lib/components/Combobox.svelte';
	import FolderInput from 'lucide-svelte/icons/folder-input';
	import X from 'lucide-svelte/icons/x';

	// "Move to…" for a note or folder: the keyboard alternative to drag and drop.
	let {
		name,
		location,
		targets,
		onpick,
		onclose
	}: {
		/** What is being moved (note title or folder name). */
		name: string;
		/** Where it is now, e.g. "Projects / Archive" ('' for the top level). */
		location: string;
		/** Every valid destination (the current place and invalid ones already left out). */
		targets: MoveTarget[];
		onpick: (folderId: number | null) => void;
		onclose: () => void;
	} = $props();

	let panel: HTMLElement;
	let value = $state('');

	onMount(() => panel.querySelector('input')?.focus());

	function pick(raw: string): void {
		const id = targetFolderId(raw);
		if (id !== undefined) onpick(id);
	}

	function onkeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
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
		bind:this={panel}
		class="w-full max-w-sm rounded-2xl bg-white shadow-2xl ring-1 ring-slate-900/10 dark:bg-neutral-900 dark:ring-neutral-800"
		transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
		role="dialog"
		aria-modal="true"
		aria-label="Move {name}"
	>
		<div class="flex items-start gap-3 px-5 pb-3 pt-4">
			<div class="rounded-lg bg-sky-50 p-1.5 text-sky-600 ring-1 ring-sky-100 dark:bg-sky-500/10 dark:text-sky-400 dark:ring-sky-500/20">
				<FolderInput size={16} />
			</div>
			<div class="min-w-0 flex-1">
				<p class="truncate text-sm font-semibold text-slate-900 dark:text-neutral-100">Move “{name}”</p>
				<p class="truncate text-xs text-slate-500 dark:text-neutral-400">Now in {location || 'Notes (root)'}</p>
			</div>
			<button
				type="button"
				class="btn-icon-ghost -mr-1.5 -mt-1"
				title="Close (Esc)"
				aria-label="Close"
				onclick={onclose}
			>
				<X size={16} />
			</button>
		</div>
		<div class="px-5 pb-5">
			{#if targets.length}
				<Combobox items={targets} bind:value placeholder="Search folders…" picker inline onchange={pick} />
				<p class="mt-2 text-xs text-slate-400 dark:text-neutral-500">↑↓ to choose · Enter to move · Esc to cancel</p>
			{:else}
				<p class="text-sm text-slate-500 dark:text-neutral-400">There is nowhere else to move it yet. Create a folder first.</p>
			{/if}
		</div>
	</div>
</div>
