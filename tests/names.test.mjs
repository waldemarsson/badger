import assert from 'node:assert/strict';
import test from 'node:test';

const { byName, plural } = await import('../src/lib/names.ts');

test('byName sorts case-insensitively without mutating the input', () => {
	const input = [{ name: 'roxtec' }, { name: 'Apoteket' }, { name: 'Omegapoint' }];
	const sorted = byName(input);

	assert.deepEqual(sorted.map((x) => x.name), ['Apoteket', 'Omegapoint', 'roxtec']);
	assert.equal(input[0].name, 'roxtec');
});

test('plural pluralises everything except one', () => {
	assert.equal(plural(0, 'project'), '0 projects');
	assert.equal(plural(1, 'project'), '1 project');
	assert.equal(plural(2, 'customer'), '2 customers');
});
