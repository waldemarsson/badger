<script lang="ts">
	import { onMount } from 'svelte';

	// Inline name editor for create and rename.
	// Enter saves, Esc cancels, blur saves only if the value changed from `initial`
	// (so blurring an empty add-input simply cancels).
	let {
		initial = '',
		placeholder = '',
		label,
		onsave,
		oncancel
	}: {
		initial?: string;
		placeholder?: string;
		label: string;
		onsave: (name: string) => Promise<void>;
		oncancel: () => void;
	} = $props();

	// The editor is mounted per edit, so snapshotting the starting value is intended.
	// svelte-ignore state_referenced_locally
	let value = $state(initial);
	let error = $state('');
	let busy = $state(false);
	let done = false; // set once saved or cancelled, so a trailing blur is ignored
	let lastFailed: string | null = null; // skip blur-retry of a value the backend just rejected
	let input: HTMLInputElement;

	onMount(() => {
		input.focus();
		input.select();
	});

	async function save(fromBlur = false) {
		if (done || busy) return;
		if (fromBlur && value.trim() === lastFailed) return;
		if (value.trim() === initial.trim()) return cancel();
		if (!value.trim()) {
			error = 'Name is required';
			return;
		}
		busy = true;
		error = '';
		try {
			await onsave(value.trim());
			done = true;
		} catch (e) {
			error = String(e);
			lastFailed = value.trim();
		} finally {
			busy = false;
		}
	}

	function cancel() {
		if (done) return;
		done = true;
		oncancel();
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.isComposing || e.keyCode === 229) return; // WebKit: IME-commit Enter
		if (e.key === 'Enter') {
			e.preventDefault();
			save();
		} else if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			cancel();
		}
	}
</script>

<div class="min-w-0 flex-1">
	<input
		bind:this={input}
		bind:value
		class="name-input {error ? 'name-input-error' : ''}"
		aria-label={label}
		aria-invalid={error ? 'true' : undefined}
		{placeholder}
		readonly={busy}
		{onkeydown}
		onblur={() => save(true)}
	/>
	{#if error}
		<p class="form-error mt-1">{error}</p>
	{/if}
</div>
