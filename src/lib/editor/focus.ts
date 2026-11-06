// Tracks editor focus in the state, so decorations can render everything (no revealed
// markers) while the editor is not focused.
import { StateEffect, StateField, type EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

export const setFocused = StateEffect.define<boolean>();

export const focusField = StateField.define<boolean>({
	create: () => false,
	update(value, tr) {
		for (const effect of tr.effects) if (effect.is(setFocused)) value = effect.value;
		return value;
	}
});

export const focusTracking = [focusField, EditorView.focusChangeEffect.of((_state, focusing) => setFocused.of(focusing))];

/** After `view.setState`, the new state's focus flag may be stale; this corrects it. */
export function syncFocus(view: EditorView): void {
	if (view.state.field(focusField, false) !== view.hasFocus) view.dispatch({ effects: setFocused.of(view.hasFocus) });
}

/** Line numbers touched by the selection while focused; markers on these lines show as source. */
export function revealedLines(state: EditorState): Set<number> {
	const lines = new Set<number>();
	if (!state.field(focusField, false)) return lines;
	for (const range of state.selection.ranges) {
		const first = state.doc.lineAt(range.from).number;
		const last = state.doc.lineAt(range.to).number;
		for (let n = first; n <= last; n++) lines.add(n);
	}
	return lines;
}

/** True when the selection (focused) touches [from, to], boundaries included. */
export function selectionTouches(state: EditorState, from: number, to: number): boolean {
	if (!state.field(focusField, false)) return false;
	return state.selection.ranges.some((r) => r.from <= to && r.to >= from);
}

/** True when the selection (focused) touches any line in [from, to]. */
export function selectionTouchesLines(state: EditorState, from: number, to: number): boolean {
	return selectionTouches(state, state.doc.lineAt(from).from, state.doc.lineAt(to).to);
}
