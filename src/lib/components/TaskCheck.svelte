<script lang="ts">
	// The round task check. Hover previews the tick; `ticking` plays the completion (pop, tick
	// draw, six sparks; app.css). Red ring when the task needs attention (D10).
	let {
		done = false,
		attention = false,
		ticking = false,
		label,
		disabled = false,
		onclick
	}: {
		done?: boolean;
		attention?: boolean;
		ticking?: boolean;
		/** "Complete X" / "Mark X as not done". */
		label: string;
		disabled?: boolean;
		onclick: () => void;
	} = $props();

	const SPARKS = [0, 60, 120, 180, 240, 300];
</script>

<button
	type="button"
	class="task-check"
	class:task-check-ticking={ticking}
	class:task-check-done={done && !ticking}
	class:task-check-attention={attention && !done && !ticking}
	aria-label={label}
	aria-pressed={done || ticking}
	data-task-check
	{disabled}
	{onclick}
>
	<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path class="task-tick" d="M20 6 9 17l-5-5"></path></svg>
	{#if ticking}
		<span class="task-burst" aria-hidden="true">
			{#each SPARKS as a (a)}<i style="--a: {a}deg"></i>{/each}
		</span>
	{/if}
</button>
