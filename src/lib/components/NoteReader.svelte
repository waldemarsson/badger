<script lang="ts">
	// Read view for the Notes page: HTML rendered (and locked down) by Rust's
	// render_markdown, ```calc blocks shown as expression/result rows (a click on a result
	// copies it), code blocks highlighted with the editor's colors, and links
	// opened in the default browser instead of navigating the WebView. Tags, chips and
	// wikilinks (data-tag / data-note, see markdown.rs) are followed in the app, and
	// `#heading` links scroll to that heading.
	import { openUrl } from '@tauri-apps/plugin-opener';
	import { renderCalcBlocks } from '#lib/calc-reader.js';
	import { highlightCalc, highlightCodeBlocks } from '#lib/editor/highlight-code.js';
	import { isOpenableUrl } from '#lib/links.js';
	import type { LinkFollow } from '#lib/note-links.js';

	let {
		html,
		onerror,
		onfollow,
		class: className = ''
	}: {
		/** Output of render_markdown; empty for an empty note. */
		html: string;
		/** A link could not be opened. */
		onerror: (message: string) => void;
		/** A tag, chip or wikilink was clicked. */
		onfollow: (link: LinkFollow) => void;
		class?: string;
	} = $props();

	let host: HTMLElement;

	$effect(() => {
		void html; // re-render calc blocks and re-highlight after every render
		let live = true;
		renderCalcBlocks(host, { highlight: highlightCalc }); // before highlighting: it replaces their <code>
		void highlightCodeBlocks(host, () => live);
		return () => {
			live = false;
		};
	});

	function onclick(e: MouseEvent): void {
		const copy = e.target instanceof Element ? e.target.closest<HTMLElement>('[data-calc-copy]') : null;
		if (copy) return copyResult(copy);
		const link = e.target instanceof Element ? e.target.closest('a') : null;
		if (!link) return;
		e.preventDefault(); // the WebView never navigates away from the app
		const tag = link.getAttribute('data-tag');
		if (tag) return onfollow({ kind: 'tag', tag });
		const title = link.getAttribute('data-note');
		if (title !== null) {
			const id = Number(link.getAttribute('data-note-id'));
			return onfollow({ kind: 'note', title, id: Number.isInteger(id) && id > 0 ? id : null });
		}
		const href = link.getAttribute('href') ?? '';
		if (href.startsWith('#')) return scrollToHeading(href.slice(1));
		if (!isOpenableUrl(href)) return;
		openUrl(href).catch(() => onerror(`Couldn't open ${href}`));
	}

	/** A calc result: copied as shown, without the ⚠. */
	function copyResult(button: HTMLElement): void {
		navigator.clipboard.writeText(button.dataset.calcCopy ?? '').then(
			() => {
				button.classList.add('calc-copied');
				setTimeout(() => button.classList.remove('calc-copied'), 900);
			},
			() => onerror("Couldn't copy the result")
		);
	}

	/** `[text](#heading)`: render_markdown gives headings matching ids (href percent-encoded). */
	function scrollToHeading(fragment: string): void {
		let id = fragment;
		try {
			id = decodeURIComponent(fragment);
		} catch {
			// malformed escape: look it up as written
		}
		const target = id ? host.querySelector(`[id="${CSS.escape(id)}"]`) : null;
		if (!target) return onerror(`No heading “${id}” in this note`);
		target.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	export function focus(): void {
		host.focus({ preventScroll: true });
	}
</script>

<!-- Clicks are only handled on links and calc results, which get keyboard activation (Enter) for free. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
<article bind:this={host} class="note-reader {className}" tabindex="0" aria-label="Note" {onclick}>
	{#if html}
		<!-- eslint-disable-next-line svelte/no-at-html-tags -- Rust escapes raw HTML and drops unsafe links and images -->
		<div class="note-prose">{@html html}</div>
	{:else}
		<p class="note-reader-empty">Empty note</p>
	{/if}
</article>
