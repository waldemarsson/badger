<script lang="ts">
	// Read view for the Notes page: HTML rendered (and locked down) by Rust's
	// render_markdown, code blocks highlighted with the editor's colors, and links
	// opened in the default browser instead of navigating the WebView.
	import { openUrl } from '@tauri-apps/plugin-opener';
	import { highlightCodeBlocks } from '#lib/editor/highlight-code.js';
	import { isOpenableUrl } from '#lib/links.js';

	let {
		html,
		onerror,
		class: className = ''
	}: {
		/** Output of render_markdown; empty for an empty note. */
		html: string;
		/** A link could not be opened. */
		onerror: (message: string) => void;
		class?: string;
	} = $props();

	let host: HTMLElement;

	$effect(() => {
		void html; // re-highlight after every render
		let live = true;
		void highlightCodeBlocks(host, () => live);
		return () => {
			live = false;
		};
	});

	function onclick(e: MouseEvent): void {
		const link = e.target instanceof Element ? e.target.closest('a') : null;
		if (!link) return;
		e.preventDefault(); // the WebView never navigates away from the app
		const href = link.getAttribute('href') ?? '';
		if (!isOpenableUrl(href)) return;
		openUrl(href).catch(() => onerror(`Couldn't open ${href}`));
	}

	export function focus(): void {
		host.focus({ preventScroll: true });
	}
</script>

<!-- Clicks are only handled on links, which get keyboard activation (Enter) for free. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
<article bind:this={host} class="note-reader {className}" tabindex="0" aria-label="Note" {onclick}>
	{#if html}
		<!-- eslint-disable-next-line svelte/no-at-html-tags -- Rust escapes raw HTML and drops unsafe links and images -->
		<div class="note-prose">{@html html}</div>
	{:else}
		<p class="note-reader-empty">Empty note</p>
	{/if}
</article>
