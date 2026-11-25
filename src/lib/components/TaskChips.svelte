<script lang="ts">
	import type { Chip, ChipIcon, ChipTone } from '#lib/tasks.js';
	import Bell from '@lucide/svelte/icons/bell';
	import BellRing from '@lucide/svelte/icons/bell-ring';
	import CalendarDays from '@lucide/svelte/icons/calendar-days';
	import Laptop from '@lucide/svelte/icons/laptop';
	import Link2 from '@lucide/svelte/icons/link-2';
	import Repeat from '@lucide/svelte/icons/repeat';
	import FileText from '@lucide/svelte/icons/file-text';

	// Row and quick-add preview chips (labels from `taskChips` / `planChips`). Rows use the
	// bare look (icon and colored text); the quick-add preview keeps pills.
	let { chips, variant = 'pill' }: { chips: Chip[]; variant?: 'pill' | 'bare' } = $props();

	// Full class names, so Tailwind's content scan keeps them.
	const TONES: Record<ChipTone, string> = {
		red: 'task-chip-red',
		amber: 'task-chip-amber',
		sky: 'task-chip-sky',
		violet: 'task-chip-violet',
		slate: 'task-chip-slate'
	};
	const BARE_TONES: Record<ChipTone, string> = {
		red: 'task-chip-bare-red',
		amber: 'task-chip-bare-amber',
		sky: 'task-chip-bare-sky',
		violet: 'task-chip-bare-violet',
		slate: 'task-chip-bare-slate'
	};
	const ICONS: Record<ChipIcon, typeof Bell> = {
		cal: CalendarDays,
		bell: Bell,
		ring: BellRing,
		repeat: Repeat,
		link: Link2,
		note: FileText,
		mac: Laptop
	};
</script>

{#each chips as chip, i (i)}
	{@const Icon = ICONS[chip.icon]}
	{#if variant === 'bare'}
		<span class="task-chip-bare {BARE_TONES[chip.tone]}" title={chip.title}>
			<Icon size={13} class="shrink-0" aria-hidden="true" /><span class="truncate">{chip.label}</span>
		</span>
	{:else}
		<span class="task-chip {TONES[chip.tone]}" title={chip.title}>
			<Icon size={12} class="shrink-0" aria-hidden="true" /><span class="truncate">{chip.label}</span>
		</span>
	{/if}
{/each}
