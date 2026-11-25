// The read view's ```calc blocks: Rust renders them as plain `<pre><code class="language-calc">`;
// this turns each into rows of expression and result, evaluated together (variables span
// the note) with the editor's evaluator (src/lib/calc.ts). Runs synchronously on the
// rendered DOM before highlightCodeBlocks. Builds with textContent only, never innerHTML.
// The `<pre>` itself stays (Svelte's {@html} tracks its top-level nodes); only its content
// is replaced. Other code blocks are not touched.
import { evaluateBlocks, type CalcKind } from '#lib/calc.js';

export interface CalcReaderOptions {
	/** One line's source as highlighted nodes; plain text without it. */
	highlight?: (text: string) => Node;
}

const ROW_CLASS: Record<CalcKind, string> = {
	value: 'calc-row',
	sum: 'calc-row calc-row-sum',
	error: 'calc-row calc-row-error',
	comment: 'calc-row calc-row-comment',
	blank: 'calc-row calc-row-blank'
};

/** `language-calc` in any case: pulldown-cmark keeps the info string's first word as typed. */
function isCalcCode(code: Element): boolean {
	return code.className.split(/\s+/).some((c) => c.toLowerCase() === 'language-calc');
}

/** The block's lines: pulldown-cmark ends code with a newline. */
function linesOf(text: string): string[] {
	const lines = text.split('\n');
	if (lines[lines.length - 1] === '') lines.pop();
	return lines;
}

/** Renders every calc block under `root` as a table of rows. Returns how many it rendered. */
export function renderCalcBlocks(root: ParentNode, options: CalcReaderOptions = {}): number {
	const codes = [...root.querySelectorAll('pre > code')].filter(isCalcCode);
	if (!codes.length) return 0;
	const sources = codes.map((code) => linesOf(code.textContent ?? ''));
	const results = evaluateBlocks(sources);
	const source = (doc: Document, text: string): Node => options.highlight?.(text) ?? doc.createTextNode(text);

	codes.forEach((code, b) => {
		const pre = code.parentNode as HTMLElement;
		const doc = code.ownerDocument;
		const el = <K extends keyof HTMLElementTagNameMap>(tag: K, className?: string) => {
			const node = doc.createElement(tag);
			if (className) node.className = className;
			return node;
		};
		const lines = results[b];
		// Blank lines at the edges would only pad the block.
		let first = 0;
		let last = lines.length - 1;
		while (first <= last && lines[first].kind === 'blank') first++;
		while (last >= first && lines[last].kind === 'blank') last--;
		pre.className = 'calc-block';
		if (first > last) {
			pre.hidden = true; // an empty block renders nothing
			pre.replaceChildren();
			return;
		}
		const body = el('tbody');
		for (let i = first; i <= last; i++) {
			const { kind, text, copy } = lines[i];
			const row = el('tr', ROW_CLASS[kind]);
			const line = sources[b][i].trim();
			if (kind === 'blank' || kind === 'comment') {
				const cell = el('td', 'calc-expr');
				cell.colSpan = 2;
				if (kind === 'comment') cell.append(source(doc, line));
				row.append(cell);
			} else {
				const expr = el('td', 'calc-expr');
				expr.append(source(doc, line));
				const slot = el('td', 'calc-result');
				if (copy) {
					const button = el('button', 'calc-copy');
					button.type = 'button';
					button.title = 'Copy';
					button.setAttribute('data-calc-copy', copy);
					button.textContent = text;
					slot.append(button);
				} else {
					slot.textContent = text;
				}
				row.append(expr, slot);
			}
			body.append(row);
		}
		const table = el('table', 'calc-table');
		table.append(body);
		pre.replaceChildren(table);
	});
	return codes.length;
}
