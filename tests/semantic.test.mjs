import assert from 'node:assert/strict';
import test from 'node:test';

const {
	qualifiesForSemantic,
	shouldApply,
	keepActiveIndex,
	downloadPercent,
	semanticStatusLabel,
	semanticReady,
	semanticHintVisible,
	MIN_SEMANTIC_CHARS,
	HYBRID_DEBOUNCE_MS
} = await import('../src/lib/semantic.ts');

const status = (extra = {}) => ({
	state: 'off',
	enabled: false,
	downloaded_bytes: 0,
	download_bytes: 295_730_322,
	indexing_done: 0,
	indexing_total: 0,
	error: null,
	model_name: 'multilingual-e5-base (int8)',
	model_bytes: 0,
	indexed_notes: 0,
	notes: 12,
	index_bytes: 0,
	...extra
});

test('constants match the brief', () => {
	assert.equal(MIN_SEMANTIC_CHARS, 4);
	assert.equal(HYBRID_DEBOUNCE_MS, 250);
});

test('qualifiesForSemantic: 4+ characters (code points), trimmed', () => {
	assert.equal(qualifiesForSemantic('abc'), false);
	assert.equal(qualifiesForSemantic('  abc  '), false);
	assert.equal(qualifiesForSemantic(''), false);
	assert.equal(qualifiesForSemantic('plan'), true);
	assert.equal(qualifiesForSemantic('åäö!'), true);
	assert.equal(qualifiesForSemantic('🚀🚀🚀'), false, 'emoji count once each');
});

test('shouldApply: latest search only; hybrid results are not replaced by a late keyword response', () => {
	assert.equal(shouldApply(2, null, { seq: 1, hybrid: false }), false, 'stale search');
	assert.equal(shouldApply(2, null, { seq: 2, hybrid: false }), true);
	assert.equal(shouldApply(2, { seq: 2, hybrid: false }, { seq: 2, hybrid: true }), true, 'hybrid merges in');
	assert.equal(shouldApply(2, { seq: 2, hybrid: true }, { seq: 2, hybrid: false }), false, 'late keyword');
	assert.equal(shouldApply(3, { seq: 2, hybrid: true }, { seq: 3, hybrid: false }), true, 'new search');
	assert.equal(shouldApply(3, { seq: 3, hybrid: false }, { seq: 2, hybrid: true }), false, 'stale hybrid');
});

test('downloadPercent clamps and floors', () => {
	assert.equal(downloadPercent({ downloaded_bytes: 0, download_bytes: 0 }), 0);
	assert.equal(downloadPercent({ downloaded_bytes: 50, download_bytes: 200 }), 25);
	assert.equal(downloadPercent({ downloaded_bytes: 199, download_bytes: 200 }), 99);
	assert.equal(downloadPercent({ downloaded_bytes: 300, download_bytes: 200 }), 100);
});

test('semanticStatusLabel names every state', () => {
	assert.equal(semanticStatusLabel(status()), 'Off');
	assert.equal(
		semanticStatusLabel(status({ state: 'downloading', downloaded_bytes: 147_865_161 })),
		'Downloading 50%'
	);
	assert.equal(semanticStatusLabel(status({ state: 'loading' })), 'Loading model…');
	assert.equal(
		semanticStatusLabel(status({ state: 'indexing', indexing_done: 3, indexing_total: 12 })),
		'Indexing 3 of 12'
	);
	assert.equal(semanticStatusLabel(status({ state: 'ready' })), 'Ready');
	assert.equal(semanticStatusLabel(status({ state: 'error', error: 'x' })), 'Error');
});

test('semanticReady and the Settings hint', () => {
	assert.equal(semanticReady(null), false);
	assert.equal(semanticReady(status({ state: 'ready' })), true);
	assert.equal(semanticReady(status({ state: 'indexing' })), true, 'search what is indexed so far');
	assert.equal(semanticReady(status({ state: 'downloading' })), false);
	assert.equal(semanticHintVisible(null), false, 'unknown: stay quiet');
	assert.equal(semanticHintVisible(status()), true);
	assert.equal(semanticHintVisible(status({ state: 'error' })), true);
	assert.equal(semanticHintVisible(status({ state: 'downloading' })), false);
	assert.equal(semanticHintVisible(status({ state: 'ready' })), false);
});

test('keepActiveIndex keeps the active item by id when the list is replaced', () => {
	const before = ['new-todo', 'note:1', 'note:2'];
	// note:2 moved down when a related hit was fused in above it
	assert.equal(keepActiveIndex(before, ['new-todo', 'note:1', 'note:9', 'note:2'], 2), 3);
	// still at the same place
	assert.equal(keepActiveIndex(before, ['new-todo', 'note:1', 'note:2'], 1), 1);
	// gone → first row
	assert.equal(keepActiveIndex(before, ['new-todo', 'note:9'], 2), 0);
	// out-of-range active index or empty lists → first row
	assert.equal(keepActiveIndex(before, before, 7), 0);
	assert.equal(keepActiveIndex([], ['note:1'], 0), 0);
	assert.equal(keepActiveIndex(before, [], 1), 0);
});
