import assert from 'node:assert/strict';
import test from 'node:test';

const { PAD_ID, isBlank, panelKeyAction } = await import('../src/lib/scratchpad.ts');
const { SCRATCHPAD_SHORTCUT, togglesScratchpad, DEFAULT_SHORTCUT } = await import('../src/lib/palette.ts');

const key = (code, mods = {}) => ({
	code,
	key: code === 'Escape' ? 'Escape' : code.replace(/^Key/, '').toLowerCase(),
	metaKey: false,
	ctrlKey: false,
	altKey: false,
	shiftKey: false,
	defaultPrevented: false,
	repeat: false,
	isComposing: false,
	...mods
});

const idle = { paletteOpen: false, recording: false, paletteShortcut: DEFAULT_SHORTCUT };

test('the pad uses a fixed pseudo-id that no real note has', () => {
	assert.ok(Number.isInteger(PAD_ID) && PAD_ID <= 0);
});

test('isBlank treats empty and whitespace-only pads as blank', () => {
	for (const t of ['', ' ', '\n\n', '\t \r\n', ' ']) assert.equal(isBlank(t), true, JSON.stringify(t));
	for (const t of ['x', '  #  ', '\n-\n']) assert.equal(isBlank(t), false, JSON.stringify(t));
});

test('⌘J toggles the scratchpad', () => {
	assert.equal(SCRATCHPAD_SHORTCUT, 'Meta+J');
	assert.equal(togglesScratchpad(key('KeyJ', { metaKey: true }), idle), true);
	assert.equal(togglesScratchpad(key('KeyJ'), idle), false);
	assert.equal(togglesScratchpad(key('KeyJ', { metaKey: true, shiftKey: true }), idle), false);
	assert.equal(togglesScratchpad(key('KeyK', { metaKey: true }), idle), false);
});

test('⌘J is ignored while the palette is open or recording, and when ⌘J is the palette shortcut', () => {
	const cmdJ = key('KeyJ', { metaKey: true });
	assert.equal(togglesScratchpad(cmdJ, { ...idle, paletteOpen: true }), false);
	assert.equal(togglesScratchpad(cmdJ, { ...idle, recording: true }), false);
	assert.equal(togglesScratchpad(cmdJ, { ...idle, paletteShortcut: 'Meta+J' }), false);
});

test('panelKeyAction: Esc closes unless something inside claimed it', () => {
	assert.equal(panelKeyAction(key('Escape')), 'close');
	assert.equal(panelKeyAction(key('Escape', { defaultPrevented: true })), null);
	assert.equal(panelKeyAction(key('Escape', { isComposing: true })), null);
	assert.equal(panelKeyAction(key('Escape', { shiftKey: true })), null);
});

test('panelKeyAction: ⌘E toggles the read view; other keys are not the drawer’s', () => {
	assert.equal(panelKeyAction(key('KeyE', { metaKey: true })), 'toggle-read');
	assert.equal(panelKeyAction(key('KeyE', { metaKey: true, repeat: true })), 'repeat');
	assert.equal(panelKeyAction(key('KeyE', { metaKey: true, shiftKey: true })), null);
	assert.equal(panelKeyAction(key('KeyE')), null);
	assert.equal(panelKeyAction(key('KeyN', { metaKey: true })), null);
	assert.equal(panelKeyAction(key('KeyF', { metaKey: true })), null);
});
