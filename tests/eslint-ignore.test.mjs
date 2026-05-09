import assert from 'node:assert/strict';
import test from 'node:test';
import { ESLint } from 'eslint';

test('ESLint ignores generated Rust build output under target/', async () => {
	const eslint = new ESLint();

	const ignored = await eslint.isPathIgnored(
		'target/release/build/time-tracker-example/out/__global-api-script.js'
	);

	assert.equal(ignored, true);
});
