<script lang="ts">
	import { cn } from '#lib/utils.js';

	type Item = { value: string | number; label: string };

	let {
		items = [],
		value = $bindable(''),
		placeholder = 'Select...',
		allowNew = false,
		disabled = false,
		picker = false,
		inline = false,
		onchange
	}: {
		items: Item[];
		value?: string;
		placeholder?: string;
		allowNew?: boolean;
		disabled?: boolean;
		/**
		 * Keyboard picker (outside forms): ↑/↓ and hover highlight an item, typing highlights
		 * the first match, Enter picks the highlighted one, and every match is listed.
		 */
		picker?: boolean;
		/** The list sits in the flow below the input (its container grows) instead of floating. */
		inline?: boolean;
		onchange?: (v: string) => void;
	} = $props();

	let open = $state(false);
	let inputText = $state('');
	let containerEl: HTMLElement;
	let listEl = $state<HTMLUListElement>();
	let active = $state(-1); // picker: highlighted item (↑/↓, hover); Enter picks it

	// When value changes externally, sync the display text
	$effect(() => {
		const match = items.find((i) => String(i.value) === String(value));
		inputText = match ? match.label : value ?? '';
	});

	const filtered = $derived(
		items.filter((i) => i.label.toLowerCase().includes(inputText.toLowerCase()))
	);

	const shown = $derived(picker ? filtered : filtered.slice(0, 10));

	$effect(() => {
		if (open && active >= 0) listEl?.children[active]?.scrollIntoView({ block: 'nearest' });
	});

	const isNew = $derived(
		allowNew &&
			inputText.trim().length > 0 &&
			!items.some((i) => i.label.toLowerCase() === inputText.toLowerCase().trim())
	);

	function select(item: Item) {
		value = String(item.value);
		inputText = item.label;
		open = false;
		active = -1;
		onchange?.(String(item.value));
	}

	function handleInput() {
		open = true;
		active = picker ? 0 : -1;
		if (allowNew) {
			value = inputText;
			onchange?.(inputText);
		}
	}

	function handleFocusOut(e: FocusEvent) {
		// Close only if focus moves outside the container
		if (!containerEl?.contains(e.relatedTarget as Node)) {
			open = false;
			active = -1;
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
			active = -1;
		} else if (e.key === 'ArrowDown' && filtered.length > 0) {
			open = true;
			if (picker) {
				e.preventDefault();
				active = Math.min(active + 1, shown.length - 1);
			}
		} else if (picker && e.key === 'ArrowUp' && open && shown.length > 0) {
			e.preventDefault();
			active = Math.max(active - 1, 0);
		} else if (picker && e.key === 'Enter' && open && shown[active]) {
			e.preventDefault();
			select(shown[active]);
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
			onfocus={() => {
				open = true;
				active = -1;
			}}
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
			bind:this={listEl}
			class={cn(
				'mt-1 max-h-60 w-full overflow-auto rounded-xl border border-slate-200 dark:border-neutral-700 bg-white dark:bg-neutral-800',
				inline ? 'shadow-sm dark:shadow-black/20' : 'absolute z-50 shadow-lg dark:shadow-black/40'
			)}
		>
			{#each shown as item, i (item.value)}
				<li>
					<button
						type="button"
						class={cn(
							'w-full px-3 py-2 text-left text-sm text-slate-700 dark:text-neutral-300 hover:bg-slate-50 dark:hover:bg-neutral-700 focus:bg-slate-50 dark:focus:bg-neutral-700 focus:outline-none',
							picker && i === active && 'bg-slate-50 dark:bg-neutral-700'
						)}
						onmousedown={() => select(item)}
						onmouseenter={picker ? () => (active = i) : undefined}
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
	{:else if open && inline}
		<p class="mt-1 rounded-xl border border-dashed border-slate-200 px-3 py-2 text-sm text-slate-400 dark:border-neutral-700 dark:text-neutral-500">No matches</p>
	{/if}
</div>
