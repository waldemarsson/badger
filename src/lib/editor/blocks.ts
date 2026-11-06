// Multi-line live preview, provided from a StateField (plugins can't replace line
// breaks): GFM tables render as a block widget, and setext underlines are hidden.
// Both show their source while the selection touches them.
import { syntaxTree } from '@codemirror/language';
import { Prec, StateField, type EditorState, type Range } from '@codemirror/state';
import { Decoration, EditorView, keymap, WidgetType, type DecorationSet } from '@codemirror/view';
import { focusField, selectionTouches, selectionTouchesLines } from '#lib/editor/focus.js';
import { parseInline, parseTable, type InlineSpan } from '#lib/notes.js';

const ROW_HEIGHT = 34; // px, a height estimate until the widget is measured

const spanClasses: [keyof Omit<InlineSpan, 'text'>, string][] = [
	['bold', 'cm-md-strong'],
	['italic', 'cm-md-em'],
	['code', 'cm-md-code'],
	['strike', 'cm-md-strike'],
	['link', 'cm-md-link']
];

/** Cell content built from text nodes only (never innerHTML). */
function renderInline(parent: HTMLElement, text: string): void {
	for (const span of parseInline(text)) {
		const classes = spanClasses.filter(([key]) => span[key]).map(([, cls]) => cls);
		if (!classes.length) {
			parent.appendChild(document.createTextNode(span.text));
			continue;
		}
		const el = document.createElement('span');
		el.className = classes.join(' ');
		el.appendChild(document.createTextNode(span.text));
		parent.appendChild(el);
	}
}

class TableWidget extends WidgetType {
	constructor(readonly source: string) {
		super();
	}

	eq(other: TableWidget): boolean {
		return other.source === this.source;
	}

	get estimatedHeight(): number {
		return (this.source.split('\n').length - 1) * ROW_HEIGHT + 16;
	}

	toDOM(view: EditorView): HTMLElement {
		const model = parseTable(this.source);
		const wrap = document.createElement('div');
		wrap.className = 'cm-md-table-wrap';
		const frame = document.createElement('div');
		frame.className = 'cm-md-table-frame';
		const table = document.createElement('table');
		table.className = 'cm-md-table';

		const addRow = (section: HTMLElement, cells: typeof model.header, tag: 'th' | 'td') => {
			const tr = document.createElement('tr');
			cells.forEach((cell, i) => {
				const el = document.createElement(tag);
				const align = model.align[i];
				if (align) el.style.textAlign = align;
				el.dataset.to = String(cell.to);
				renderInline(el, cell.text);
				tr.appendChild(el);
			});
			section.appendChild(tr);
		};

		const thead = document.createElement('thead');
		addRow(thead, model.header, 'th');
		table.appendChild(thead);
		if (model.rows.length) {
			const tbody = document.createElement('tbody');
			for (const row of model.rows) addRow(tbody, row, 'td');
			table.appendChild(tbody);
		}
		frame.appendChild(table);
		wrap.appendChild(frame);

		// Clicking puts the cursor at the end of the clicked cell's source (fallback: table start).
		wrap.addEventListener('mousedown', (event) => {
			if (event.button !== 0) return;
			event.preventDefault();
			const start = view.posAtDOM(wrap);
			const cell = (event.target as Element | null)?.closest<HTMLElement>('[data-to]');
			const offset = cell ? Number(cell.dataset.to) : 0;
			const anchor = Math.min(view.state.doc.length, start + (Number.isFinite(offset) ? offset : 0));
			view.dispatch({ selection: { anchor }, userEvent: 'select.pointer' });
			view.focus();
		});
		return wrap;
	}

	ignoreEvent(): boolean {
		return true;
	}
}

/** Containers whose children can hold a table or setext heading. */
const containers = new Set(['Document', 'BulletList', 'OrderedList', 'ListItem']);

interface Blocks {
	decorations: DecorationSet;
	/** Hidden setext underlines; tables stay out so the arrow keys land on their boundary. */
	atomic: DecorationSet;
}

function build(state: EditorState): Blocks {
	const decos: Range<Decoration>[] = [];
	const atomic: Range<Decoration>[] = [];
	syntaxTree(state).iterate({
		enter: (node) => {
			if (containers.has(node.name)) return;
			if (node.name === 'Table') {
				const from = state.doc.lineAt(node.from).from;
				const to = state.doc.lineAt(node.to).to;
				// Boundaries count as inside, so the arrow keys can enter the table.
				if (!selectionTouches(state, from, to)) {
					const widget = new TableWidget(state.sliceDoc(from, to));
					decos.push(Decoration.replace({ widget, block: true }).range(from, to));
				}
			} else if (node.name === 'SetextHeading1' || node.name === 'SetextHeading2') {
				const underline = node.node.lastChild;
				if (underline?.name === 'HeaderMark' && !selectionTouchesLines(state, node.from, node.to)) {
					const markLine = state.doc.lineAt(underline.from);
					if (markLine.from > 0) {
						const hide = Decoration.replace({}).range(markLine.from - 1, markLine.to);
						decos.push(hide);
						atomic.push(hide);
					}
				}
			}
			return false;
		}
	});
	return { decorations: Decoration.set(decos, true), atomic: Decoration.set(atomic, true) };
}

const blockField = StateField.define<Blocks>({
	create: build,
	update(value, tr) {
		const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
		const focusChanged = tr.state.field(focusField, false) !== tr.startState.field(focusField, false);
		if (tr.docChanged || tr.selection || treeChanged || focusChanged) return build(tr.state);
		return value;
	},
	provide: (field) => [
		EditorView.decorations.from(field, (blocks) => blocks.decorations),
		EditorView.atomicRanges.of((view) => view.state.field(field).atomic)
	]
});

/**
 * ↓/↑ from the line next to a rendered table: vertical motion would skip the block
 * widget, so put the cursor on the table's first (↓) or last (↑) row instead, which
 * shows its source.
 */
function enterTable(forward: boolean) {
	return (view: EditorView): boolean => {
		const { state } = view;
		const { main } = state.selection;
		if (state.selection.ranges.length > 1 || !main.empty) return false;
		const line = state.doc.lineAt(main.head);
		if (state.doc.lineAt(view.moveVertically(main, forward).head).number === line.number) return false;
		const edge = forward ? line.to + 1 : line.from - 1;
		if (edge < 0 || edge > state.doc.length) return false;
		let row: number | null = null; // start of the table row to land on
		state.field(blockField).decorations.between(edge, edge, (from, to, deco) => {
			if (!deco.spec.block) return;
			if (forward && from === edge) row = from;
			else if (!forward && to === edge) row = state.doc.lineAt(to).from;
			if (row !== null) return false;
		});
		if (row === null) return false;
		const target = state.doc.lineAt(row);
		const anchor = target.from + Math.min(main.head - line.from, target.length);
		view.dispatch({ selection: { anchor }, scrollIntoView: true, userEvent: 'select' });
		return true;
	};
}

export const blockPreview = [
	blockField,
	Prec.high(
		keymap.of([
			{ key: 'ArrowDown', run: enterTable(true) },
			{ key: 'ArrowUp', run: enterTable(false) }
		])
	)
];
