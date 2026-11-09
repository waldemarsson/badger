<script lang="ts">
	// CodeMirror host for the Notes page and the scratchpad. The host drives it through bind:this
	// (open/close/forget/focus/setLinks); each note keeps its own undo history while mounted.
	import { onMount } from 'svelte';
	import type { LinkData } from '#lib/editor/links.js';
	import { NoteView } from '#lib/editor/note-view.js';
	import type { LinkFollow } from '#lib/note-links.js';

	let {
		onchange,
		onfollow,
		class: className = ''
	}: {
		/** Called after every edit with the note id and its full body. */
		onchange: (id: number, body: string) => void;
		/** A tag, chip or wikilink was ⌘-clicked. */
		onfollow: (link: LinkFollow) => void;
		class?: string;
	} = $props();

	let links: LinkData | undefined;

	let host: HTMLDivElement;
	let editor: NoteView | undefined;

	/** Shows note `id`; its cached state (unsaved edits, undo history) wins over `body`. */
	export function open(id: number, body: string): void {
		editor?.open(id, body);
	}

	/** Shows nothing (no note selected). */
	export function close(): void {
		editor?.close();
	}

	/** Drops a note's cached state (deleted, or rewritten by a rename). */
	export function forget(id: number): void {
		editor?.forget(id);
	}

	/** What wikilinks and chips resolve to, and the `#` suggestions. */
	export function setLinks(data: LinkData): void {
		links = data;
		editor?.setLinks(data);
	}

	export function focus(): void {
		editor?.focus();
	}

	/** Turns typing on or off (the scratchpad locks it while Save as note commits). */
	export function setEditable(editable: boolean): void {
		editor?.setEditable(editable);
	}

	/** Focuses with the caret at the end of the text. */
	export function focusEnd(): void {
		editor?.focusEnd();
	}

	/** The open note's current text, including edits not saved yet. */
	export function text(): string {
		return editor?.text() ?? '';
	}

	onMount(() => {
		editor = new NoteView(
			host,
			(id, body) => onchange(id, body),
			(link) => onfollow(link)
		);
		if (links) editor.setLinks(links);
		return () => {
			editor?.destroy();
			editor = undefined;
		};
	});
</script>

<div bind:this={host} class="note-editor {className}"></div>
