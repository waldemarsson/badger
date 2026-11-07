<script lang="ts">
	import { fly } from 'svelte/transition';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { toast } from '#lib/palette-state.svelte.js';

	const DURATION_MS = 2500;

	$effect(() => {
		const id = toast.id;
		if (!toast.message) return;
		const timer = setTimeout(() => {
			if (toast.id === id) toast.message = '';
		}, DURATION_MS);
		return () => clearTimeout(timer);
	});
</script>

<div class="pointer-events-none fixed bottom-4 right-4 z-[70]" role="status" aria-live="polite">
	{#key toast.id}
		{#if toast.message}
			<div
				class="flex items-center gap-2 rounded-xl bg-slate-900 px-4 py-2.5 text-sm font-medium text-white shadow-lg ring-1 ring-slate-900/10 dark:bg-neutral-100 dark:text-neutral-900 dark:ring-neutral-100/10"
				transition:fly={{ y: 8, duration: 180 }}
			>
				<CircleCheck size={16} class="text-emerald-400 dark:text-emerald-600" />
				{toast.message}
			</div>
		{/if}
	{/key}
</div>
