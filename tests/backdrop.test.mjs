import { test } from 'node:test';
import assert from 'node:assert/strict';
import { backdropDismiss } from '../src/lib/backdrop.ts';

const backdrop = {};
const inside = {};
const on = (target) => ({ target, currentTarget: backdrop });

test('a press and release on the backdrop closes', () => {
	let closed = 0;
	const b = backdropDismiss(() => closed++);
	b.onpointerdown(on(backdrop));
	b.onclick(on(backdrop));
	assert.equal(closed, 1);
});

test('a press inside released on the backdrop (text selection) does not close', () => {
	let closed = 0;
	const b = backdropDismiss(() => closed++);
	b.onpointerdown(on(inside));
	b.onclick(on(backdrop));
	assert.equal(closed, 0);
});

test('a press on the backdrop released inside does not close', () => {
	let closed = 0;
	const b = backdropDismiss(() => closed++);
	b.onpointerdown(on(backdrop));
	b.onclick(on(inside));
	assert.equal(closed, 0);
});

test('a click without a preceding backdrop press does not close', () => {
	let closed = 0;
	const b = backdropDismiss(() => closed++);
	b.onpointerdown(on(backdrop));
	b.onclick(on(inside));
	b.onclick(on(backdrop));
	assert.equal(closed, 0);
});
