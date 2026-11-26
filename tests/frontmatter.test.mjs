import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const { bodyStart, frontmatterTagSpans, frontmatterAliases } = await import('../src/lib/frontmatter.ts');

// Shared with the Rust parser (src-tauri/src/frontmatter.rs).
const cases = JSON.parse(readFileSync(new URL('./fixtures/frontmatter-cases.json', import.meta.url), 'utf8'));

for (const c of cases) {
	test(`frontmatter: ${c.name}`, () => {
		assert.equal(c.md.slice(bodyStart(c.md)), c.body);
		assert.deepEqual(
			frontmatterTagSpans(c.md).map((s) => c.md.slice(s.from, s.to)),
			c.tags
		);
		assert.deepEqual(frontmatterAliases(c.md), c.aliases);
	});
}
