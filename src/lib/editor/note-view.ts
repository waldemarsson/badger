// One EditorView that shows one note at a time. Each note keeps its own EditorState
// (unsaved edits, undo history) and scroll position while the view lives.
import type { EditorState, Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { syncFocus } from '#lib/editor/focus.js';
import { emptyLinkData, linkDataField, setLinkData, type LinkData } from '#lib/editor/links.js';
import { createNoteState, noteExtensions } from '#lib/editor/setup.js';
import type { LinkFollow } from '#lib/note-links.js';

export class NoteView {
	private readonly view: EditorView;
	private readonly extensions: Extension[];
	private readonly states = new Map<number, EditorState>();
	private readonly scrolls = new Map<number, number>();
	private currentId: number | null = null;
	/** What links resolve to; pushed into each note's state when it is shown. */
	private links: LinkData = emptyLinkData;

	constructor(
		parent: HTMLElement,
		onchange: (id: number, body: string) => void,
		onfollow: (link: LinkFollow) => void
	) {
		this.extensions = noteExtensions((body) => {
			if (this.currentId !== null) onchange(this.currentId, body);
		}, onfollow);
		this.view = new EditorView({ parent, state: createNoteState('', this.extensions) });
	}

	/** Updates link resolution and `#` suggestions (redraws chips and wikilinks). */
	setLinks(links: LinkData): void {
		this.links = links;
		this.syncLinks();
	}

	/** Shows note `id`. Its cached state wins over `body`. */
	open(id: number, body: string): void {
		if (id === this.currentId) return;
		this.stash();
		this.currentId = id;
		this.view.setState(this.states.get(id) ?? createNoteState(body, this.extensions));
		syncFocus(this.view);
		this.syncLinks();
		const top = this.scrolls.get(id) ?? 0;
		requestAnimationFrame(() => {
			if (this.currentId === id) this.view.scrollDOM.scrollTop = top;
		});
	}

	/** Shows nothing; the current note's state stays cached. */
	close(): void {
		this.stash();
		this.reset();
	}

	/** Drops a note's cached state (deleted, or its body was rewritten by a rename). */
	forget(id: number): void {
		this.states.delete(id);
		this.scrolls.delete(id);
		if (id === this.currentId) this.reset();
	}

	focus(): void {
		this.view.focus();
	}

	/** The open note's current text, including edits not saved yet. */
	text(): string {
		return this.view.state.doc.toString();
	}

	destroy(): void {
		this.view.destroy();
	}

	private reset(): void {
		this.currentId = null;
		this.view.setState(createNoteState('', this.extensions));
	}

	/** A cached state may hold older link data. */
	private syncLinks(): void {
		if (this.view.state.field(linkDataField) !== this.links) this.view.dispatch({ effects: setLinkData.of(this.links) });
	}

	private stash(): void {
		if (this.currentId === null) return;
		this.states.set(this.currentId, this.view.state);
		this.scrolls.set(this.currentId, this.view.scrollDOM.scrollTop);
	}
}
