// The note editor's extensions: markdown (GFM), history, keys, live preview, tags and
// wikilinks, the `/`, `#` and `[[` menus, and theme.
// No basicSetup: only what a notes editor needs.
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { EditorState, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, keymap, placeholder } from '@codemirror/view';
import { blockPreview } from '#lib/editor/blocks.js';
import { focusTracking } from '#lib/editor/focus.js';
import { noteLinks, type LinkFollowScope } from '#lib/editor/links.js';
import { livePreview } from '#lib/editor/live-preview.js';
import { slashMenu } from '#lib/editor/slash.js';
import { noteTheme } from '#lib/editor/theme.js';
import type { LinkFollow } from '#lib/note-links.js';

/**
 * Extensions shared by every note's state; `onDocChange` gets the new body after each edit,
 * `onFollow` a ⌘-clicked tag, chip or wikilink.
 */
export function noteExtensions(
	onDocChange: (body: string) => void,
	onFollow: (link: LinkFollow) => void,
	followScope: LinkFollowScope = 'all'
): Extension[] {
	return [
		history(),
		drawSelection(),
		EditorView.lineWrapping,
		// markdown() also installs markdownKeymap: list continuation, empty item exits.
		// Fenced code is parsed with its language, loaded on first use (one chunk each).
		// No HTML tag completion: the `/`, `#` and `[[` menus are the only completion sources.
		markdown({ base: markdownLanguage, codeLanguages: languages, completeHTMLTags: false }),
		keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
		EditorView.contentAttributes.of({ spellcheck: 'true', autocorrect: 'off', autocapitalize: 'off' }),
		placeholder('Start writing…'),
		focusTracking,
		livePreview,
		noteLinks(onFollow, followScope),
		blockPreview,
		slashMenu,
		noteTheme,
		EditorView.updateListener.of((update) => {
			if (update.docChanged) onDocChange(update.state.doc.toString());
		})
	];
}

export function createNoteState(body: string, extensions: Extension[]): EditorState {
	return EditorState.create({ doc: body, extensions });
}
