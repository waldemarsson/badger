// JSON mode of the note editor (JSON notes and the scratchpad in JSON): JSON highlighting,
// folding, bracket matching and closing, a parse-error lint, and ⇧⌥F to format the whole
// document. No markdown preview, no `/`, `#` or `[[` menus. The formatter is the
// Lezer-tree printer in src/lib/json-format.ts.
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { json } from '@codemirror/lang-json';
import { bracketMatching, foldGutter, foldKeymap, syntaxHighlighting, syntaxTree } from '@codemirror/language';
import { linter, lintGutter, type Diagnostic } from '@codemirror/lint';
import { Facet, Prec, type Extension } from '@codemirror/state';
import { EditorView, keymap, placeholder } from '@codemirror/view';
import { codeHighlight } from '#lib/editor/theme.js';
import { changedSpan, formatJsonAs, jsonErrorMessage, jsonStateLintError, type JsonFormatMode } from '#lib/json-format.js';

/** Where ⇧⌥F reports its outcome (an "Invalid JSON: …" message, or null when it worked). */
export const formatReport = Facet.define<(error: string | null) => void, ((error: string | null) => void) | null>({
	combine: (values) => values[0] ?? null
});

/**
 * Formats or minifies the whole document in one transaction (one undo step). Only the
 * span that differs is replaced, so the caret and folds outside it stay put. Returns the
 * error message when the text is not valid JSON (nothing changes), else null. Does
 * nothing in a read-only editor.
 */
export function formatDocument(view: EditorView, mode: JsonFormatMode): string | null {
	if (view.state.readOnly) return null;
	const text = view.state.doc.toString();
	const result = formatJsonAs(text, mode);
	if (!result.ok) return jsonErrorMessage(result);
	const changes = changedSpan(text, result.text);
	if (changes) view.dispatch({ changes, userEvent: 'input.format' });
	return null;
}

/** The longest the linter's parse may hold the thread at a time. */
const LINT_PARSE_SLICE_MS = 50;

/**
 * At most one diagnostic, at the first error node of the editor's own (incremental) syntax
 * tree, where Format reports it. A fresh parse per edit took seconds on multi-MB notes, so
 * this finishes the editor's parse in short slices, yielding in between, and lints nothing
 * when the doc changes meanwhile. Not `jsonParseLinter`: it positions errors from
 * `JSON.parse` messages, which JavaScriptCore words differently.
 */
async function jsonDiagnostics(view: EditorView): Promise<Diagnostic[]> {
	const error = await jsonStateLintError(() => (view.dom.isConnected ? view.state : null), LINT_PARSE_SLICE_MS);
	return error ? [{ ...error, severity: 'error' }] : [];
}

export function jsonMode(): Extension {
	return [
		json(),
		syntaxHighlighting(codeHighlight),
		foldGutter(),
		bracketMatching(),
		closeBrackets(),
		Prec.high(
			keymap.of([
				...closeBracketsKeymap,
				...foldKeymap,
				{
					key: 'Shift-Alt-f',
					preventDefault: true,
					run(view) {
						const error = formatDocument(view, 'format');
						view.state.facet(formatReport)?.(error);
						return true;
					}
				}
			])
		),
		linter(jsonDiagnostics, {
			needsRefresh: (update) => syntaxTree(update.state) !== syntaxTree(update.startState)
		}),
		lintGutter(),
		EditorView.editorAttributes.of({ class: 'cm-json-mode' }),
		EditorView.contentAttributes.of({ spellcheck: 'false', autocorrect: 'off', autocapitalize: 'off' }),
		placeholder('Paste or type JSON…')
	];
}
