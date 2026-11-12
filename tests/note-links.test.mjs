import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const {
	extractRefs,
	slug,
	tagTerm,
	splitSearchQuery,
	tagMatches,
	entityTag,
	tagTrigger,
	wikilinkTrigger,
	tagCompletion,
	wikilinkCompletion,
	filterTagOptions,
	tagPreselect,
	wikilinkOptions,
	indexLinkTargets,
	chipFor
} = await import('../src/lib/note-links.ts');

// Shared with the Rust extractor (src-tauri/src/note_links.rs).
const cases = JSON.parse(readFileSync(new URL('./fixtures/note-links-cases.json', import.meta.url), 'utf8'));

const actual = (md) =>
	extractRefs(md).map((r) => {
		const out = { kind: r.kind, target: r.target, text: md.slice(r.from, r.to) };
		if (r.kind === 'link') out.title = md.slice(r.targetFrom, r.targetTo);
		if (r.alias !== null) out.alias = r.alias;
		return out;
	});

for (const c of cases.extract) {
	test(`extractRefs: ${c.name}`, () => {
		assert.deepEqual(actual(c.md), c.refs);
	});
}

test('slug matches the shared case table', () => {
	for (const [name, expected] of cases.slug) assert.equal(slug(name), expected, name);
});

test('tag target range excludes the #', () => {
	const md = 'x #Acme/Web.';
	const [r] = extractRefs(md);
	assert.equal(md.slice(r.from, r.to), '#Acme/Web');
	assert.equal(md.slice(r.targetFrom, r.targetTo), 'Acme/Web');
});

test('offsets are string indices with astral characters before them', () => {
	const md = '🎉 #tag [[Fête 🎉]]';
	assert.deepEqual(actual(md), [
		{ kind: 'tag', target: 'tag', text: '#tag' },
		{ kind: 'link', target: 'fête 🎉', text: '[[Fête 🎉]]', title: 'Fête 🎉' }
	]);
});

test('tagTerm and splitSearchQuery separate #tags from text', () => {
	assert.equal(tagTerm('#Acme'), 'acme');
	assert.equal(tagTerm('#acme/web,'), 'acme/web');
	assert.equal(tagTerm('#123'), null);
	assert.equal(tagTerm('#'), null);
	assert.equal(tagTerm('acme'), null);
	assert.deepEqual(splitSearchQuery('  budget #Acme  q3 #x/y #12 '), {
		tags: ['acme', 'x/y'],
		text: 'budget q3 #12'
	});
	assert.deepEqual(splitSearchQuery('#acme'), { tags: ['acme'], text: '' });
});

test('splitSearchQuery dedupes tags case-insensitively', () => {
	assert.deepEqual(splitSearchQuery('#Acme x #acme #ACME, #b'), { tags: ['acme', 'b'], text: 'x' });
});

test('tagMatches is a nested prefix match', () => {
	assert.ok(tagMatches('acme', 'acme'));
	assert.ok(tagMatches('acme/web', 'acme'));
	assert.ok(!tagMatches('acmeco', 'acme'));
	assert.ok(!tagMatches('acme', 'acme/web'));
});

test('entityTag recognises issue and todo chips', () => {
	assert.deepEqual(entityTag('issue/12'), { kind: 'issue', id: 12 });
	assert.deepEqual(entityTag('todo/3'), { kind: 'todo', id: 3 });
	assert.equal(entityTag('issue/x'), null);
	assert.equal(entityTag('issue/12/a'), null);
	assert.equal(entityTag('acme'), null);
	assert.equal(entityTag('issue/99999999999999999999'), null);
});

test('tagTrigger opens after # and a tag char at a tag boundary', () => {
	assert.deepEqual(tagTrigger('#a'), { from: 0, query: 'a' });
	assert.deepEqual(tagTrigger('see #acme/w'), { from: 4, query: 'acme/w' });
	assert.deepEqual(tagTrigger('(#Å'), { from: 1, query: 'Å' });
	assert.deepEqual(tagTrigger('- [ ] #12'), { from: 6, query: '12' });
	for (const before of ['#', '# ', '# Heading', '##a', 'a#b', 'x/#a', 'see #', '`#code', 'https://x.se/(#a', '\\#a']) {
		assert.equal(tagTrigger(before), null, before);
	}
});

test('wikilinkTrigger opens after [[ until the title is closed', () => {
	assert.deepEqual(wikilinkTrigger('[['), { from: 0, query: '' });
	assert.deepEqual(wikilinkTrigger('see [[Meet'), { from: 4, query: 'Meet' });
	assert.deepEqual(wikilinkTrigger('a [[Å b'), { from: 2, query: 'Å b' });
	for (const before of ['[', '[[a]]', '[[a|', '[[a#', '![[a', '\\[[a', '`[[a', '[[[a']) {
		assert.equal(wikilinkTrigger(before), null, before);
	}
});

test('completions insert the full tag and a closed wikilink once', () => {
	assert.equal(tagCompletion('acme/web'), '#acme/web');
	assert.deepEqual(wikilinkCompletion('Meeting notes', ''), { insert: '[[Meeting notes]]', consumeAfter: 0 });
	assert.deepEqual(wikilinkCompletion('A', ']] more'), { insert: '[[A]]', consumeAfter: 2 });
	assert.deepEqual(wikilinkCompletion('A', '] x'), { insert: '[[A]]', consumeAfter: 1 });
});

const option = (kind, tag, label, detail = null, status = null) => ({ kind, tag, label, detail, status });

test('filterTagOptions ranks tag prefix, then word starts, then substrings', () => {
	const options = [
		option('tag', 'meeting', 'meeting'),
		option('customer', 'acme', 'Acme'),
		option('project', 'acme/website', 'Website', 'Acme'),
		option('customer', 'bigcorp', 'BigCorp'),
		option('issue', 'issue/12', 'Login fails — Acme / Website'),
		option('todo', 'todo/3', 'Call Åsa about the website', null, 'in_progress')
	];
	const tags = (q) => filterTagOptions(options, q).map((o) => o.tag);
	assert.deepEqual(tags('acme'), ['acme', 'acme/website', 'issue/12']);
	assert.deepEqual(tags('web'), ['acme/website', 'issue/12', 'todo/3']);
	assert.deepEqual(tags('Acme/W'), ['acme/website']);
	assert.deepEqual(tags('login'), ['issue/12']);
	assert.deepEqual(tags('åsa'), ['todo/3']);
	assert.deepEqual(tags('eet'), ['meeting']);
	assert.deepEqual(tags('issue/1'), ['issue/12']);
	assert.deepEqual(tags('zzz'), []);
	assert.equal(filterTagOptions(options, '', 2).length, 2);
});

test('filterTagOptions ranks an exact tag match first', () => {
	const options = [
		option('tag', 'acme-old', 'acme-old'),
		option('project', 'acme/web', 'Web', 'Acme'),
		option('customer', 'acme', 'Acme')
	];
	const tags = (q) => filterTagOptions(options, q).map((o) => o.tag);
	assert.deepEqual(tags('acme'), ['acme', 'acme-old', 'acme/web']);
	assert.deepEqual(tags('ACME'), ['acme', 'acme-old', 'acme/web']);
	assert.deepEqual(tags('acm'), ['acme-old', 'acme/web', 'acme']);
});

test('tagPreselect skips preselection when the typed tag is the top option', () => {
	assert.equal(tagPreselect('acm', 'acme'), 0);
	assert.equal(tagPreselect('acme', 'acme'), -1);
	assert.equal(tagPreselect('Acme', 'acme'), -1);
	assert.equal(tagPreselect('acme', 'acme/web'), 0);
	assert.equal(tagPreselect('acme', undefined), 0);
});

test('wikilinkOptions lists matching titles and offers create without an exact match', () => {
	const notes = [{ id: 1, title: 'Weekly sync' }, { id: 2, title: 'Sync log' }, { id: 3, title: 'Async ideas' }, { id: 4, title: 'Plan' }];
	const titles = (q) => wikilinkOptions(notes, q).notes.map((n) => n.title);
	assert.deepEqual(titles('sync'), ['Sync log', 'Weekly sync', 'Async ideas']);
	assert.equal(wikilinkOptions(notes, 'sync').create, 'sync');
	assert.equal(wikilinkOptions(notes, ' plan ').create, null);
	assert.deepEqual(titles('plan'), ['Plan']);
	assert.equal(wikilinkOptions(notes, '').create, null);
	assert.deepEqual(titles(''), ['Async ideas', 'Plan', 'Sync log', 'Weekly sync']);
	assert.deepEqual(wikilinkOptions(notes, 'New idea'), { notes: [], create: 'New idea' });
});

test('chipFor resolves known issue and todo ids only', () => {
	const index = indexLinkTargets({
		notes: [{ id: 7, title: 'Plan Å' }],
		issues: [{ id: 12, title: 'Login fails', context: 'Acme / Web' }],
		todos: [{ id: 3, title: 'Call Åsa', status: 'done' }]
	});
	assert.equal(index.notes.get('plan å'), 7);
	assert.deepEqual(chipFor('issue/12', index), { kind: 'issue', id: 12, title: 'Login fails', status: null, detail: 'Acme / Web' });
	assert.deepEqual(chipFor('todo/3', index), { kind: 'todo', id: 3, title: 'Call Åsa', status: 'done', detail: null });
	assert.equal(chipFor('todo/4', index), null);
	assert.equal(chipFor('issue/3', index), null);
	assert.equal(chipFor('acme', index), null);
});

// ── extractRefs with ranges (the editor scans only its visible lines) ──

/** The full result's refs on the lines that [from, to] touches. */
const refsOnLines = (md, from, to) => {
	const start = md.lastIndexOf('\n', from - 1) + 1;
	const nl = md.indexOf('\n', to);
	const end = nl < 0 ? md.length : nl;
	return extractRefs(md).filter((r) => r.from >= start && r.to <= end);
};

test('extractRefs with ranges equals the full result on those lines (every case, every line pair)', () => {
	for (const c of cases.extract) {
		const md = c.md;
		const starts = [0];
		for (let i = 0; i < md.length; i++) if (md[i] === '\n') starts.push(i + 1);
		for (let a = 0; a < starts.length; a++) {
			for (let b = a; b < starts.length; b++) {
				// Mid-line positions on purpose: ranges widen to whole lines.
				const from = Math.min(md.length, starts[a] + 1);
				const to = Math.min(md.length, starts[b] + 2);
				assert.deepEqual(extractRefs(md, undefined, [{ from, to }]), refsOnLines(md, from, to), `${c.name} [${from}, ${to}]`);
			}
		}
	}
});

test('extractRefs with several unsorted, overlapping ranges merges them', () => {
	const md = ['#a [[One]]', '```', '#code [[no]]', '```', '#b', 'text', '#c [[Two|alias]]'].join('\n');
	const all = extractRefs(md);
	const line = (n) => md.split('\n').slice(0, n).join('\n').length + (n ? 1 : 0); // start of line n (0-based)
	const ranges = [
		{ from: line(6), to: md.length },
		{ from: line(0), to: line(0) + 3 },
		{ from: line(2) + 1, to: line(4) },
		{ from: line(4), to: line(4) }
	];
	assert.deepEqual(
		extractRefs(md, undefined, ranges).map((r) => md.slice(r.from, r.to)),
		['#a', '[[One]]', '#b', '#c', '[[Two|alias]]']
	);
	assert.deepEqual(extractRefs(md, undefined, [{ from: 0, to: md.length }]), all);
	assert.deepEqual(extractRefs(md, undefined, []), []);
	assert.deepEqual(extractRefs('', undefined, [{ from: 0, to: 0 }]), []);
});

test('extractRefs with ranges: a code block or reference definition outside the range still applies', () => {
	const fence = ['```', 'x', '#inside [[in]]', '```', '#after'].join('\n');
	const from = fence.indexOf('#inside');
	assert.deepEqual(extractRefs(fence, undefined, [{ from, to: fence.length }]).map((r) => fence.slice(r.from, r.to)), ['#after']);

	const ref = ['[[Foo]] and [Bar] #t', '', 'filler', '', '[Foo]: https://a.se'].join('\n');
	const refs = extractRefs(ref, undefined, [{ from: 0, to: 3 }]).map((r) => ref.slice(r.from, r.to));
	assert.deepEqual(refs, ['#t'], 'defined [Foo] is a real link, so [[Foo]] is not a wikilink');
});

test('reference definitions inside lists and quotes are found', () => {
	for (const def of ['- [Foo]: https://a.se', '> [Foo]: https://a.se', '1. item\n\n   [Foo]: https://a.se']) {
		const md = `[[Foo]] #t\n\n${def}`;
		assert.deepEqual(actual(md), [{ kind: 'tag', target: 't', text: '#t' }], def);
	}
});
