// Syntax colors for the read view's code blocks: the editor's parsers (language-data,
// calc) and its code colors, applied to the `<pre><code class="language-x">` Rust renders.
import type { Language } from '@codemirror/language';
import { highlightCode, type Highlighter } from '@lezer/highlight';
import { calcLanguage, calcNumbers, matchCodeLanguage } from '#lib/editor/calc-language.js';
import { codeHighlight } from '#lib/editor/theme.js';

/** `text` as highlighted nodes (spans with the editor's code-color classes, plus `extra`'s). */
export function highlightText(text: string, language: Language, extra: Highlighter[] = []): DocumentFragment {
	const tree = language.parser.parse(text);
	const out = document.createDocumentFragment();
	highlightCode(
		text,
		tree,
		[codeHighlight, ...extra],
		(piece, classes) => {
			if (!classes) return void out.append(piece);
			const span = document.createElement('span');
			span.className = classes;
			span.textContent = piece;
			out.append(span);
		},
		() => out.append('\n')
	);
	return out;
}

/** One line of a calc block, highlighted (the read view's calc rows); numbers marked `calc-num`. */
export function highlightCalc(text: string): DocumentFragment {
	return highlightText(text, calcLanguage, [calcNumbers]);
}

/**
 * Highlights every `code[class^="language-"]` under `root` in place. Stops when
 * `isCurrent()` turns false (the content was replaced while a language loaded).
 */
export async function highlightCodeBlocks(root: HTMLElement, isCurrent: () => boolean): Promise<void> {
	for (const code of root.querySelectorAll<HTMLElement>('pre > code[class^="language-"]')) {
		const name = code.className.slice('language-'.length);
		const desc = matchCodeLanguage(name);
		if (!desc) continue;
		let support = desc.support;
		if (!support) {
			try {
				support = await desc.load();
			} catch {
				continue; // stays plain monospace
			}
			if (!isCurrent()) return;
		}
		code.replaceChildren(highlightText(code.textContent ?? '', support.language));
	}
}
