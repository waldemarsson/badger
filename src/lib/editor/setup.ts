// The note editor's extensions: history, keys, theme and link data shared by every note,
// plus one mode per note kind. Markdown mode: markdown (GFM), live preview, tags and
// wikilinks, the `/`, `#` and `[[` menus, and Format/Minify for ```json blocks. JSON mode:
// see json-mode.ts. No basicSetup: only what a notes editor needs.
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { EditorState, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, keymap, placeholder } from '@codemirror/view';
import { blockPreview } from '#lib/editor/blocks.js';
import { focusTracking } from '#lib/editor/focus.js';
import { jsonBlocks } from '#lib/editor/json-blocks.js';
import { formatReport, jsonMode } from '#lib/editor/json-mode.js';
import { linkState, noteLinks, type LinkFollowScope } from '#lib/editor/links.js';
import { livePreview } from '#lib/editor/live-preview.js';
import { slashMenu } from '#lib/editor/slash.js';
import { noteTheme } from '#lib/editor/theme.js';
import type { LinkFollow } from '#lib/note-links.js';
import type { NoteKind } from '#lib/types.js';

/**
 * Extensions shared by every note's state, whatever its mode; `onDocChange` gets the new
 * body after each edit, `onFormat` the outcome of ⇧⌥F in JSON mode.
 */
export function noteExtensions(
	onDocChange: (body: string) => void,
	followScope: LinkFollowScope = 'all',
	onFormat: (error: string | null) => void = () => {}
): Extension[] {
	return [
		history(),
		drawSelection(),
		EditorView.lineWrapping,
		keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
		focusTracking,
		linkState(followScope),
		formatReport.of(onFormat),
		noteTheme,
		EditorView.updateListener.of((update) => {
			if (update.docChanged) onDocChange(update.state.doc.toString());
		})
	];
}

/** Markdown mode; `onFollow` gets a ⌘-clicked tag, chip or wikilink. */
export function markdownMode(onFollow: (link: LinkFollow) => void): Extension {
	return [
		// markdown() also installs markdownKeymap: list continuation, empty item exits.
		// Fenced code is parsed with its language, loaded on first use (one chunk each).
		// No HTML tag completion: the `/`, `#` and `[[` menus are the only completion sources.
		markdown({ base: markdownLanguage, codeLanguages: languages, completeHTMLTags: false }),
		EditorView.contentAttributes.of({ spellcheck: 'true', autocorrect: 'off', autocapitalize: 'off' }),
		placeholder('Start writing…'),
		livePreview,
		noteLinks(onFollow),
		blockPreview,
		slashMenu,
		jsonBlocks
	];
}

/** One extension per note kind, each built once so a state's mode can be compared. */
export function noteModes(onFollow: (link: LinkFollow) => void): Record<NoteKind, Extension> {
	return { markdown: markdownMode(onFollow), json: jsonMode() };
}

export function createNoteState(body: string, extensions: Extension[]): EditorState {
	return EditorState.create({ doc: body, extensions });
}
