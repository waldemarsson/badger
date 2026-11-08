<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	// Week/Month switch shared by the report pages (one "Reports" nav tab covers both).
	const options = [
		{ href: '/reports/weekly', label: 'Week' },
		{ href: '/reports/monthly', label: 'Month' },
	] as const;
</script>

<nav class="segmented gap-0.5 p-0.5" aria-label="Report period">
	{#each options as option (option.href)}
		<a
			href={resolve(option.href)}
			class="segmented-option px-3 py-1.5 outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
			class:segmented-option-active={page.url.pathname === option.href}
			aria-current={page.url.pathname === option.href ? 'page' : undefined}
		>{option.label}</a>
	{/each}
</nav>
