import assert from 'node:assert/strict';
import test from 'node:test';

const { isThemeMode, toThemeMode, resolveTheme } = await import('../src/lib/theme.ts');

test('resolveTheme follows the OS only in system mode', () => {
	assert.equal(resolveTheme('system', true), 'dark');
	assert.equal(resolveTheme('system', false), 'light');
	assert.equal(resolveTheme('light', true), 'light');
	assert.equal(resolveTheme('dark', false), 'dark');
});

test('isThemeMode accepts only the three modes', () => {
	for (const mode of ['system', 'light', 'dark']) assert.equal(isThemeMode(mode), true);
	for (const value of ['', 'Dark', 'sepia', null, undefined, 1]) assert.equal(isThemeMode(value), false);
});

test('toThemeMode falls back to system for unknown values', () => {
	assert.equal(toThemeMode('dark'), 'dark');
	assert.equal(toThemeMode(null), 'system');
	assert.equal(toThemeMode('sepia'), 'system');
});
