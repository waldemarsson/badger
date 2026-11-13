<script lang="ts">
	// CodeMirror host for the Notes page and the scratchpad. The host drives it through bind:this
	// (open/close/forget/focus/setLinks); each note keeps its own undo history while mounted.
	import { onMount } from 'svelte';
	import type { LinkData, LinkFollowScope } from '#lib/editor/links.js';
	import { NoteView } from '#lib/editor/note-view.js';
	import type { JsonFormatMode } from '#lib/json-format.js';
	import type { LinkFollow } from '#lib/note-links.js';
	import type { NoteKind } from '#lib/types.js';

	let {
		onchange,
		onfollow,
		onformat,
		followScope = 'all',
		class: className = ''
	}: {
		/** Called after every edit with the note id and its full body. */
		onchange: (id: number, body: string) => void;
		/** A tag, chip or wikilink was ⌘-clicked. */
		onfollow: (link: LinkFollow) => void;
		/** ⇧⌥F in a JSON note: an "Invalid JSON: …" message, or null when it formatted. */
		onformat?: (error: string | null) => void;
		/** What ⌘-click can follow (read once, at mount): everything, or resolved wikilinks only. */
		followScope?: LinkFollowScope;
		class?: string;
	} = $props();

	let links: LinkData | undefined;

	let host: HTMLDivElement;
	let editor: NoteView | undefined;

	/** Shows note `id` in `kind`'s mode; its cached state (unsaved edits, undo history) wins over `body`. */
	export function open(id: number, body: string, kind: NoteKind = 'markdown'): void {
		editor?.open(id, body, kind);
	}

	/** Switches the shown note between the markdown and JSON editors (the text stays). */
	export function setKind(kind: NoteKind): void {
		editor?.setKind(kind);
	}

	/** Formats or minifies the shown JSON note; returns "Invalid JSON: …" or null. */
	export function format(mode: JsonFormatMode): string | null {
		return editor?.format(mode) ?? null;
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
			(link) => onfollow(link),
			followScope,
			(error) => onformat?.(error)
		);
		if (links) editor.setLinks(links);
		return () => {
			editor?.destroy();
			editor = undefined;
		};
	});
</script>

<div bind:this={host} class="note-editor {className}"></div>
