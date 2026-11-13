// The playground's code editor: one EditorView showing one script at a time, each script
// keeping its own EditorState (unsaved edits, undo history) and scroll position while the
// view lives. JavaScript comes from the language-data entry (lazy chunk, its local-variable
// completions and snippets included); on top of it: curated QuickJS globals, `console.`
// members, note titles inside `note("…")`, a syntax-error lint from the parse tree, ⌘↵ to
// run and Esc to stop. No TypeScript, no basicSetup.
import {
	autocompletion,
	closeBrackets,
	closeBracketsKeymap,
	completionKeymap,
	completionStatus,
	type Completion,
	type CompletionContext,
	type CompletionResult
} from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { bracketMatching, ensureSyntaxTree, indentOnInput, syntaxTree, type LanguageSupport } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { linter, lintGutter, type Diagnostic } from '@codemirror/lint';
import { EditorState, Prec, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, keymap, lineNumbers, placeholder } from '@codemirror/view';
import { noteTheme } from '#lib/editor/theme.js';
import { memberCompletions, noteTitleContext, PLAYGROUND_GLOBALS, quoteTitle } from '#lib/playground.js';

export interface JsEditorHooks {
	/** After every edit: the shown script's id and its full text. */
	onchange: (id: number, body: string) => void;
	/** ⌘↵. */
	onrun: () => void;
	/** Esc with no completion open; true when it stopped a run (the key is consumed). */
	onescape: () => boolean;
	/** Note titles for `note("…")`. */
	noteTitles: () => string[];
}

let jsSupport: Promise<LanguageSupport> | null = null;

/** The JavaScript language from language-data, loaded once (a separate chunk). */
export function loadJavaScript(): Promise<LanguageSupport> {
	if (!jsSupport) {
		const desc = languages.find((l) => l.name === 'JavaScript');
		jsSupport = desc ? desc.load() : Promise.reject(new Error('JavaScript support is missing'));
		// A failed load can be retried by the next caller.
		jsSupport.catch(() => (jsSupport = null));
	}
	return jsSupport;
}

const mono = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

const jsTheme = EditorView.theme({
	'&.cm-js-mode .cm-content': { fontFamily: mono, fontSize: '13.5px', lineHeight: '1.6', padding: '0.75rem 0 6rem', width: 'auto' },
	'&.cm-js-mode .cm-line': { padding: '0 1.5rem 0 0.5rem' },
	'&.cm-js-mode .cm-gutters': { paddingLeft: '0.5rem' },
	'&.cm-js-mode .cm-lineNumbers .cm-gutterElement': {
		fontFamily: mono,
		fontSize: '12px',
		lineHeight: '21.6px',
		minWidth: '2.5ch',
		padding: '0 0.25rem 0 0',
		color: 'var(--note-muted)'
	},
	'&.cm-js-mode .cm-matchingBracket': { backgroundColor: 'var(--note-selection)', outline: 'none' },
	'&.cm-js-mode .cm-lintRange-error': {
		backgroundImage: 'none',
		textDecoration: 'underline wavy var(--note-code-invalid)',
		textUnderlineOffset: '3px'
	}
});

// ── Completion ────────────────────────────────────────────────────────────────

const NO_GLOBALS_IN = new Set(['String', 'TemplateString', 'LineComment', 'BlockComment', 'PropertyName', 'VariableDefinition']);

/** Note titles inside `note("…")`; the closing quote is added unless it is already there. */
function noteTitleSource(titles: () => string[]) {
	return (context: CompletionContext): CompletionResult | null => {
		const line = context.state.doc.lineAt(context.pos);
		const found = noteTitleContext(line.text.slice(0, context.pos - line.from));
		if (!found) return null;
		const { quote } = found;
		const options: Completion[] = titles().map((title) => ({
			label: title,
			type: 'text',
			detail: 'note',
			apply: (view, _completion, from, to) => {
				const closed = view.state.sliceDoc(to, to + 1) === quote;
				const insert = quoteTitle(title, quote) + (closed ? '' : quote);
				view.dispatch({
					changes: { from, to, insert },
					selection: { anchor: from + insert.length + (closed ? 1 : 0) },
					userEvent: 'input.complete'
				});
			}
		}));
		return { from: line.from + found.from, options, validFor: /^[^"'`\\\n]*$/ };
	};
}

const globalOptions: Completion[] = PLAYGROUND_GLOBALS.map((g) => ({ label: g.label, type: g.type, info: g.info, boost: g.label === 'note' ? 2 : 1 }));

/** Curated globals, and members of `console.`, `JSON.`, `Math.`, … */
function globalsSource(context: CompletionContext): CompletionResult | null {
	const node = syntaxTree(context.state).resolveInner(context.pos, -1);
	const member = context.matchBefore(/(?:^|[^\w$.])([A-Za-z_$][\w$]*)\.[\w$]*$/);
	if (member) {
		const object = /([A-Za-z_$][\w$]*)\.[\w$]*$/.exec(member.text)?.[1] ?? '';
		const members = memberCompletions(object);
		if (!members) return null;
		const dot = member.text.lastIndexOf('.');
		const props = members.map((m): Completion => ({ label: m, type: /^[A-Z_]+$/.test(m) ? 'constant' : 'method' }));
		return { from: member.from + dot + 1, options: props, validFor: /^[\w$]*$/ };
	}
	if (NO_GLOBALS_IN.has(node.name)) return null;
	const word = context.matchBefore(/[A-Za-z_$][\w$]*$/);
	if (!word && !context.explicit) return null;
	const from = word?.from ?? context.pos;
	if (from > 0 && context.state.sliceDoc(from - 1, from) === '.') return null;
	return { from, options: globalOptions, validFor: /^[\w$]*$/ };
}

// ── Lint ──────────────────────────────────────────────────────────────────────

/** The first error node of the parse tree (parsing at most 100 ms), as one diagnostic. */
function syntaxDiagnostics(view: EditorView): Diagnostic[] {
	const { state } = view;
	const tree = ensureSyntaxTree(state, state.doc.length, 100) ?? syntaxTree(state);
	let found: Diagnostic | null = null;
	tree.iterate({
		enter(node) {
			if (found) return false;
			if (!node.type.isError) return;
			const from = Math.min(node.from, state.doc.length);
			const to = Math.max(node.to, Math.min(from + 1, state.doc.length));
			found = { from, to, severity: 'error', message: 'Syntax error' };
			return false;
		}
	});
	return found ? [found] : [];
}

// ── Editor ────────────────────────────────────────────────────────────────────

function jsExtensions(js: LanguageSupport, hooks: JsEditorHooks, onchange: (body: string) => void): Extension[] {
	return [
		js,
		js.language.data.of({ autocomplete: noteTitleSource(hooks.noteTitles) }),
		js.language.data.of({ autocomplete: globalsSource }),
		Prec.highest(
			keymap.of([
				{
					key: 'Mod-Enter',
					preventDefault: true,
					run: () => {
						hooks.onrun();
						return true;
					}
				}
			])
		),
		history(),
		drawSelection(),
		lineNumbers(),
		bracketMatching(),
		closeBrackets(),
		indentOnInput(),
		autocompletion({ icons: false, closeOnBlur: true }),
		keymap.of([
			{ key: 'Escape', run: (view) => completionStatus(view.state) === null && hooks.onescape() },
			...closeBracketsKeymap,
			...completionKeymap,
			...defaultKeymap,
			...historyKeymap,
			indentWithTab
		]),
		linter(syntaxDiagnostics, { needsRefresh: (update) => syntaxTree(update.state) !== syntaxTree(update.startState) }),
		lintGutter(),
		noteTheme,
		jsTheme,
		EditorView.editorAttributes.of({ class: 'cm-js-mode' }),
		EditorView.contentAttributes.of({ spellcheck: 'false', autocorrect: 'off', autocapitalize: 'off', 'aria-label': 'Script' }),
		placeholder('// note("Title") reads a note. ⌘↵ runs the script.'),
		EditorView.updateListener.of((update) => {
			if (update.docChanged) onchange(update.state.doc.toString());
		})
	];
}

export class JsEditor {
	private readonly view: EditorView;
	private readonly extensions: Extension[];
	private readonly states = new Map<number, EditorState>();
	private readonly scrolls = new Map<number, number>();
	private currentId: number | null = null;

	constructor(parent: HTMLElement, js: LanguageSupport, hooks: JsEditorHooks) {
		this.extensions = jsExtensions(js, hooks, (body) => {
			if (this.currentId !== null) hooks.onchange(this.currentId, body);
		});
		this.view = new EditorView({ parent, state: this.createState('') });
	}

	private createState(body: string): EditorState {
		return EditorState.create({ doc: body, extensions: this.extensions });
	}

	/** Shows script `id`; its cached state (unsaved edits, undo history) wins over `body`. */
	open(id: number, body: string): void {
		if (id === this.currentId) return;
		this.stash();
		this.currentId = id;
		this.view.setState(this.states.get(id) ?? this.createState(body));
		const top = this.scrolls.get(id) ?? 0;
		requestAnimationFrame(() => {
			if (this.currentId === id) this.view.scrollDOM.scrollTop = top;
		});
	}

	/** Shows nothing; the current script's state stays cached. */
	close(): void {
		this.stash();
		this.currentId = null;
		this.view.setState(this.createState(''));
	}

	/** Drops a script's cached state (deleted, or replaced by a restore). */
	forget(id: number): void {
		this.states.delete(id);
		this.scrolls.delete(id);
		if (id === this.currentId) {
			this.currentId = null;
			this.view.setState(this.createState(''));
		}
	}

	/** The shown script's current text (unsaved edits included). */
	text(): string {
		return this.view.state.doc.toString();
	}

	focus(): void {
		this.view.focus();
	}

	/** Puts the caret at 1-based `line`:`col` (clamped) and focuses the editor. */
	goTo(line: number, col: number | null): void {
		const doc = this.view.state.doc;
		const l = doc.line(Math.min(Math.max(line, 1), doc.lines));
		const pos = Math.min(l.from + Math.max((col ?? 1) - 1, 0), l.to);
		this.view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: 'center' }) });
		this.view.focus();
	}

	destroy(): void {
		this.view.destroy();
	}

	private stash(): void {
		if (this.currentId === null) return;
		this.states.set(this.currentId, this.view.state);
		this.scrolls.set(this.currentId, this.view.scrollDOM.scrollTop);
	}
}
