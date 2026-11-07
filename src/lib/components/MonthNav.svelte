<script lang="ts">
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { MONTH_NAMES } from '#lib/travel.js';

	// Month navigator: prev/next buttons plus a jump selector (current year ± 3).
	let {
		year = $bindable(),
		month = $bindable(), // 1-based
		disabled = false,
		onchange,
	}: { year: number; month: number; disabled?: boolean; onchange: () => void } = $props();

	const currentYear = new Date().getFullYear();
	const options: { value: string; label: string }[] = [];
	for (let y = currentYear - 3; y <= currentYear + 3; y++) {
		for (let m = 1; m <= 12; m++) {
			options.push({ value: `${y}-${String(m).padStart(2, '0')}`, label: `${MONTH_NAMES[m - 1]} ${y}` });
		}
	}

	const value = $derived(`${year}-${String(month).padStart(2, '0')}`);

	function shift(delta: number) {
		const d = new Date(year, month - 1 + delta, 1);
		year = d.getFullYear();
		month = d.getMonth() + 1;
		onchange();
	}

	function select(e: Event) {
		const [y, m] = (e.target as HTMLSelectElement).value.split('-').map(Number);
		year = y;
		month = m;
		onchange();
	}
</script>

<div class="flex items-center gap-3">
	<button class="btn-icon-secondary" onclick={() => shift(-1)} {disabled} title="Previous month">
		<ChevronLeft size={16} />
	</button>
	<span class="min-w-[8rem] text-center text-sm font-semibold text-slate-700 dark:text-neutral-300">
		{MONTH_NAMES[month - 1]} {year}
	</span>
	<button class="btn-icon-secondary" onclick={() => shift(1)} {disabled} title="Next month">
		<ChevronRight size={16} />
	</button>
</div>
<select class="form-select w-auto" {value} onchange={select} aria-label="Jump to month">
	{#each options as opt (opt.value)}
		<option value={opt.value}>{opt.label}</option>
	{/each}
</select>
