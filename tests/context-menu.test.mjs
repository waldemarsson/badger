import assert from 'node:assert/strict';
import test from 'node:test';

const { placeMenu } = await import('../src/lib/context-menu.ts');

// A 200×150 menu in a 1000×800 window with an 8px margin.
const W = 200;
const H = 150;
const VW = 1000;
const VH = 800;
const M = 8;

test('placeMenu opens down-right of the point when it fits', () => {
	assert.deepEqual(placeMenu(100, 120, W, H, VW, VH, M), { left: 100, top: 120 });
	// Exactly touching the margin still fits.
	assert.deepEqual(placeMenu(VW - W - M, VH - H - M, W, H, VW, VH, M), { left: VW - W - M, top: VH - H - M });
});

test('placeMenu flips left near the right edge', () => {
	assert.deepEqual(placeMenu(950, 120, W, H, VW, VH, M), { left: 750, top: 120 });
});

test('placeMenu flips up near the bottom edge', () => {
	assert.deepEqual(placeMenu(100, 750, W, H, VW, VH, M), { left: 100, top: 600 });
});

test('placeMenu flips both ways in the bottom-right corner', () => {
	assert.deepEqual(placeMenu(990, 790, W, H, VW, VH, M), { left: 790, top: 640 });
});

test('placeMenu shifts back inside when a flip would leave the viewport', () => {
	// Near the right edge of a narrow window: flipping left would cross the left margin.
	assert.deepEqual(placeMenu(150, 120, W, H, 300, VH, M), { left: M, top: 120 });
	// Near the bottom of a short window: flipping up would cross the top margin.
	assert.deepEqual(placeMenu(100, 100, W, H, VW, 200, M), { left: 100, top: M });
});

test('placeMenu clamps a point outside the viewport', () => {
	assert.deepEqual(placeMenu(-40, -10, W, H, VW, VH, M), { left: M, top: M });
	assert.deepEqual(placeMenu(1200, 900, W, H, VW, VH, M), { left: 1000 - W - M, top: 800 - H - M });
});

test('placeMenu pins to the margin when the window is smaller than the menu', () => {
	assert.deepEqual(placeMenu(50, 40, W, H, 150, 100, M), { left: M, top: M });
	assert.deepEqual(placeMenu(50, 40, W, H, 150, 100, 0), { left: 0, top: 0 });
});

test('placeMenu flips up above an anchor row instead of covering it', () => {
	// Keyboard-opened: the point is the row's bottom-left; the row is 32px tall.
	assert.deepEqual(placeMenu(100, 780, W, H, VW, VH, M, 32), { left: 100, top: 780 - 32 - H });
	// Fits below: the anchor height changes nothing.
	assert.deepEqual(placeMenu(100, 300, W, H, VW, VH, M, 32), { left: 100, top: 300 });
});

test('placeMenu defaults the margin to 8px', () => {
	assert.deepEqual(placeMenu(1200, 900, W, H, VW, VH), { left: VW - W - 8, top: VH - H - 8 });
});
