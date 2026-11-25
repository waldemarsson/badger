// Syntax colors for ```calc blocks (src/lib/calc.ts): names, numbers, operators, `sum`/`of`,
// labels and comments, as a small StreamLanguage. `matchCodeLanguage` picks a fenced-code
// language (calc, or language-data's) for the editor and the read view's highlighting.
import { LanguageDescription, LanguageSupport, StreamLanguage, type StringStream } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { Tag, tagHighlighter, tags } from '@lezer/highlight';
import { isCalcInfo } from '#lib/calc.js';

/** Calc-only tags, so their colors never restyle other languages' variables and operators. */
export const calcTags = {
	name: Tag.define(tags.variableName),
	operator: Tag.define(tags.operator),
	/** A word after a number or `)` and a space, like `kr` in `1200 kr`: read as a label. */
	label: Tag.define(tags.labelName)
};

interface CalcState {
	/** The line's previous token ended an operand (a number or `)`). */
	afterOperand: boolean;
}

const NUMBER = /^(?:\d{1,3}(?:[ \u00a0\u202f]\d{3}(?!\d))+|\d+)(?:[.,]\d+)*/;
const NAME = /^[\p{L}_][\p{L}\d_]*/u;

function token(stream: StringStream, state: CalcState): string | null {
	if (stream.sol()) {
		state.afterOperand = false;
		if (stream.match(/^\s*(?:\/\/|#)/)) {
			stream.skipToEnd();
			return 'comment';
		}
	}
	if (stream.eatSpace()) return null;
	if (stream.pos > 0 && /\s/.test(stream.string[stream.pos - 1]) && stream.match('//')) {
		stream.skipToEnd();
		return 'comment';
	}
	const afterOperand = state.afterOperand;
	state.afterOperand = false;
	if (stream.match(NUMBER)) {
		state.afterOperand = true;
		return 'number';
	}
	const name = stream.match(NAME);
	if (name) {
		const word = name === true ? '' : name[0];
		if (word === 'sum' || word === 'of') return 'keyword';
		if (!afterOperand) return 'calcName';
		// A label needs a space before it; glued on (`1e3`, `2x`) it is an error.
		return /\s/.test(stream.string[stream.start - 1] ?? '') ? 'calcLabel' : 'invalid';
	}
	if (stream.match(/^[-+−*/^%=()]/)) {
		state.afterOperand = stream.current() === ')';
		return 'calcOperator';
	}
	if (stream.match(/^\p{Sc}/u)) return 'unit';
	stream.next();
	return 'invalid';
}

export const calcLanguage = StreamLanguage.define<CalcState>({
	name: 'calc',
	startState: () => ({ afterOperand: false }),
	copyState: (state) => ({ ...state }),
	token,
	tokenTable: { calcName: calcTags.name, calcOperator: calcTags.operator, calcLabel: calcTags.label },
	languageData: { commentTokens: { line: '//' } }
});

export const calcDescription = LanguageDescription.of({ name: 'calc', support: new LanguageSupport(calcLanguage) });

/**
 * The language for a fenced block's info string: calc only for the first word `calc` (any
 * case, as isCalcInfo), never by language-data's fuzzy match (`calculator`, `calcx`);
 * otherwise language-data's match.
 */
export function matchCodeLanguage(info: string): LanguageDescription | null {
	if (isCalcInfo(info)) return calcDescription;
	return LanguageDescription.matchLanguageName(languages, info, true);
}

/** Marks number tokens `calc-num` (read view: a grouped number never wraps at its spaces). */
export const calcNumbers = tagHighlighter([{ tag: tags.number, class: 'calc-num' }]);
