<script lang="ts">
	// CodeMirror host for the Notes page. The page drives it through bind:this
	// (open/close/forget/focus); each note keeps its own undo history while mounted.
	import { onMount } from 'svelte';
	import { NoteView } from '#lib/editor/note-view.js';

	let {
		onchange,
		class: className = ''
	}: {
		/** Called after every edit with the note id and its full body. */
		onchange: (id: number, body: string) => void;
		class?: string;
	} = $props();

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

	/** Drops a deleted note's cached state. */
	export function forget(id: number): void {
		editor?.forget(id);
	}

	export function focus(): void {
		editor?.focus();
	}

	onMount(() => {
		editor = new NoteView(host, (id, body) => onchange(id, body));
		return () => {
			editor?.destroy();
			editor = undefined;
		};
	});
</script>

<div bind:this={host} class="note-editor {className}"></div>
