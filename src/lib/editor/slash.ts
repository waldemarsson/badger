// The `/` snippet menu. Only this source runs (`override`), so there is no word or
// language completion anywhere. The menu's own keys (arrows, Enter, Esc) outrank the
// markdown keymap while it is open; snippet Tab/Shift-Tab outrank indentWithTab only while
// a snippet is active. The rules and insertion live in src/lib/slash-menu.ts.
import {
	autocompletion,
	completionStatus,
	currentCompletions,
	selectedCompletionIndex,
	setSelectedCompletion,
	type Completion,
	type CompletionContext,
	type CompletionResult
} from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';
import { ViewPlugin, type EditorView, type ViewUpdate } from '@codemirror/view';
import { applySlashItem, isCodeAt, slashItemsAt, slashTrigger } from '#lib/slash-menu.js';

function slashSource(context: CompletionContext): CompletionResult | null {
	const { state, pos } = context;
	const line = state.doc.lineAt(pos);
	const trigger = slashTrigger(line.text.slice(0, pos - line.from));
	if (!trigger) return null;
	const slash = line.from + trigger.from;
	const tree = syntaxTree(state);
	if (isCodeAt(tree, slash)) return null;

	const options: Completion[] = slashItemsAt(tree, slash).map((item, i) => ({
		label: item.label,
		detail: item.detail,
		boost: -i, // keep the menu order for an empty query
		// `from` is just after the `/`: the `/` is replaced too.
		apply: (view: EditorView, completion: Completion, from: number, to: number) =>
			applySlashItem(view, item, from - 1, to, new Date(), completion)
	}));
	return { from: slash + 1, options, validFor: /^[\p{L}\p{N}]*$/u };
}

/**
 * Nothing is preselected for a bare `/`, so Enter keeps its normal meaning (new line, list
 * continuation) and a line ending in ` /` stays as typed; arrows still select. Once a filter
 * is typed, the first match is preselected so Enter picks it.
 */
const preselect = ViewPlugin.define((view) => ({
	update(update: ViewUpdate) {
		const { state } = update;
		if (completionStatus(state) !== 'active') return;
		const options = currentCompletions(state);
		// Neither the text nor the options changed (e.g. arrow keys): leave the selection.
		if (!options.length || (!update.docChanged && options === currentCompletions(update.startState))) return;
		const head = state.selection.main.head;
		const line = state.doc.lineAt(head);
		const trigger = slashTrigger(line.text.slice(0, head - line.from));
		if (!trigger) return;
		const want = trigger.query ? 0 : -1;
		if ((selectedCompletionIndex(state) ?? -1) === want) return;
		// No dispatching inside an update.
		queueMicrotask(() => {
			if (view.state === state) view.dispatch({ effects: setSelectedCompletion(want) });
		});
	}
}));

export const slashMenu = [
	autocompletion({
		override: [slashSource],
		icons: false,
		closeOnBlur: true,
		selectOnOpen: false
	}),
	preselect
];
