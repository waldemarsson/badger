<script lang="ts">
	// An inline error under a handover card, with optional Retry and Dismiss.
	import { fade } from 'svelte/transition';
	import CircleAlert from '@lucide/svelte/icons/circle-exclamation-point';
	import X from '@lucide/svelte/icons/x';

	let { message, ondismiss, onretry }: { message: string; ondismiss?: () => void; onretry?: () => void } = $props();
</script>

<div class="notes-inline-error mt-2" role="alert" transition:fade={{ duration: 150 }}>
	<CircleAlert size={14} class="mt-0.5 shrink-0" />
	<span class="min-w-0 flex-1">{message}</span>
	{#if onretry}
		<button type="button" class="shrink-0 font-semibold underline-offset-2 hover:underline" onclick={onretry}>Retry</button>
	{/if}
	{#if ondismiss}
		<button type="button" class="shrink-0" onclick={ondismiss} title="Dismiss" aria-label="Dismiss"><X size={14} /></button>
	{/if}
</div>
