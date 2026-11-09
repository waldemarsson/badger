import assert from 'node:assert/strict';
import test from 'node:test';

const { togglesTask } = await import('../src/lib/editor/task-keys.ts');

const key = (k, mods = {}) => ({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, isComposing: false, ...mods });

test('Space and Enter toggle a focused task checkbox', () => {
	assert.equal(togglesTask(key(' ')), true);
	assert.equal(togglesTask(key('Enter')), true);
});

test('other keys, modifiers and IME composition do not toggle', () => {
	for (const k of ['Tab', 'a', 'x', 'Escape', 'ArrowDown', 'Spacebar']) assert.equal(togglesTask(key(k)), false, k);
	for (const mod of ['metaKey', 'ctrlKey', 'altKey', 'shiftKey', 'isComposing']) {
		assert.equal(togglesTask(key(' ', { [mod]: true })), false, mod);
		assert.equal(togglesTask(key('Enter', { [mod]: true })), false, mod);
	}
});
