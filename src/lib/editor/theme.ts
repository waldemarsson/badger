// Editor look. Colors come from the --note-* CSS variables in src/app.css (light and
// .dark), so switching themes needs no reconfigure.
import { markdownLanguage } from '@codemirror/lang-markdown';
import { HighlightStyle, languageDataProp, syntaxHighlighting } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import type { Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import type { NodeType } from '@lezer/common';
import { tags, type Highlighter } from '@lezer/highlight';

const mono = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

const editorTheme = EditorView.theme({
	'&': {
		height: '100%',
		fontSize: '15px',
		color: 'var(--note-fg)',
		backgroundColor: 'transparent'
	},
	'&.cm-focused': { outline: 'none' },
	'.cm-scroller': {
		fontFamily: 'inherit',
		lineHeight: '1.7',
		overflow: 'auto'
	},
	'.cm-content': {
		width: '100%',
		margin: '0 auto',
		padding: '1rem 0 40vh',
		caretColor: 'var(--note-caret)'
	},
	'.cm-line': { padding: '0 2rem' },
	'.cm-cursor, .cm-dropCursor': { borderLeft: '2px solid var(--note-caret)' },
	'&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
		{ backgroundColor: 'var(--note-selection)' },
	'.cm-placeholder': { color: 'var(--note-muted)' },

	// Headings
	'.cm-md-h1, .cm-md-h2, .cm-md-h3, .cm-md-h4, .cm-md-h5, .cm-md-h6': {
		color: 'var(--note-heading)',
		fontWeight: '650',
		letterSpacing: '-0.01em'
	},
	'.cm-md-h1': { fontSize: '1.85em', lineHeight: '1.3', paddingTop: '0.5em', paddingBottom: '0.1em' },
	'.cm-md-h2': { fontSize: '1.45em', lineHeight: '1.35', paddingTop: '0.45em' },
	'.cm-md-h3': { fontSize: '1.2em', lineHeight: '1.4', paddingTop: '0.35em' },
	'.cm-md-h4, .cm-md-h5, .cm-md-h6': { fontSize: '1.05em', paddingTop: '0.25em' },

	// Inline
	'.cm-md-strong': { fontWeight: '650', color: 'var(--note-heading)' },
	'.cm-md-em': { fontStyle: 'italic' },
	'.cm-md-strike': { textDecoration: 'line-through', textDecorationColor: 'var(--note-muted)' },
	'.cm-md-code': {
		fontFamily: mono,
		fontSize: '0.88em',
		color: 'var(--note-code-fg)',
		backgroundColor: 'var(--note-code-bg)',
		borderRadius: '0.35rem',
		padding: '0.1em 0.3em'
	},
	'.cm-md-link': {
		color: 'var(--note-link)',
		textDecoration: 'underline',
		textDecorationColor: 'var(--note-link-underline)',
		textUnderlineOffset: '3px'
	},
	'.cm-md-url': { color: 'var(--note-muted)' },

	// Lists and tasks
	'.cm-md-bullet': { color: 'var(--note-muted)', fontWeight: '700', padding: '0 0.15em' },
	'.cm-md-listmark': { color: 'var(--note-muted)' },
	'.cm-md-checkbox': {
		display: 'inline-block',
		boxSizing: 'border-box',
		width: '1.05em',
		height: '1.05em',
		verticalAlign: '-0.17em',
		marginRight: '0.1em',
		border: '1.5px solid var(--note-checkbox-border)',
		borderRadius: '0.3em',
		backgroundColor: 'var(--note-checkbox-bg)',
		cursor: 'pointer',
		transition: 'background-color 150ms, border-color 150ms'
	},
	'.cm-md-checkbox:hover': { borderColor: 'var(--note-check)' },
	'.cm-md-checkbox-checked': {
		backgroundColor: 'var(--note-check)',
		borderColor: 'var(--note-check)',
		backgroundImage:
			"url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='white' stroke-width='3.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M5 12.5l4.5 4.5L19 7.5'/%3E%3C/svg%3E\")",
		backgroundSize: '80%',
		backgroundPosition: 'center',
		backgroundRepeat: 'no-repeat'
	},
	'.cm-md-task-done': {
		color: 'var(--note-muted)',
		textDecoration: 'line-through',
		textDecorationColor: 'var(--note-muted)'
	},

	// Blocks
	'.cm-md-quote': {
		borderLeft: '3px solid var(--note-quote-border)',
		marginLeft: '2rem',
		paddingLeft: '0.9rem',
		color: 'var(--note-quote-fg)'
	},
	'.cm-md-hr': {
		display: 'inline-block',
		width: '100%',
		verticalAlign: 'middle',
		borderTop: '1px solid var(--note-border)'
	},
	'.cm-md-codeblock': {
		fontFamily: mono,
		fontSize: '0.88em',
		color: 'var(--note-code-fg)',
		backgroundColor: 'var(--note-code-bg)',
		margin: '0 1.25rem',
		padding: '0 0.75rem'
	},
	'.cm-md-codeblock-first': { borderTopLeftRadius: '0.6rem', borderTopRightRadius: '0.6rem', paddingTop: '0.3rem' },
	'.cm-md-codeblock-last': { borderBottomLeftRadius: '0.6rem', borderBottomRightRadius: '0.6rem', paddingBottom: '0.3rem' },
	'.cm-md-codeinfo': {
		fontFamily: 'inherit',
		fontSize: '0.75rem',
		fontWeight: '600',
		letterSpacing: '0.08em',
		textTransform: 'uppercase',
		color: 'var(--note-muted)'
	},

	// Table source (while the selection is in it)
	'.cm-md-table-src': { fontFamily: mono, fontSize: '0.88em' },
	// Rendered table
	'.cm-md-table-wrap': { padding: '0.35rem 2rem', overflowX: 'auto' },
	'.cm-md-table-frame': {
		display: 'inline-block',
		minWidth: '50%',
		maxWidth: '100%',
		border: '1px solid var(--note-border)',
		borderRadius: '0.75rem',
		overflow: 'hidden'
	},
	'.cm-md-table': { borderCollapse: 'collapse', width: '100%', fontSize: '0.93em', lineHeight: '1.5' },
	'.cm-md-table th, .cm-md-table td': {
		padding: '0.4rem 0.85rem',
		textAlign: 'left',
		borderTop: '1px solid var(--note-border)',
		cursor: 'text'
	},
	'.cm-md-table th': {
		borderTop: 'none',
		fontWeight: '600',
		color: 'var(--note-heading)',
		backgroundColor: 'var(--note-table-head)'
	},
	'.cm-md-table th + th, .cm-md-table td + td': { borderLeft: '1px solid var(--note-border)' },

	// `/` menu (the selectors outweigh autocomplete's light/dark base theme)
	'.cm-tooltip.cm-tooltip-autocomplete': {
		border: 'none',
		borderRadius: '0.75rem',
		padding: '0.375rem',
		backgroundColor: 'var(--note-popup-bg)',
		boxShadow:
			'0 0 0 1px var(--note-popup-ring), 0 10px 15px -3px var(--note-popup-shadow), 0 4px 6px -4px var(--note-popup-shadow)'
	},
	'.cm-tooltip.cm-tooltip-autocomplete > ul': {
		fontFamily: 'inherit',
		minWidth: '15rem',
		maxHeight: '16rem'
	},
	'.cm-tooltip.cm-tooltip-autocomplete > ul > li': {
		display: 'flex',
		alignItems: 'baseline',
		padding: '0.4rem 0.65rem',
		borderRadius: '0.5rem',
		fontSize: '0.875rem',
		lineHeight: '1.4',
		color: 'var(--note-fg)',
		transition: 'background-color 150ms, color 150ms'
	},
	'.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]': {
		backgroundColor: 'var(--note-popup-active-bg)',
		color: 'var(--note-popup-active-fg)'
	},
	'.cm-tooltip.cm-tooltip-autocomplete .cm-completionLabel': { fontWeight: '500' },
	'.cm-tooltip.cm-tooltip-autocomplete .cm-completionMatchedText': { textDecoration: 'none', fontWeight: '700' },
	'.cm-tooltip.cm-tooltip-autocomplete .cm-completionDetail': {
		marginLeft: 'auto',
		paddingLeft: '1.5rem',
		fontFamily: mono,
		fontSize: '0.75rem',
		fontStyle: 'normal',
		color: 'var(--note-muted)'
	},
	'.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected] .cm-completionDetail': {
		color: 'var(--note-popup-active-fg)',
		opacity: '0.75'
	},
	// Snippet tab stops
	'.cm-content .cm-snippetField': {
		backgroundColor: 'var(--note-snippet-field)',
		borderRadius: '0.25rem'
	},
	'.cm-content .cm-snippetFieldPosition': { borderLeft: '1.5px dotted var(--note-snippet-marker)' }
});

/** Colors for markdown source that shows while editing (markers, URLs). */
const markdownHighlight = HighlightStyle.define([
	{ tag: tags.processingInstruction, color: 'var(--note-mark)' },
	{ tag: tags.contentSeparator, color: 'var(--note-mark)' },
	{ tag: tags.url, color: 'var(--note-muted)' },
	{ tag: tags.labelName, color: 'var(--note-muted)' },
	{ tag: tags.quote, color: 'var(--note-quote-fg)' },
	{ tag: tags.comment, color: 'var(--note-muted)' }
]);

/** Syntax colors inside fenced code blocks whose language language-data loaded. */
const codeHighlight = HighlightStyle.define([
	{
		tag: [
			tags.keyword,
			tags.controlKeyword,
			tags.operatorKeyword,
			tags.definitionKeyword,
			tags.moduleKeyword,
			tags.modifier,
			tags.self
		],
		color: 'var(--note-code-keyword)'
	},
	{ tag: [tags.string, tags.special(tags.string), tags.regexp, tags.character, tags.escape], color: 'var(--note-code-string)' },
	{ tag: [tags.number, tags.bool, tags.null, tags.atom, tags.unit, tags.constant(tags.name)], color: 'var(--note-code-number)' },
	{ tag: [tags.typeName, tags.className, tags.namespace, tags.attributeName], color: 'var(--note-code-type)' },
	{
		tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.macroName],
		color: 'var(--note-code-function)'
	},
	{ tag: [tags.propertyName, tags.tagName], color: 'var(--note-code-property)' },
	{ tag: tags.comment, color: 'var(--note-code-comment)', fontStyle: 'italic' },
	{ tag: [tags.meta, tags.annotation, tags.processingInstruction], color: 'var(--note-code-meta)' },
	{ tag: [tags.invalid, tags.deleted], color: 'var(--note-code-invalid)' },
	{ tag: tags.inserted, color: 'var(--note-code-string)' },
	{ tag: [tags.heading, tags.strong], fontWeight: '650' },
	{ tag: tags.emphasis, fontStyle: 'italic' }
]);

const codeTypes = new WeakSet<NodeType>();

/** Is `type` the top node of a fenced-code language (loaded from language-data)? */
export function isCodeLanguage(type: NodeType): boolean {
	if (codeTypes.has(type)) return true;
	const data = type.prop(languageDataProp);
	// Every markdown language shares one data facet, including the one a ```md block loads:
	// markdown is never code here, or the note itself would get code colors.
	if (!data || data === markdownLanguage.data) return false;
	// Only positive answers are cached: a description may load later.
	if (!languages.some((desc) => desc.support?.language.data === data)) return false;
	codeTypes.add(type);
	return true;
}

/** `style`, applied only within languages `scope` accepts (checked at each language's top node). */
function scopedHighlighting(style: HighlightStyle, scope: (type: NodeType) => boolean): Extension {
	const highlighter: Highlighter = { style: (t) => style.style(t), scope };
	return [syntaxHighlighting(highlighter), style.module ? EditorView.styleModule.of(style.module) : []];
}

export const noteTheme = [
	editorTheme,
	// Each style stays in its own language: markdown colors never leak into code, and code
	// colors never restyle markdown text the live preview already styles.
	scopedHighlighting(markdownHighlight, (type) => !isCodeLanguage(type)),
	scopedHighlighting(codeHighlight, isCodeLanguage)
];
