<script lang="ts">
	import Laptop from '@lucide/svelte/icons/laptop';

	// "Mac only" mark on todo cards and reminder rows kept off the phone (sync on only).
	let { title = 'Mac only: kept off your phone' }: { title?: string } = $props();
</script>

<span class="kanban-badge mac-only-badge shrink-0" {title}>
	<Laptop size={12} class="shrink-0" aria-hidden="true" />
	<span class="sr-only">Mac only</span>
</span>
