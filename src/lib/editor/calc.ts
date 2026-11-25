// ```calc results in the markdown editor: each value, sum or error line gets its result
// right-aligned at the line's end, greyed out (sums bolder, errors red). A widget, never
// text: the note body is untouched. Clicking a result copies it. Variables span the whole
// note, so every edit re-evaluates every block (linear in calc lines; notes without a calc
// fence only pay for a scan of their lines). The evaluator is src/lib/calc.ts.
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import type { EditorState, Range } from '@codemirror/state';
import { Decoration, ViewPlugin, WidgetType, type DecorationSet, type EditorView, type ViewUpdate } from '@codemirror/view';
import { calcBlocks, evaluateBlocks, hasCalcFence, type CalcLine } from '#lib/calc.js';

/** How long one evaluation may spend parsing the rest of the note (ms). */
const PARSE_BUDGET = 20;
const FLASH_MS = 900;

export interface CalcResultAt {
	/** The document line's start and end. */
	from: number;
	to: number;
	result: CalcLine;
}

/** Every calc line that shows a result (value, sum or error), in document order. */
export function calcResults(state: EditorState): CalcResultAt[] {
	if (!hasCalcFence(state.doc.iterLines())) return [];
	// Scope is note-wide, so this needs the whole tree, not just the viewport's.
	const tree = ensureSyntaxTree(state, state.doc.length, PARSE_BUDGET) ?? syntaxTree(state);
	const blocks = calcBlocks(tree, state.doc);
	if (!blocks.length) return [];
	const results = evaluateBlocks(blocks.map((block) => block.lines.map((line) => line.text)));
	const out: CalcResultAt[] = [];
	blocks.forEach((block, b) => {
		block.lines.forEach((line, i) => {
			const result = results[b][i];
			if (result.text) out.push({ from: line.from, to: line.to, result });
		});
	});
	return out;
}

const KIND_CLASS: Partial<Record<CalcLine['kind'], string>> = {
	sum: 'cm-calc-result cm-calc-result-sum',
	error: 'cm-calc-result cm-calc-result-error'
};

export class CalcResultWidget extends WidgetType {
	readonly result: CalcLine;

	constructor(result: CalcLine) {
		super();
		this.result = result;
	}

	/** Same text, same DOM: results that didn't change survive every keystroke. */
	eq(other: CalcResultWidget): boolean {
		return other.result.text === this.result.text && other.result.kind === this.result.kind;
	}

	toDOM(): HTMLElement {
		const { kind, text, copy } = this.result;
		// Zero-width anchor at the line end; the result itself is positioned in the line's
		// reserved right padding, so it never covers text and the caret ignores it.
		const anchor = document.createElement('span');
		anchor.className = 'cm-calc-anchor';
		anchor.setAttribute('aria-hidden', 'true');
		const slot = document.createElement(copy ? 'button' : 'span');
		slot.className = KIND_CLASS[kind] ?? 'cm-calc-result';
		slot.textContent = text;
		if (slot instanceof HTMLButtonElement) {
			slot.type = 'button';
			slot.tabIndex = -1;
			slot.title = 'Copy';
			// Keep the caret and focus where they are.
			slot.addEventListener('mousedown', (e) => e.preventDefault());
			slot.addEventListener('click', (e) => {
				e.preventDefault();
				const flash = (cls: string, title: string) => {
					slot.classList.add(cls);
					slot.title = title;
					setTimeout(() => {
						slot.classList.remove(cls);
						slot.title = 'Copy';
					}, FLASH_MS);
				};
				navigator.clipboard.writeText(copy).then(
					() => flash('cm-calc-copied', 'Copied'),
					() => flash('cm-calc-copy-failed', 'Couldn’t copy')
				);
			});
		}
		anchor.append(slot);
		return anchor;
	}

	ignoreEvent(): boolean {
		return true;
	}
}

/** Characters the result needs, as reserved line padding (⚠ is wider than a digit). */
function width(result: CalcLine): string {
	return `${[...result.text].length + (result.text === result.copy ? 0 : 1)}ch`;
}

export function calcDecorations(state: EditorState): DecorationSet {
	const decos: Range<Decoration>[] = [];
	for (const { from, to, result } of calcResults(state)) {
		decos.push(Decoration.line({ class: 'cm-calc-line', attributes: { style: `--cm-calc-width: ${width(result)}` } }).range(from));
		decos.push(Decoration.widget({ widget: new CalcResultWidget(result), side: 1 }).range(to));
	}
	return Decoration.set(decos, true);
}

class CalcResults {
	decorations: DecorationSet;
	/** Set while an IME/dead-key composition defers a rebuild. */
	stale = false;

	constructor(view: EditorView) {
		this.decorations = calcDecorations(view.state);
	}

	update(update: ViewUpdate): void {
		if (update.view.composing) {
			// Rebuilding mid-composition breaks WebKit IME and dead keys: only map positions.
			this.decorations = this.decorations.map(update.changes);
			this.stale = true;
			return;
		}
		if (this.stale || update.docChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) {
			this.stale = false;
			this.decorations = calcDecorations(update.state);
		}
	}
}

export const calcResultsPlugin = ViewPlugin.fromClass(CalcResults, {
	decorations: (plugin) => plugin.decorations,
	eventHandlers: {
		compositionend(_event, view) {
			// Rebuild once the composition has been committed.
			setTimeout(() => {
				if (this.stale && !view.composing && view.dom.isConnected) view.dispatch({});
			}, 20);
		}
	}
});
