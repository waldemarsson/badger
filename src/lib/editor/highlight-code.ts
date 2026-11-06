// Syntax colors for the read view's code blocks: the editor's parsers (language-data)
// and its code colors, applied to the `<pre><code class="language-x">` Rust renders.
import { LanguageDescription } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { highlightCode } from '@lezer/highlight';
import { codeHighlight } from '#lib/editor/theme.js';

/**
 * Highlights every `code[class^="language-"]` under `root` in place. Stops when
 * `isCurrent()` turns false (the content was replaced while a language loaded).
 */
export async function highlightCodeBlocks(root: HTMLElement, isCurrent: () => boolean): Promise<void> {
	for (const code of root.querySelectorAll<HTMLElement>('pre > code[class^="language-"]')) {
		const name = code.className.slice('language-'.length);
		const desc = LanguageDescription.matchLanguageName(languages, name, true);
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
		const text = code.textContent ?? '';
		const tree = support.language.parser.parse(text);
		const out = document.createDocumentFragment();
		highlightCode(
			text,
			tree,
			codeHighlight,
			(piece, classes) => {
				if (!classes) return void out.append(piece);
				const span = document.createElement('span');
				span.className = classes;
				span.textContent = piece;
				out.append(span);
			},
			() => out.append('\n')
		);
		code.replaceChildren(out);
	}
}
