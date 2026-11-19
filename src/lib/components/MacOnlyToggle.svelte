<script lang="ts">
	import Laptop from '@lucide/svelte/icons/laptop';

	// "Keep off phone": a small switch for todos and reminders, shown while Apple Reminders
	// sync is on. Disabled with a hint when the value is decided elsewhere (its todo).

	let {
		checked = $bindable(false),
		disabled = false,
		hint = ''
	}: {
		checked?: boolean;
		disabled?: boolean;
		/** Shown after the label, e.g. "Follows its todo". */
		hint?: string;
	} = $props();

	const id = $props.id();
</script>

<div class="mac-only-toggle" class:mac-only-toggle-disabled={disabled}>
	<button
		type="button"
		role="switch"
		class="mini-switch"
		class:mini-switch-on={checked}
		aria-checked={checked}
		aria-labelledby="{id}-label"
		aria-describedby="{id}-hint"
		{disabled}
		onclick={() => (checked = !checked)}
	>
		<span class="mini-switch-knob"></span>
	</button>
	<span id="{id}-label" class="inline-flex items-center gap-1">
		<Laptop size={13} class="shrink-0" />
		Keep off phone
	</span>
	<span id="{id}-hint" class="text-slate-400 dark:text-neutral-500">{hint || 'Not synced to Apple Reminders'}</span>
</div>
