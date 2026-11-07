import assert from 'node:assert/strict';
import test from 'node:test';

const {
	compareFolders,
	sortFolders,
	buildTree,
	visibleRows,
	folderPath,
	folderChain,
	descendantIds,
	moveTargets,
	ROOT_TARGET,
	targetFolderId,
	firstFreeName,
	sameIds
} = await import('../src/lib/folders.ts');

const folder = (id, name, parent_id = null) => ({ id, parent_id, name });
const note = (id, folder_id = null) => ({
	id,
	folder_id,
	title: `Note ${id}`,
	excerpt: '',
	pinned: false,
	created_at: '2026-11-01T09:00:00.000',
	updated_at: '2026-11-01T09:00:00.000'
});

// Projects (1) > Archive (2) > Deep (5); Inbox (3); empty (4) "beta"
const folders = [folder(2, 'Archive', 1), folder(1, 'Projects'), folder(3, 'inbox'), folder(4, 'Beta'), folder(5, 'Deep', 2)];
// Notes come pre-sorted (sortNotes); the tree keeps their order per folder.
const notes = [note(30, 2), note(10), note(20, 1), note(11), note(40, 99)];
const keys = (rows) => rows.map((r) => r.key);

test('compareFolders sorts A–Z case-insensitively, numbers naturally, then by id', () => {
	const sorted = sortFolders([folder(1, 'b'), folder(2, 'A'), folder(3, 'Work 10'), folder(4, 'Work 2'), folder(5, 'a')]);
	assert.deepEqual(
		sorted.map((f) => f.name),
		['A', 'a', 'b', 'Work 2', 'Work 10']
	);
	assert.ok(compareFolders(folder(1, 'x'), folder(2, 'X')) < 0, 'same name: lower id first');
	assert.deepEqual(folders.map((f) => f.id), [2, 1, 3, 4, 5], 'input not mutated');
});

test('buildTree groups folders and notes by parent; unknown parents fall back to root', () => {
	const tree = buildTree([...folders, folder(6, 'Lost', 77)], notes);
	assert.deepEqual(tree.folders.get(null).map((f) => f.name), ['Beta', 'inbox', 'Lost', 'Projects']);
	assert.deepEqual(tree.folders.get(1).map((f) => f.id), [2]);
	assert.equal(tree.folders.get(4), undefined);
	assert.deepEqual(tree.notes.get(null).map((n) => n.id), [10, 11, 40], 'keeps input order; stale folder → root');
	assert.deepEqual(tree.notes.get(2).map((n) => n.id), [30]);
	assert.equal(tree.byId.get(5).name, 'Deep');
});

test('visibleRows: folders first, then notes, nested only inside expanded folders', () => {
	const tree = buildTree(folders, notes);
	assert.deepEqual(keys(visibleRows(tree, new Set())), ['f:4', 'f:3', 'f:1', 'n:10', 'n:11', 'n:40']);

	const rows = visibleRows(tree, new Set([1, 2, 4, 5]));
	assert.deepEqual(keys(rows), ['f:4', 'e:4', 'f:3', 'f:1', 'f:2', 'f:5', 'e:5', 'n:30', 'n:20', 'n:10', 'n:11', 'n:40']);
	const byKey = new Map(rows.map((r) => [r.key, r]));
	assert.deepEqual(
		['f:1', 'f:2', 'f:5', 'e:5', 'n:30', 'n:20', 'n:10'].map((k) => [byKey.get(k).depth, byKey.get(k).parentId]),
		[[0, null], [1, 1], [2, 2], [3, 5], [2, 2], [1, 1], [0, null]]
	);
	assert.deepEqual(
		['f:4', 'f:3', 'f:1', 'f:5'].map((k) => [byKey.get(k).expanded, byKey.get(k).hasChildren]),
		[[true, false], [false, false], [true, true], [true, false]]
	);
	assert.equal(byKey.get('n:30').note.id, 30);
	assert.equal(byKey.get('f:2').folder.name, 'Archive');
});

test('visibleRows: expanding a folder inside a collapsed one shows nothing; pinned notes stay put', () => {
	const pinned = [{ ...note(50, 2), pinned: true }, ...notes];
	const rows = visibleRows(buildTree(folders, pinned), new Set([2]));
	assert.deepEqual(keys(rows), ['f:4', 'f:3', 'f:1', 'n:10', 'n:11', 'n:40']);
});

test('buildTree attaches folders in a parent cycle (legacy data) to root, with their notes', () => {
	// A (1) ⇄ B (2) form a cycle; D (4) hangs below A; C (3) is fine; E (5) is its own parent.
	const cyclic = [folder(1, 'A', 2), folder(2, 'B', 1), folder(3, 'C'), folder(4, 'D', 1), folder(5, 'E', 5)];
	const tree = buildTree(cyclic, [note(10, 1), note(20, 2), note(40, 4), note(50, 5)]);
	assert.deepEqual(tree.folders.get(null).map((f) => f.id), [1, 2, 3, 5], 'cycle members at root');
	assert.deepEqual(tree.folders.get(1).map((f) => f.id), [4], 'a folder below the cycle keeps its parent');
	assert.equal(tree.byId.get(1).parent_id, null, 'byId agrees with the tree');
	assert.equal(tree.byId.get(4).parent_id, 1);
	assert.equal(cyclic[0].parent_id, 2, 'input not mutated');
	const rows = visibleRows(tree, new Set([1, 2, 3, 4, 5]));
	assert.deepEqual(keys(rows), ['f:1', 'f:4', 'n:40', 'n:10', 'f:2', 'n:20', 'f:3', 'e:3', 'f:5', 'n:50']);
	assert.equal(folderPath(tree.byId, 4), 'A / D');
	assert.deepEqual(folderChain(tree.byId, 1), [1]);
	assert.deepEqual(moveTargets(tree).map((t) => t.label), ['Notes (root)', 'A', 'A / D', 'B', 'C', 'E']);
});

test('folderPath joins ancestors with " / "; root is empty', () => {
	const { byId } = buildTree(folders, []);
	assert.equal(folderPath(byId, 5), 'Projects / Archive / Deep');
	assert.equal(folderPath(byId, 1), 'Projects');
	assert.equal(folderPath(byId, null), '');
	assert.equal(folderPath(byId, 99), '', 'unknown folder reads as root');
});

test('folderChain lists the folder and its ancestors, innermost first', () => {
	const { byId } = buildTree(folders, []);
	assert.deepEqual(folderChain(byId, 5), [5, 2, 1]);
	assert.deepEqual(folderChain(byId, 3), [3]);
	assert.deepEqual(folderChain(byId, null), []);
	assert.deepEqual(folderChain(byId, 99), []);
});

test('descendantIds includes the folder itself and every level below', () => {
	const tree = buildTree(folders, []);
	assert.deepEqual([...descendantIds(tree, 1)].sort(), [1, 2, 5]);
	assert.deepEqual([...descendantIds(tree, 5)], [5]);
	assert.deepEqual([...descendantIds(tree, 99)], []);
});

test('moveTargets: root first, then every folder by full path in tree order, minus the moved subtree', () => {
	const tree = buildTree(folders, []);
	assert.deepEqual(moveTargets(tree), [
		{ value: ROOT_TARGET, label: 'Notes (root)' },
		{ value: 4, label: 'Beta' },
		{ value: 3, label: 'inbox' },
		{ value: 1, label: 'Projects' },
		{ value: 2, label: 'Projects / Archive' },
		{ value: 5, label: 'Projects / Archive / Deep' }
	]);
	assert.deepEqual(
		moveTargets(tree, 2).map((t) => t.value),
		[ROOT_TARGET, 4, 3, 1]
	);
});

test('targetFolderId maps picker values back to folder ids', () => {
	assert.equal(targetFolderId(ROOT_TARGET), null);
	assert.equal(targetFolderId('12'), 12);
	assert.equal(targetFolderId(''), undefined);
	assert.equal(targetFolderId('Projects'), undefined);
	assert.equal(targetFolderId('1.5'), undefined);
});

test('firstFreeName mirrors the Rust suffixing, case-insensitively', () => {
	assert.equal(firstFreeName('New folder', []), 'New folder');
	assert.equal(firstFreeName('New folder', ['new FOLDER', 'New folder 2']), 'New folder 3');
	assert.equal(firstFreeName('Åsa', ['ÅSA']), 'Åsa 2');
});

test('sameIds compares id sets regardless of order and duplicates', () => {
	assert.ok(sameIds([1, 2], new Set([2, 1])));
	assert.ok(sameIds([], []));
	assert.ok(sameIds([1, 1, 2], [2, 1]));
	assert.ok(!sameIds([1], [1, 2]));
	assert.ok(!sameIds([1, 3], [1, 2]));
});

const { treeKeyAction, noteNeighbour, dropFolder, autoScrollStep } = await import('../src/lib/folders.ts');

test('treeKeyAction: ↑/↓/Home/End move between rows and skip "Empty" placeholders', () => {
	// f:4 e:4 f:3 f:1 f:2 f:5 e:5 n:30 n:20 n:10 n:11 n:40
	const rows = visibleRows(buildTree(folders, notes), new Set([1, 2, 4, 5]));
	assert.deepEqual(treeKeyAction(rows, 0, 'ArrowDown'), { kind: 'focus', index: 2 }, 'skips e:4');
	assert.deepEqual(treeKeyAction(rows, 2, 'ArrowUp'), { kind: 'focus', index: 0 });
	assert.deepEqual(treeKeyAction(rows, 0, 'ArrowUp'), { kind: 'exit' }, '↑ on the first row → out of the tree (root row)');
	assert.equal(treeKeyAction(rows, rows.length - 1, 'ArrowDown'), null, '↓ on the last row stays');
	assert.deepEqual(treeKeyAction(rows, 5, 'Home'), { kind: 'focus', index: 0 });
	assert.deepEqual(treeKeyAction(rows, 5, 'End'), { kind: 'focus', index: rows.length - 1 });
	assert.equal(treeKeyAction(rows, 0, 'Home'), null);
	assert.equal(treeKeyAction(rows, 99, 'ArrowDown'), null);
	assert.equal(treeKeyAction(rows, 0, 'x'), null);
});

test('treeKeyAction: → expands or steps in, ← collapses or steps out', () => {
	const rows = visibleRows(buildTree(folders, notes), new Set([1, 2, 4, 5]));
	const at = (key) => rows.findIndex((r) => r.key === key);
	assert.deepEqual(treeKeyAction(rows, at('f:3'), 'ArrowRight'), { kind: 'expand', folderId: 3 }, 'collapsed → expand');
	assert.deepEqual(treeKeyAction(rows, at('f:1'), 'ArrowRight'), { kind: 'focus', index: at('f:2') }, 'expanded → first child');
	assert.equal(treeKeyAction(rows, at('f:4'), 'ArrowRight'), null, 'expanded but empty');
	assert.equal(treeKeyAction(rows, at('n:30'), 'ArrowRight'), null);
	assert.deepEqual(treeKeyAction(rows, at('f:2'), 'ArrowLeft'), { kind: 'collapse', folderId: 2 });
	assert.deepEqual(treeKeyAction(rows, at('n:30'), 'ArrowLeft'), { kind: 'focus', index: at('f:2') }, 'note → its folder');
	assert.deepEqual(treeKeyAction(rows, at('f:3'), 'ArrowLeft'), null, 'collapsed root folder');
	assert.equal(treeKeyAction(rows, at('n:10'), 'ArrowLeft'), null, 'root note');
});

test('treeKeyAction: Enter/Space activate, F2 renames folders only', () => {
	const rows = visibleRows(buildTree(folders, notes), new Set([4]));
	assert.deepEqual(treeKeyAction(rows, 0, 'Enter'), { kind: 'activate' });
	assert.deepEqual(treeKeyAction(rows, 0, ' '), { kind: 'activate' });
	assert.deepEqual(treeKeyAction(rows, 0, 'F2'), { kind: 'rename', folderId: 4 });
	assert.equal(treeKeyAction(rows, rows.findIndex((r) => r.kind === 'note'), 'F2'), null);
	assert.equal(treeKeyAction(rows, 1, 'Enter'), null, 'the Empty row');
});

test('noteNeighbour picks the next note in the same folder, else the previous, regardless of collapse', () => {
	const tree = buildTree(folders, [note(30, 2), note(31, 2), note(32, 2), note(20, 1), note(10), note(11)]);
	assert.equal(noteNeighbour(tree, 30), 31, 'next in Archive');
	assert.equal(noteNeighbour(tree, 31), 32);
	assert.equal(noteNeighbour(tree, 32), 31, 'last in folder → previous');
	assert.equal(noteNeighbour(tree, 20), null, 'only note in Projects: never jumps to another folder');
	assert.equal(noteNeighbour(tree, 10), 11, 'root');
	assert.equal(noteNeighbour(tree, 11), 10);
	assert.equal(noteNeighbour(tree, 77), null, 'unknown note');
	assert.equal(noteNeighbour(buildTree([], [note(1)]), 1), null, 'last note');
	const stale = buildTree(folders, [note(1, 99), note(2)]);
	assert.equal(noteNeighbour(stale, 1), 2, 'a note in an unknown folder counts as root');
});

test('dropFolder: valid targets only; self, descendants and the current place do nothing', () => {
	const tree = buildTree(folders, []);
	const noteIn = (folderId) => ({ kind: 'note', id: 1, folderId });
	const projects = { kind: 'folder', id: 1, parentId: null };
	assert.equal(dropFolder(tree, noteIn(null), 2), 2);
	assert.equal(dropFolder(tree, noteIn(2), null), null, 'to root');
	assert.equal(dropFolder(tree, noteIn(2), 2), undefined, 'same folder');
	assert.equal(dropFolder(tree, noteIn(null), null), undefined, 'already at root');
	assert.equal(dropFolder(tree, noteIn(null), 99), undefined, 'unknown folder');
	assert.equal(dropFolder(tree, noteIn(null), undefined), undefined, 'no target');
	assert.equal(dropFolder(tree, projects, 3), 3);
	assert.equal(dropFolder(tree, projects, 1), undefined, 'itself');
	assert.equal(dropFolder(tree, projects, 5), undefined, 'a descendant');
	assert.equal(dropFolder(tree, projects, null), undefined, 'already at root');
	assert.equal(dropFolder(tree, { kind: 'folder', id: 5, parentId: 2 }, null), null);
	assert.equal(dropFolder(tree, { kind: 'folder', id: 5, parentId: 2 }, 1), 1, 'to an ancestor');
});

test('autoScrollStep scrolls near the edges, faster closer to them', () => {
	assert.equal(autoScrollStep(300, 100, 500), 0);
	assert.ok(autoScrollStep(130, 100, 500) < 0);
	assert.ok(autoScrollStep(105, 100, 500) < autoScrollStep(130, 100, 500), 'faster nearer the top');
	assert.equal(autoScrollStep(50, 100, 500), -14, 'capped past the edge');
	assert.ok(autoScrollStep(480, 100, 500) > 0);
	assert.equal(autoScrollStep(900, 100, 500), 14);
});
