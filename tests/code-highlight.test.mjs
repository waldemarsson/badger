import './helpers/lib-resolve.mjs';
import assert from 'node:assert/strict';
import test from 'node:test';
import { markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';

const { isCodeLanguage } = await import('../src/lib/editor/theme.ts');

const topType = (language) => language.parser.parse('').type;
const load = (name) => languages.find((d) => d.name === name).load();

test('code colors apply to fenced-code languages, never to markdown', async () => {
	assert.equal(isCodeLanguage(topType(markdownLanguage)), false);
	const rust = await load('Rust');
	assert.equal(isCodeLanguage(topType(rust.language)), true);
	assert.equal(isCodeLanguage(topType(rust.language)), true); // cached
});

test('a ```md block loading Markdown does not turn the note into code', async () => {
	const md = await load('Markdown');
	assert.equal(isCodeLanguage(topType(md.language)), false);
	assert.equal(isCodeLanguage(topType(markdownLanguage)), false);
});

test('calc blocks get code colors', async () => {
	const { calcLanguage } = await import('../src/lib/editor/calc-language.ts');
	assert.equal(isCodeLanguage(topType(calcLanguage)), true);
});
