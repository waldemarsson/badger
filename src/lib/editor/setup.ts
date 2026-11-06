// The note editor's extensions: markdown (GFM), history, keys, live preview and theme.
// No basicSetup: only what a notes editor needs.
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { EditorState, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, keymap, placeholder } from '@codemirror/view';
import { blockPreview } from '#lib/editor/blocks.js';
import { focusTracking } from '#lib/editor/focus.js';
import { livePreview } from '#lib/editor/live-preview.js';
import { noteTheme } from '#lib/editor/theme.js';

/** Extensions shared by every note's state; `onDocChange` gets the new body after each edit. */
export function noteExtensions(onDocChange: (body: string) => void): Extension[] {
	return [
		history(),
		drawSelection(),
		EditorView.lineWrapping,
		// markdown() also installs markdownKeymap: list continuation, empty item exits.
		markdown({ base: markdownLanguage }),
		keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
		EditorView.contentAttributes.of({ spellcheck: 'true', autocorrect: 'off', autocapitalize: 'off' }),
		placeholder('Start writing…'),
		focusTracking,
		livePreview,
		blockPreview,
		noteTheme,
		EditorView.updateListener.of((update) => {
			if (update.docChanged) onDocChange(update.state.doc.toString());
		})
	];
}

export function createNoteState(body: string, extensions: Extension[]): EditorState {
	return EditorState.create({ doc: body, extensions });
}
