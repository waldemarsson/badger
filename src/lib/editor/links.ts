// `#tags`, `#issue/<id>` / `#todo/<id>` chips and `[[wikilinks]]` in the editor: `#` and `[[`
// completion, live-preview decorations, and ⌘-click to follow. What links resolve to (note
// titles, issue and todo titles) lives in a StateField the page fills through
// `setLinkData`, which also redraws the decorations. The rules live in src/lib/note-links.ts.
import { pickedCompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';
import { Facet, Prec, StateEffect, StateField, type EditorState, type Range, type Text } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import type { Tree } from '@lezer/common';
import { focusField, revealedLines } from '#lib/editor/focus.js';
import {
	chipFor,
	extractRefs,
	filterTagOptions,
	indexLinkTargets,
	tagCompletion,
	tagTrigger,
	wikilinkCompletion,
	wikilinkOptions,
	wikilinkTrigger,
	type LinkFollow,
	type LinkIndex,
	type LinkTargetData,
	type NoteRef,
	type TagOption
} from '#lib/note-links.js';
import { isCodeAt } from '#lib/slash-menu.js';

// ── Resolution data ───────────────────────────────────────────────────────────

export interface LinkData {
	notes: LinkTargetData['notes'];
	index: LinkIndex;
	suggestions: TagOption[];
}

export function linkData(targets: LinkTargetData, suggestions: TagOption[]): LinkData {
	return { notes: targets.notes, index: indexLinkTargets(targets), suggestions };
}

export const emptyLinkData: LinkData = linkData({ notes: [], issues: [], todos: [] }, []);

export const setLinkData = StateEffect.define<LinkData>();

export const linkDataField = StateField.define<LinkData>({
	create: () => emptyLinkData,
	update(value, tr) {
		for (const effect of tr.effects) if (effect.is(setLinkData)) value = effect.value;
		return value;
	}
});

/** Tags and wikilinks of the document, parsed once per document and syntax tree. */
const refCache = new WeakMap<Tree, { doc: Text; refs: NoteRef[] }>();

function refsOf(state: EditorState): NoteRef[] {
	const tree = syntaxTree(state);
	const cached = refCache.get(tree);
	if (cached && cached.doc === state.doc) return cached.refs;
	const refs = extractRefs(state.doc.toString(), tree);
	refCache.set(tree, { doc: state.doc, refs });
	return refs;
}

// ── Completion ────────────────────────────────────────────────────────────────

const kindLabel: Record<TagOption['kind'], string> = {
	tag: 'Tag',
	customer: 'Customer',
	project: 'Project',
	issue: 'Issue',
	todo: 'Todo'
};

/** Prefix of the completion `type` that picks the row's kind badge (`linkKindBadge`). */
const kindPrefix = 'link-';

function tagOptionCompletion(option: TagOption): Completion {
	const tag = tagCompletion(option.tag);
	const chip = option.kind === 'issue' || option.kind === 'todo';
	const project = option.kind === 'project' && option.detail ? `${option.detail} / ${option.label}` : option.label;
	return {
		label: tag,
		// Issues and todos show their title; everything else its tag, with the name beside it.
		displayLabel: chip ? option.label : option.kind === 'project' ? project : option.kind === 'tag' ? tag : option.label,
		detail: option.kind === 'tag' ? undefined : tag,
		type: `${kindPrefix}${option.kind === 'todo' ? `todo-${option.status ?? 'not_started'}` : option.kind}`,
		apply: tag
	};
}

function tagSource(context: CompletionContext): CompletionResult | null {
	const { state, pos } = context;
	const line = state.doc.lineAt(pos);
	const trigger = tagTrigger(line.text.slice(0, pos - line.from));
	if (!trigger) return null;
	const hash = line.from + trigger.from;
	if (isCodeAt(syntaxTree(state), hash)) return null;

	// Tags written in this note but not saved yet are offered too.
	const { suggestions } = state.field(linkDataField);
	const known = new Set(suggestions.map((s) => s.tag));
	const local: TagOption[] = [];
	for (const ref of refsOf(state)) {
		if (ref.kind !== 'tag' || ref.from === hash || known.has(ref.target)) continue;
		known.add(ref.target);
		local.push({ kind: 'tag', tag: ref.target, label: ref.target, detail: null, status: null });
	}
	const options = filterTagOptions([...local, ...suggestions], trigger.query).map(tagOptionCompletion);
	// Not filtered or re-sorted by CodeMirror: issues and todos match on their title too.
	return { from: hash, options, filter: false };
}

function applyWikilink(title: string) {
	return (view: EditorView, completion: Completion, from: number, to: number) => {
		const { insert, consumeAfter } = wikilinkCompletion(title, view.state.sliceDoc(to, to + 2));
		view.dispatch({
			changes: { from, to: to + consumeAfter, insert },
			selection: { anchor: from + insert.length },
			userEvent: 'input.complete',
			annotations: pickedCompletion.of(completion)
		});
	};
}

function wikilinkSource(context: CompletionContext): CompletionResult | null {
	const { state, pos } = context;
	const line = state.doc.lineAt(pos);
	const trigger = wikilinkTrigger(line.text.slice(0, pos - line.from));
	if (!trigger) return null;
	const open = line.from + trigger.from;
	if (isCodeAt(syntaxTree(state), open)) return null;

	const { notes, create } = wikilinkOptions(state.field(linkDataField).notes, trigger.query);
	const options: Completion[] = notes.map((note) => ({
		label: note.title,
		type: `${kindPrefix}note`,
		apply: applyWikilink(note.title)
	}));
	if (create) {
		options.push({ label: `Create “${create}”`, type: `${kindPrefix}new`, apply: applyWikilink(create) });
	}
	return { from: open, options, filter: false };
}

export const linkCompletionSources = [tagSource, wikilinkSource];

/** Badge text and class per completion kind (literal class names, for Tailwind). */
const badges: Record<string, [text: string, className: string]> = {
	note: ['Note', 'cm-link-kind'],
	new: ['New note', 'cm-link-kind cm-link-kind-new'],
	tag: [kindLabel.tag, 'cm-link-kind cm-link-kind-tag'],
	customer: [kindLabel.customer, 'cm-link-kind'],
	project: [kindLabel.project, 'cm-link-kind'],
	issue: [kindLabel.issue, 'cm-link-kind cm-link-kind-issue'],
	'todo-not_started': [kindLabel.todo, 'cm-link-kind'],
	'todo-in_progress': [kindLabel.todo, 'cm-link-kind cm-link-kind-todo-in_progress']
};

/** The kind badge in front of `#` and `[[` completion rows (the `/` menu has none). */
export const linkKindBadge = {
	position: 20,
	render(completion: Completion): Node | null {
		const kind = completion.type?.startsWith(kindPrefix) ? completion.type.slice(kindPrefix.length) : null;
		const badge = kind ? badges[kind] : undefined;
		if (!badge) return null;
		const el = document.createElement('span');
		[el.textContent, el.className] = badge;
		return el;
	}
};

// ── Live preview ──────────────────────────────────────────────────────────────

/**
 * What ⌘-click can follow in this editor: everything (the notes page), or only resolved
 * [[wikilinks]] (hosts that can open a note but can't search tags or create notes, e.g. the
 * handover cards). Links that can't be followed get no ⌘-click hint or pointer.
 */
export type LinkFollowScope = 'all' | 'notes';

export const linkFollowScope = Facet.define<LinkFollowScope, LinkFollowScope>({
	combine: (values) => values[0] ?? 'all'
});

const hidden = Decoration.replace({});

class ChipWidget extends WidgetType {
	constructor(
		readonly tag: string,
		readonly kind: 'issue' | 'todo',
		readonly title: string,
		readonly status: string | null,
		readonly detail: string | null,
		readonly followable: boolean
	) {
		super();
	}
	eq(other: ChipWidget): boolean {
		return (
			other.tag === this.tag &&
			other.title === this.title &&
			other.status === this.status &&
			other.detail === this.detail &&
			other.followable === this.followable
		);
	}
	toDOM(): HTMLElement {
		const el = document.createElement('span');
		// Literal class names: Tailwind only keeps app.css component rules it finds in sources.
		el.className = this.kind === 'issue' ? 'note-chip note-chip-issue' : 'note-chip note-chip-todo';
		if (this.followable) el.dataset.linkTag = this.tag;
		if (this.status) el.dataset.status = this.status;
		el.title = `${this.detail ? `${this.detail} · ` : ''}#${this.tag}${this.followable ? ` · ${modKey}-click to find notes` : ''}`;
		el.textContent = this.title;
		return el;
	}
	ignoreEvent(): boolean {
		return false; // a click places the cursor (and reveals the source)
	}
}

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
const modKey = isMac ? '⌘' : 'Ctrl';
/** ⌘ on macOS (CodeMirror's own ⌘-click adds a cursor), Ctrl elsewhere. */
const followModifier = (event: MouseEvent | KeyboardEvent) => (isMac ? event.metaKey : event.ctrlKey);

function build(view: EditorView): { decorations: DecorationSet; atomic: DecorationSet } {
	const { state } = view;
	const { index } = state.field(linkDataField);
	const all = state.facet(linkFollowScope) === 'all'; // else only resolved wikilinks follow
	const revealed = revealedLines(state);
	const isRevealed = (pos: number) => revealed.has(state.doc.lineAt(pos).number);
	const visible = view.visibleRanges;
	const decos: Range<Decoration>[] = [];
	const atomic: Range<Decoration>[] = [];
	const hide = (from: number, to: number) => {
		if (to <= from) return;
		decos.push(hidden.range(from, to));
		atomic.push(hidden.range(from, to));
	};

	for (const ref of refsOf(state)) {
		if (!visible.some((r) => ref.to >= r.from && ref.from <= r.to)) continue;
		const rendered = !isRevealed(ref.from); // markers hidden, chips drawn
		if (ref.kind === 'tag') {
			const chip = chipFor(ref.target, index);
			if (chip && rendered) {
				const deco = Decoration.replace({
					widget: new ChipWidget(ref.target, chip.kind, chip.title, chip.status, chip.detail, all)
				});
				decos.push(deco.range(ref.from, ref.to));
				atomic.push(deco.range(ref.from, ref.to));
				continue;
			}
			decos.push(
				Decoration.mark({
					class: 'note-tag',
					attributes: all ? { 'data-link-tag': ref.target, title: `${modKey}-click to find notes tagged #${ref.target}` } : {}
				}).range(ref.from, ref.to)
			);
			continue;
		}

		const title = state.sliceDoc(ref.targetFrom, ref.targetTo);
		const id = index.notes.get(ref.target);
		const attributes: Record<string, string> = {};
		if (id !== undefined) {
			attributes['data-link-note'] = title;
			attributes['data-link-note-id'] = String(id);
			attributes.title = `${modKey}-click to open “${title}”`;
		} else if (all) {
			attributes['data-link-note'] = title;
			attributes.title = `${modKey}-click to create “${title}”`;
		}
		const mark = Decoration.mark({
			class: id === undefined ? 'note-wikilink note-wikilink-unresolved' : 'note-wikilink',
			attributes
		});
		if (!rendered) {
			decos.push(mark.range(ref.targetFrom, ref.targetTo));
			continue;
		}
		// Shown: the alias if any (text after the `|`), else the title; brackets hidden.
		let textFrom = ref.targetFrom;
		let textTo = ref.targetTo;
		if (ref.alias !== null) {
			const pipe = state.sliceDoc(ref.targetTo, ref.to).indexOf('|') + ref.targetTo;
			const raw = state.sliceDoc(pipe + 1, ref.to - 2);
			textFrom = pipe + 1 + (raw.length - raw.trimStart().length);
			textTo = textFrom + ref.alias.length;
		}
		hide(ref.from, textFrom);
		decos.push(mark.range(textFrom, textTo));
		hide(textTo, ref.to);
	}
	return { decorations: Decoration.set(decos, true), atomic: Decoration.set(atomic, true) };
}

class LinkPreview {
	decorations: DecorationSet;
	atomic: DecorationSet;
	stale = false;

	constructor(view: EditorView) {
		({ decorations: this.decorations, atomic: this.atomic } = build(view));
	}

	update(update: ViewUpdate): void {
		if (update.view.composing) {
			// Same as live-preview.ts: only map positions while an IME composes.
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
			update.state.field(linkDataField) !== update.startState.field(linkDataField) ||
			syntaxTree(update.state) !== syntaxTree(update.startState)
		) {
			this.stale = false;
			({ decorations: this.decorations, atomic: this.atomic } = build(update.view));
		}
	}
}

const linkPreview = ViewPlugin.fromClass(LinkPreview, {
	decorations: (plugin) => plugin.decorations,
	eventHandlers: {
		compositionend(_event, view) {
			setTimeout(() => {
				if (this.stale && !view.composing && view.dom.isConnected) view.dispatch({});
			}, 20);
		}
	},
	provide: (plugin) => EditorView.atomicRanges.of((view) => view.plugin(plugin)?.atomic ?? Decoration.none)
});

// ── Following ─────────────────────────────────────────────────────────────────

/** The tag or wikilink element under `target`, if any. */
function linkTarget(target: EventTarget | null): LinkFollow | null {
	const el = target instanceof Element ? target.closest<HTMLElement>('[data-link-tag], [data-link-note]') : null;
	if (!el) return null;
	if (el.dataset.linkTag) return { kind: 'tag', tag: el.dataset.linkTag };
	const id = Number(el.dataset.linkNoteId);
	return { kind: 'note', title: el.dataset.linkNote ?? '', id: Number.isInteger(id) && id > 0 ? id : null };
}

/**
 * ⌘-click follows a tag, chip or wikilink. Highest precedence and preventDefault, so
 * CodeMirror never sees it (on macOS it would add a cursor). While ⌘ is held over the
 * editor, links show a pointer.
 */
function following(onFollow: (link: LinkFollow) => void) {
	const setArmed = (view: EditorView, on: boolean) => view.dom.classList.toggle('cm-link-armed', on);
	return Prec.highest(
		EditorView.domEventHandlers({
			mousedown(event) {
				if (event.button !== 0 || !followModifier(event)) return false;
				const link = linkTarget(event.target);
				if (!link) return false;
				event.preventDefault();
				onFollow(link);
				return true;
			},
			mousemove(event, view) {
				setArmed(view, followModifier(event));
				return false;
			},
			mouseleave(_event, view) {
				setArmed(view, false);
				return false;
			},
			keydown(event, view) {
				setArmed(view, followModifier(event));
				return false;
			},
			keyup(event, view) {
				setArmed(view, followModifier(event));
				return false;
			},
			blur(_event, view) {
				setArmed(view, false);
				return false;
			}
		})
	);
}

/** Decorations, resolution data and ⌘-click. Completion sources are wired in slash.ts. */
export function noteLinks(onFollow: (link: LinkFollow) => void, scope: LinkFollowScope = 'all') {
	return [linkDataField, linkFollowScope.of(scope), linkPreview, following(onFollow)];
}
