<script lang="ts">
	import { cn } from '#lib/utils.js';

	type Item = { value: string | number; label: string };

	let {
		items = [],
		value = $bindable(''),
		placeholder = 'Select...',
		allowNew = false,
		disabled = false,
		onchange
	}: {
		items: Item[];
		value?: string;
		placeholder?: string;
		allowNew?: boolean;
		disabled?: boolean;
		onchange?: (v: string) => void;
	} = $props();

	let open = $state(false);
	let inputText = $state('');
	let containerEl: HTMLElement;

	// When value changes externally, sync the display text
	$effect(() => {
		const match = items.find((i) => String(i.value) === String(value));
		inputText = match ? match.label : value ?? '';
	});

	const filtered = $derived(
		items.filter((i) => i.label.toLowerCase().includes(inputText.toLowerCase()))
	);

	const isNew = $derived(
		allowNew &&
			inputText.trim().length > 0 &&
			!items.some((i) => i.label.toLowerCase() === inputText.toLowerCase().trim())
	);

	function select(item: Item) {
		value = String(item.value);
		inputText = item.label;
		open = false;
		onchange?.(String(item.value));
	}

	function handleInput() {
		open = true;
		if (allowNew) {
			value = inputText;
			onchange?.(inputText);
		}
	}

	function handleFocusOut(e: FocusEvent) {
		// Close only if focus moves outside the container
		if (!containerEl?.contains(e.relatedTarget as Node)) {
			open = false;
			// If not allowNew, reset to last valid value
			if (!allowNew) {
				const match = items.find((i) => String(i.value) === String(value));
				inputText = match ? match.label : '';
			}
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			open = false;
		} else if (e.key === 'ArrowDown' && filtered.length > 0) {
			open = true;
		}
	}
</script>

<div bind:this={containerEl} class="relative" onfocusout={handleFocusOut}>
	<div
		class={cn(
			'relative flex items-center',
			disabled && 'pointer-events-none opacity-50'
		)}
	>
		<input
			type="text"
			class="form-input w-full pr-8"
			{placeholder}
			{disabled}
			bind:value={inputText}
			oninput={handleInput}
			onfocus={() => (open = true)}
			onkeydown={handleKeydown}
			autocomplete="off"
		/>
		<svg
			class="pointer-events-none absolute right-2 h-4 w-4 text-slate-400 dark:text-neutral-500"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			stroke-width="2"
		>
			<path d="M6 9l6 6 6-6" />
		</svg>
	</div>

	{#if open && (filtered.length > 0 || isNew)}
		<ul
			class="absolute z-50 mt-1 max-h-60 w-full overflow-auto rounded-xl border border-slate-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 shadow-lg dark:shadow-black/40"
		>
			{#each filtered.slice(0, 10) as item (item.value)}
				<li>
					<button
						type="button"
						class="w-full px-3 py-2 text-left text-sm text-slate-700 dark:text-neutral-300 hover:bg-slate-50 dark:hover:bg-neutral-700 focus:bg-slate-50 dark:focus:bg-neutral-700 focus:outline-none"
						onmousedown={() => select(item)}
					>
						{item.label}
					</button>
				</li>
			{/each}
			{#if isNew}
				<li>
					<button
						type="button"
						class="w-full px-3 py-2 text-left text-sm hover:bg-slate-50 dark:hover:bg-neutral-700 focus:outline-none"
						onmousedown={() => {
							value = inputText;
							onchange?.(inputText);
							open = false;
						}}
					>
						<span class="font-medium text-slate-950 dark:text-neutral-100">{inputText.trim()}</span>
						<span class="ml-1 text-xs font-normal text-slate-400 dark:text-neutral-500">(new)</span>
					</button>
				</li>
			{/if}
		</ul>
	{/if}
</div>
