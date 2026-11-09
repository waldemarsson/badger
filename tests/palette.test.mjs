import assert from 'node:assert/strict';
import test from 'node:test';

const {
	DEFAULT_SHORTCUT,
	shortcutFromEvent,
	validateShortcut,
	normalizeShortcut,
	matchesShortcut,
	formatShortcut,
	filterItems,
	parseHours,
	isValidDate,
	shiftDate,
	initialIndex
} = await import('../src/lib/palette.ts');

const key = (code, mods = {}) => ({ code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods });

test('shortcutFromEvent builds canonical strings from physical keys', () => {
	assert.equal(shortcutFromEvent(key('KeyK', { metaKey: true })), 'Meta+K');
	assert.equal(shortcutFromEvent(key('Digit1', { ctrlKey: true, shiftKey: true })), 'Ctrl+Shift+1');
	assert.equal(shortcutFromEvent(key('Space', { altKey: true })), 'Alt+Space');
	assert.equal(shortcutFromEvent(key('KeyP', { shiftKey: true, metaKey: true, altKey: true })), 'Meta+Alt+Shift+P');
});

test('shortcutFromEvent ignores bare modifier presses', () => {
	assert.equal(shortcutFromEvent(key('MetaLeft', { metaKey: true })), null);
	assert.equal(shortcutFromEvent(key('ShiftRight', { shiftKey: true })), null);
	assert.equal(shortcutFromEvent(key('')), null);
});

test('matchesShortcut requires the exact modifier set', () => {
	assert.equal(matchesShortcut(key('KeyK', { metaKey: true }), 'Meta+K'), true);
	assert.equal(matchesShortcut(key('KeyK', { metaKey: true, shiftKey: true }), 'Meta+K'), false);
	assert.equal(matchesShortcut(key('KeyK'), 'Meta+K'), false);
});

test('validateShortcut accepts combos with ⌘, ⌃ or ⌥', () => {
	for (const s of ['Meta+K', 'Ctrl+Space', 'Alt+Shift+P', 'Meta+Shift+Z']) assert.equal(validateShortcut(s), null, s);
});

test('validateShortcut refuses bare keys, shift-only and reserved combos', () => {
	assert.match(validateShortcut('K'), /Include/);
	assert.match(validateShortcut('Shift+K'), /Include/);
	assert.match(validateShortcut('Meta+Shift'), /Press a key/);
	assert.match(validateShortcut('Hyper+K'), /Unrecognised/);
	for (const s of ['Meta+Q', 'Meta+W', 'Meta+H', 'Meta+M', 'Meta+C', 'Meta+V', 'Meta+X', 'Meta+A', 'Meta+Z', 'Meta+Comma']) {
		assert.match(validateShortcut(s), /reserved by macOS/, s);
	}
});

test('normalizeShortcut falls back to ⌘K for missing or corrupt values', () => {
	assert.equal(normalizeShortcut('Ctrl+Space'), 'Ctrl+Space');
	assert.equal(normalizeShortcut(undefined), DEFAULT_SHORTCUT);
	assert.equal(normalizeShortcut('garbage'), DEFAULT_SHORTCUT);
	assert.equal(normalizeShortcut('Meta+Q'), DEFAULT_SHORTCUT);
});

test('formatShortcut uses macOS symbols and order', () => {
	assert.equal(formatShortcut('Meta+K'), '⌘K');
	assert.equal(formatShortcut('Meta+Shift+K'), '⇧⌘K');
	assert.equal(formatShortcut('Ctrl+Alt+Space'), '⌃⌥Space');
	assert.equal(formatShortcut('Meta+Comma'), '⌘,');
});

test('filterItems matches every token case-insensitively, including å/ä/ö', () => {
	const items = ['Acme / Web', 'Acme / Mobile', 'Öresund / Bro', 'Globex / Web'];
	assert.deepEqual(filterItems(items, '', (s) => s), items);
	assert.deepEqual(filterItems(items, 'acme web', (s) => s), ['Acme / Web']);
	assert.deepEqual(filterItems(items, 'WEB', (s) => s), ['Acme / Web', 'Globex / Web']);
	assert.deepEqual(filterItems(items, 'öre', (s) => s), ['Öresund / Bro']);
	assert.deepEqual(filterItems(items, '   ', (s) => s), items);
	assert.deepEqual(filterItems(items, 'nope', (s) => s), []);
});

test('parseHours accepts plain decimals in (0, 24]', () => {
	assert.deepEqual(parseHours('2'), { hours: 2 });
	assert.deepEqual(parseHours(' 1.5 '), { hours: 1.5 });
	assert.deepEqual(parseHours('.5'), { hours: 0.5 });
	assert.deepEqual(parseHours('24'), { hours: 24 });
});

test('parseHours rejects empty, non-numeric, zero and too large', () => {
	assert.match(parseHours('').error, /required/);
	assert.match(parseHours('  ').error, /required/);
	assert.match(parseHours('abc').error, /number/);
	assert.match(parseHours('1:30').error, /number/);
	assert.match(parseHours('1,5').error, /number/);
	assert.match(parseHours('-1').error, /number/);
	assert.match(parseHours('0').error, /greater than 0/);
	assert.match(parseHours('24.5').error, /exceed 24/);
});

test('isValidDate checks format and calendar', () => {
	assert.equal(isValidDate('2026-11-05'), true);
	assert.equal(isValidDate('2024-02-29'), true);
	assert.equal(isValidDate('2026-02-29'), false);
	assert.equal(isValidDate('2026-13-01'), false);
	assert.equal(isValidDate('2026-1-5'), false);
	assert.equal(isValidDate('yesterday'), false);
});

test('shiftDate crosses month and year boundaries', () => {
	assert.equal(shiftDate('2026-11-05', -1), '2026-11-04');
	assert.equal(shiftDate('2026-03-01', -1), '2026-02-28');
	assert.equal(shiftDate('2026-12-31', 1), '2027-01-01');
});

test('initialIndex prefers ids in order and falls back to 0', () => {
	const items = [{ id: 1 }, { id: 2 }, { id: 3 }];
	assert.equal(initialIndex(items, 3, 2), 2);
	assert.equal(initialIndex(items, 99, 2), 1);
	assert.equal(initialIndex(items, null, undefined), 0);
	assert.equal(initialIndex([], 1), 0);
});

test('the recorder rejects ⌘J (the scratchpad) on top of validateShortcut', async () => {
	const { recorderProblem } = await import('../src/lib/palette.ts');
	assert.equal(recorderProblem('Meta+J'), '⌘J opens the Scratchpad');
	assert.match(recorderProblem('Meta+Q'), /reserved by macOS/);
	assert.match(recorderProblem('K'), /Include/);
	for (const s of ['Meta+K', 'Meta+Shift+J', 'Ctrl+J', 'Alt+J']) assert.equal(recorderProblem(s), null, s);
});

test('a saved ⌘J palette shortcut survives load (only the recorder rejects it)', () => {
	assert.equal(validateShortcut('Meta+J'), null);
	assert.equal(normalizeShortcut('Meta+J'), 'Meta+J');
});
