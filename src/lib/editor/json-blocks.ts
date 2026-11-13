// ```json blocks in markdown notes, the scratchpad and handovers: while the caret is in
// one, "Format" and "Minify" chips sit on its opening fence line; ⇧⌥F formats the block
// under the caret (and does nothing elsewhere, so the key falls through). Invalid JSON
// shows an inline error next to the chips and changes nothing. Blocks are not linted.
// Finding blocks and the edit live in src/lib/json-format.ts.
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { StateEffect, StateField, type EditorState } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, keymap, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { focusField } from '#lib/editor/focus.js';
import { findJsonBlock, jsonBlockEdit, jsonErrorMessage, type JsonFormatMode } from '#lib/json-format.js';

interface BlockError {
	/** The FencedCode start of the block it belongs to. */
	from: number;
	message: string;
}

const setBlockError = StateEffect.define<BlockError | null>();

/** The last refused Format/Minify; any edit clears it. */
const blockErrorField = StateField.define<BlockError | null>({
	create: () => null,
	update(value, tr) {
		for (const effect of tr.effects) if (effect.is(setBlockError)) return effect.value;
		return tr.docChanged ? null : value;
	}
});

/**
 * Formats or minifies the json block at `pos` (default: the caret) in one transaction.
 * False when `pos` is not in a closed ```json block, or the editor is read-only.
 */
export function formatJsonBlock(view: EditorView, mode: JsonFormatMode, pos = view.state.selection.main.head): boolean {
	const { state } = view;
	if (state.readOnly) return false;
	const tree = ensureSyntaxTree(state, state.doc.length, 200) ?? syntaxTree(state);
	const block = findJsonBlock(tree, state.doc, pos);
	if (!block) return false;
	const edit = jsonBlockEdit(block, mode);
	if (!edit.ok) {
		view.dispatch({ effects: setBlockError.of({ from: block.from, message: jsonErrorMessage(edit) }) });
		return true;
	}
	if (state.sliceDoc(edit.from, edit.to) === edit.insert) {
		if (state.field(blockErrorField)) view.dispatch({ effects: setBlockError.of(null) });
		return true;
	}
	view.dispatch({
		changes: { from: edit.from, to: edit.to, insert: edit.insert },
		effects: setBlockError.of(null),
		userEvent: 'input.format'
	});
	return true;
}

class ChipsWidget extends WidgetType {
	constructor(
		readonly blockFrom: number,
		readonly error: string | null
	) {
		super();
	}

	eq(other: ChipsWidget): boolean {
		return other.blockFrom === this.blockFrom && other.error === this.error;
	}

	toDOM(view: EditorView): HTMLElement {
		const wrap = document.createElement('span');
		wrap.className = 'cm-json-chips';
		const chip = (label: string, title: string, mode: JsonFormatMode) => {
			const button = document.createElement('button');
			button.type = 'button';
			button.className = 'cm-json-chip';
			button.textContent = label;
			button.title = title;
			// Keep focus and the caret in the editor: the chips only show while it is focused.
			button.addEventListener('mousedown', (e) => e.preventDefault());
			button.addEventListener('click', (e) => {
				e.preventDefault();
				formatJsonBlock(view, mode, this.blockFrom);
				view.focus();
			});
			wrap.append(button);
		};
		chip('Format', 'Format JSON (⇧⌥F)', 'format');
		chip('Minify', 'Minify JSON', 'minify');
		if (this.error) {
			const error = document.createElement('span');
			error.className = 'cm-json-chip-error';
			error.setAttribute('role', 'alert');
			error.textContent = this.error;
			wrap.append(error);
		}
		return wrap;
	}

	ignoreEvent(): boolean {
		return true;
	}
}

function chips(state: EditorState): DecorationSet {
	if (!state.field(focusField, false)) return Decoration.none;
	const block = findJsonBlock(syntaxTree(state), state.doc, state.selection.main.head);
	if (!block) return Decoration.none;
	const error = state.field(blockErrorField);
	const widget = new ChipsWidget(block.from, error?.from === block.from ? error.message : null);
	return Decoration.set([Decoration.widget({ widget, side: 1 }).range(block.fenceLineEnd)]);
}

const chipsPlugin = ViewPlugin.fromClass(
	class {
		decorations: DecorationSet;
		constructor(view: EditorView) {
			this.decorations = chips(view.state);
		}
		update(update: ViewUpdate) {
			const { state, startState } = update;
			if (
				update.docChanged ||
				update.selectionSet ||
				syntaxTree(state) !== syntaxTree(startState) ||
				state.field(focusField, false) !== startState.field(focusField, false) ||
				state.field(blockErrorField) !== startState.field(blockErrorField)
			) {
				this.decorations = chips(state);
			}
		}
	},
	{ decorations: (v) => v.decorations }
);

export const jsonBlocks = [
	blockErrorField,
	chipsPlugin,
	keymap.of([{ key: 'Shift-Alt-f', preventDefault: false, run: (view) => formatJsonBlock(view, 'format') }])
];
