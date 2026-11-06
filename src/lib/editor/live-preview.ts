// Obsidian-style live preview for the visible part of the document: styles inline and
// line-level markdown and hides its markers, except on lines the selection touches.
// Multi-line replacements (tables, setext underlines) live in blocks.ts.
import { isolateHistory } from '@codemirror/commands';
import { syntaxTree } from '@codemirror/language';
import type { EditorState, Range } from '@codemirror/state';
import {
	Decoration,
	EditorView,
	ViewPlugin,
	WidgetType,
	type DecorationSet,
	type ViewUpdate
} from '@codemirror/view';
import type { SyntaxNode } from '@lezer/common';
import { focusField, revealedLines, selectionTouchesLines } from '#lib/editor/focus.js';

const hidden = Decoration.replace({});

const markClass = {
	Emphasis: 'cm-md-em',
	StrongEmphasis: 'cm-md-strong',
	Strikethrough: 'cm-md-strike',
	InlineCode: 'cm-md-code'
} as const;

const inlineMarks = new Set(['EmphasisMark', 'StrikethroughMark']);

class BulletWidget extends WidgetType {
	eq(): boolean {
		return true;
	}
	toDOM(): HTMLElement {
		const el = document.createElement('span');
		el.className = 'cm-md-bullet';
		el.textContent = '•';
		return el;
	}
}

class RuleWidget extends WidgetType {
	eq(): boolean {
		return true;
	}
	toDOM(): HTMLElement {
		const el = document.createElement('span');
		el.className = 'cm-md-hr';
		el.setAttribute('aria-hidden', 'true');
		return el;
	}
}

/** Flips `[ ]` ↔ `[x]` at `from` as its own undo step. */
export function toggleTask(view: EditorView, from: number): boolean {
	const marker = view.state.sliceDoc(from, from + 3);
	if (!/^\[[ xX]\]$/.test(marker)) return false;
	view.dispatch({
		changes: { from: from + 1, to: from + 2, insert: marker[1] === ' ' ? 'x' : ' ' },
		annotations: isolateHistory.of('full'),
		userEvent: 'input.toggle'
	});
	return true;
}

class CheckboxWidget extends WidgetType {
	constructor(readonly checked: boolean) {
		super();
	}
	eq(other: CheckboxWidget): boolean {
		return other.checked === this.checked;
	}
	toDOM(view: EditorView): HTMLElement {
		const box = document.createElement('span');
		box.className = this.checked ? 'cm-md-checkbox cm-md-checkbox-checked' : 'cm-md-checkbox';
		box.setAttribute('role', 'checkbox');
		box.setAttribute('aria-checked', String(this.checked));
		box.setAttribute('aria-label', this.checked ? 'Mark as not done' : 'Mark as done');
		// mousedown + preventDefault: toggle without moving focus or the selection.
		box.addEventListener('mousedown', (event) => {
			if (event.button !== 0) return;
			event.preventDefault();
			event.stopPropagation();
			toggleTask(view, view.posAtDOM(box));
		});
		return box;
	}
	ignoreEvent(): boolean {
		return true;
	}
}

const bullet = new BulletWidget();
const rule = new RuleWidget();
const checked = new CheckboxWidget(true);
const unchecked = new CheckboxWidget(false);

function line(cls: string, at: number): Range<Decoration> {
	return Decoration.line({ class: cls }).range(at);
}

/** Every line start in [from, to]. */
function lineStarts(state: EditorState, from: number, to: number): number[] {
	const starts: number[] = [];
	for (let pos = state.doc.lineAt(from).from; pos <= to; ) {
		const l = state.doc.lineAt(pos);
		starts.push(l.from);
		pos = l.to + 1;
	}
	return starts;
}

function build(view: EditorView): { decorations: DecorationSet; atomic: DecorationSet } {
	const { state } = view;
	const revealed = revealedLines(state);
	const isRevealed = (pos: number) => revealed.has(state.doc.lineAt(pos).number);
	const decos: Range<Decoration>[] = [];
	const atomic: Range<Decoration>[] = [];
	const seen = new Map<string, boolean>(); // node key → descend into children

	const hide = (from: number, to: number) => {
		if (to <= from) return;
		decos.push(hidden.range(from, to));
		atomic.push(hidden.range(from, to));
	};
	const replaceWith = (widget: WidgetType, from: number, to: number) => {
		const deco = Decoration.replace({ widget });
		decos.push(deco.range(from, to));
		atomic.push(deco.range(from, to));
	};
	/** `to`, extended over one following space. */
	const withSpace = (to: number) => (state.sliceDoc(to, to + 1) === ' ' ? to + 1 : to);

	// A node spanning two visible ranges is entered once per range: decorate it once, but
	// descend again when it allows it (Document holds the nodes of both ranges).
	const visit = (node: SyntaxNode): boolean => {
		const key = `${node.name}:${node.from}:${node.to}`;
		let descend = seen.get(key);
		if (descend === undefined) {
			descend = decorate(node) !== false;
			seen.set(key, descend);
		}
		return descend;
	};

	const decorate = (node: SyntaxNode): boolean | void => {
		const name = node.name;

		const heading = /^ATXHeading(\d)$/.exec(name);
		if (heading) {
			decos.push(line(`cm-md-h${heading[1]}`, state.doc.lineAt(node.from).from));
			const mark = node.firstChild;
			if (mark?.name === 'HeaderMark' && !isRevealed(node.from)) hide(mark.from, withSpace(mark.to));
			return;
		}
		const setext = /^SetextHeading(\d)$/.exec(name);
		if (setext) {
			const underline = node.lastChild;
			const end = underline?.name === 'HeaderMark' ? underline.from - 1 : node.to;
			for (const start of lineStarts(state, node.from, Math.max(node.from, end))) {
				decos.push(line(`cm-md-h${setext[1]}`, start));
			}
			return;
		}

		switch (name) {
			case 'Emphasis':
			case 'StrongEmphasis':
			case 'Strikethrough':
				decos.push(Decoration.mark({ class: markClass[name] }).range(node.from, node.to));
				return;
			case 'InlineCode': {
				decos.push(Decoration.mark({ class: markClass[name] }).range(node.from, node.to));
				if (!isRevealed(node.from)) {
					for (let c = node.firstChild; c; c = c.nextSibling) if (c.name === 'CodeMark') hide(c.from, c.to);
				}
				return false;
			}
			case 'FencedCode': {
				const starts = lineStarts(state, node.from, node.to);
				starts.forEach((start, i) => {
					let cls = 'cm-md-codeblock';
					if (i === 0) cls += ' cm-md-codeblock-first';
					if (i === starts.length - 1) cls += ' cm-md-codeblock-last';
					decos.push(line(cls, start));
				});
				if (!selectionTouchesLines(state, node.from, node.to)) {
					for (let c = node.firstChild; c; c = c.nextSibling) {
						if (c.name === 'CodeMark') hide(c.from, c.to);
						else if (c.name === 'CodeInfo') decos.push(Decoration.mark({ class: 'cm-md-codeinfo' }).range(c.from, c.to));
					}
				}
				return false;
			}
			case 'Blockquote': {
				for (const start of lineStarts(state, node.from, node.to)) decos.push(line('cm-md-quote', start));
				return;
			}
			case 'QuoteMark':
				if (!isRevealed(node.from)) hide(node.from, withSpace(node.to));
				return;
			case 'HorizontalRule':
				decos.push(line('cm-md-hr-line', state.doc.lineAt(node.from).from));
				if (!isRevealed(node.from)) replaceWith(rule, node.from, node.to);
				return;
			case 'ListMark': {
				const item = node.parent;
				const task = item?.getChild('Task');
				const ordered = item?.parent?.name === 'OrderedList';
				if (ordered || isRevealed(node.from)) {
					decos.push(Decoration.mark({ class: 'cm-md-listmark' }).range(node.from, node.to));
				} else if (task) {
					hide(node.from, withSpace(node.to));
				} else {
					replaceWith(bullet, node.from, node.to);
				}
				return;
			}
			case 'Task': {
				const marker = node.firstChild;
				if (marker?.name !== 'TaskMarker') return;
				const done = /x/i.test(state.sliceDoc(marker.from, marker.to));
				if (done && marker.to < node.to) {
					decos.push(Decoration.mark({ class: 'cm-md-task-done' }).range(marker.to, node.to));
				}
				if (!isRevealed(marker.from)) replaceWith(done ? checked : unchecked, marker.from, marker.to);
				return;
			}
			case 'Link': {
				// `[text]` with no target is not a link here (e.g. a callout's `[!note]`).
				if (!node.getChild('URL') && !node.getChild('LinkLabel')) return;
				const marks = node.getChildren('LinkMark');
				if (marks.length < 2 || marks[0].to >= marks[1].from) return;
				decos.push(Decoration.mark({ class: 'cm-md-link' }).range(marks[0].to, marks[1].from));
				if (isRevealed(node.from)) {
					if (marks[1].to < node.to) decos.push(Decoration.mark({ class: 'cm-md-url' }).range(marks[1].to, node.to));
				} else {
					hide(marks[0].from, marks[0].to);
					hide(marks[1].from, node.to);
				}
				return;
			}
			case 'Autolink': {
				const url = node.getChild('URL');
				if (!url) return false;
				decos.push(Decoration.mark({ class: 'cm-md-link' }).range(url.from, url.to));
				if (!isRevealed(node.from)) {
					hide(node.from, url.from);
					hide(url.to, node.to);
				}
				return false;
			}
			case 'URL':
				if (node.parent?.name !== 'Link' && node.parent?.name !== 'Image') {
					decos.push(Decoration.mark({ class: 'cm-md-link' }).range(node.from, node.to));
				}
				return;
			case 'Table':
				// Shown only while the selection is in the table (blocks.ts renders it otherwise).
				for (const start of lineStarts(state, node.from, node.to)) decos.push(line('cm-md-table-src', start));
				return false;
			case 'Image':
			case 'HTMLBlock':
			case 'CommentBlock':
				return false;
		}

		if (inlineMarks.has(name) && !isRevealed(node.from)) hide(node.from, node.to);
	};

	const tree = syntaxTree(state);
	for (const { from, to } of view.visibleRanges) tree.iterate({ from, to, enter: (n) => visit(n.node) });

	return { decorations: Decoration.set(decos, true), atomic: Decoration.set(atomic, true) };
}

class LivePreview {
	decorations: DecorationSet;
	atomic: DecorationSet;
	/** Set while an IME/dead-key composition defers a rebuild. */
	stale = false;

	constructor(view: EditorView) {
		({ decorations: this.decorations, atomic: this.atomic } = build(view));
	}

	update(update: ViewUpdate): void {
		if (update.view.composing) {
			// Rebuilding mid-composition breaks WebKit IME and dead keys: only map positions.
			this.decorations = this.decorations.map(update.changes);
			this.atomic = this.atomic.map(update.changes);
			this.stale = true;
			return;
		}
		if (
			this.stale ||
			update.docChanged ||
			update.viewportChanged ||
			update.selectionSet ||
			update.state.field(focusField, false) !== update.startState.field(focusField, false) ||
			syntaxTree(update.state) !== syntaxTree(update.startState)
		) {
			this.stale = false;
			({ decorations: this.decorations, atomic: this.atomic } = build(update.view));
		}
	}
}

export const livePreview = ViewPlugin.fromClass(LivePreview, {
	decorations: (plugin) => plugin.decorations,
	eventHandlers: {
		compositionend(_event, view) {
			// Rebuild once the composition has been committed.
			setTimeout(() => {
				if (this.stale && !view.composing && view.dom.isConnected) view.dispatch({});
			}, 20);
		}
	},
	provide: (plugin) =>
		EditorView.atomicRanges.of((view) => view.plugin(plugin)?.atomic ?? Decoration.none)
});
