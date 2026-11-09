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
	sameIds,
	moveFolderLocally
} = await import('../src/lib/folders.ts');

const folder = (id, name, parent_id = null, position = 0) => ({ id, parent_id, name, position });
const note = (id, folder_id = null, extra = {}) => ({
	id,
	folder_id,
	title: `Note ${id}`,
	excerpt: '',
	pinned: false,
	created_at: '2026-11-01T09:00:00.000',
	updated_at: '2026-11-01T09:00:00.000',
	position: 0,
	...extra
});

// Root in manual order: Beta (4, empty), inbox (3), Projects (1) > Archive (2) > Deep (5)
const folders = [folder(2, 'Archive', 1), folder(1, 'Projects', null, 2), folder(3, 'inbox', null, 1), folder(4, 'Beta'), folder(5, 'Deep', 2)];
// Notes come pre-sorted (sortNotes); the tree keeps their order per folder.
const notes = [note(30, 2), note(10), note(20, 1), note(11), note(40, 99)];
const keys = (rows) => rows.map((r) => r.key);

test('compareFolders keeps the manual order: position, then id, never the name', () => {
	const sorted = sortFolders([folder(1, 'b', null, 2), folder(2, 'A', null, 1), folder(3, 'Work 10', null, 0), folder(4, 'Work 2', null, 1)]);
	assert.deepEqual(
		sorted.map((f) => f.name),
		['Work 10', 'A', 'Work 2', 'b']
	);
	assert.ok(compareFolders(folder(2, 'a'), folder(1, 'z')) > 0, 'same position: lower id first');
	assert.deepEqual(folders.map((f) => f.id), [2, 1, 3, 4, 5], 'input not mutated');
});

test('moveFolderLocally mirrors move_folder: insert at index (clamped; null = end), renumber densely', () => {
	const flat = [folder(1, 'A', null, 0), folder(2, 'B', null, 1), folder(3, 'C', null, 2), folder(4, 'X', 1, 0)];
	const order = (list, parent) => sortFolders(list.filter((f) => f.parent_id === parent)).map((f) => `${f.name}${f.position}`);
	assert.deepEqual(order(moveFolderLocally(flat, 3, null, 0), null), ['C0', 'A1', 'B2']);
	assert.deepEqual(order(moveFolderLocally(flat, 1, null, 99), null), ['B0', 'C1', 'A2']);
	const across = moveFolderLocally(flat, 2, 1, 0);
	assert.deepEqual(order(across, 1), ['B0', 'X1']);
	assert.deepEqual(order(across, null), ['A0', 'C2'], 'the old siblings keep their gaps, like Rust');
	assert.deepEqual(order(moveFolderLocally(flat, 2, 1, null), 1), ['X0', 'B1'], 'null: the end');
	assert.equal(moveFolderLocally(flat, 99, null, 0), flat, 'unknown id: unchanged');
	assert.equal(flat[2].position, 2, 'input not mutated');
});

test('buildTree keeps the server order: renaming a folder does not move it', () => {
	const renamed = folders.map((f) => (f.id === 4 ? { ...f, name: 'Zzz' } : f));
	assert.deepEqual(buildTree(renamed, []).folders.get(null).map((f) => f.name), ['Zzz', 'inbox', 'Projects']);
});

test('buildTree groups folders and notes by parent; unknown parents fall back to root', () => {
	const tree = buildTree([...folders, folder(6, 'Lost', 77, 1)], notes);
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
	const pinned = [note(50, 2, { pinned: true }), ...notes];
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

const { treeKeyAction, noteNeighbour, dropTarget, keyboardMove, autoScrollStep } = await import('../src/lib/folders.ts');

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

test('autoScrollStep scrolls near the edges, faster closer to them', () => {
	assert.equal(autoScrollStep(300, 100, 500), 0);
	assert.ok(autoScrollStep(130, 100, 500) < 0);
	assert.ok(autoScrollStep(105, 100, 500) < autoScrollStep(130, 100, 500), 'faster nearer the top');
	assert.equal(autoScrollStep(50, 100, 500), -14, 'capped past the edge');
	assert.ok(autoScrollStep(480, 100, 500) > 0);
	assert.equal(autoScrollStep(900, 100, 500), 14);
});

// ── Drop targets and ⌥↑/⌥↓ ────────────────────────────────────────────────────

// Root: Beta (4, empty), inbox (3), Projects (1) > Archive (2); root notes: pinned 60 61 | 10 11 12
const dndNotes = [
	note(60, null, { pinned: true }),
	note(61, null, { pinned: true }),
	note(30, 2),
	note(10),
	note(11),
	note(12)
];
const dndTree = buildTree(folders.slice(0, 4), dndNotes);
// f:4 e:4 f:3 f:1 f:2 n:30 n:60 n:61 n:10 n:11 n:12
const dndRows = visibleRows(dndTree, new Set([1, 2, 4]));
const at = (key, fraction = 0.5) => ({ index: dndRows.findIndex((r) => r.key === key), fraction });
const drop = (item, hover) => dropTarget(dndTree, dndRows, item, hover);
const rootNote = (id) => ({ kind: 'note', id, folderId: null });
const folderItem = (id, parentId = null) => ({ kind: 'folder', id, parentId });

test('dropTarget: a note between note rows inserts there, with a line', () => {
	assert.deepEqual(drop(rootNote(12), at('n:10', 0.2)), {
		folderId: null,
		index: 0,
		line: { key: 'n:10', edge: 'before', depth: 0 }
	});
	assert.deepEqual(drop(rootNote(10), at('n:11', 0.8)), {
		folderId: null,
		index: 1,
		line: { key: 'n:12', edge: 'before', depth: 0 }
	});
	assert.deepEqual(drop(rootNote(10), at('n:12', 0.8)), {
		folderId: null,
		index: 2,
		line: { key: 'n:12', edge: 'after', depth: 0 }
	});
	// Across folders: into Archive before/after its note.
	assert.deepEqual(drop(rootNote(11), at('n:30', 0.9)), {
		folderId: 2,
		index: 1,
		line: { key: 'n:30', edge: 'after', depth: 2 }
	});
});

test('dropTarget: note slots snap into the dragged note\'s pin group', () => {
	// An unpinned note over the pinned ones: the top of the unpinned group.
	assert.deepEqual(drop(rootNote(12), at('n:60', 0.1)), {
		folderId: null,
		index: 0,
		line: { key: 'n:10', edge: 'before', depth: 0 }
	});
	// A pinned note over the unpinned ones: the bottom of the pinned group.
	assert.deepEqual(drop(rootNote(60), at('n:11', 0.9)), {
		folderId: null,
		index: 1,
		line: { key: 'n:10', edge: 'before', depth: 0 }
	});
	// Into a folder whose group is empty: index 0, the line at the edge of its other notes.
	assert.deepEqual(drop(rootNote(60), at('n:30', 0.9)), {
		folderId: 2,
		index: 0,
		line: { key: 'n:30', edge: 'before', depth: 2 }
	});
});

test('dropTarget: dropping where a note already is does nothing', () => {
	assert.equal(drop(rootNote(11), at('n:11', 0.2)), undefined, 'itself');
	assert.equal(drop(rootNote(11), at('n:10', 0.8)), undefined, 'just above it');
	assert.equal(drop(rootNote(11), at('n:12', 0.2)), undefined, 'just below it');
	assert.equal(drop(rootNote(10), at('n:61', 0.5)), undefined, 'snapped back to where it is');
	assert.equal(drop(rootNote(10), 'root'), undefined, 'root: already on top of its group');
	assert.equal(drop(rootNote(10), null), undefined);
});

test('dropTarget: a note over a folder row, its Empty row or the root moves into it (top of group)', () => {
	assert.deepEqual(drop(rootNote(10), at('f:3', 0.1)), { folderId: 3, index: null, line: null }, 'any height');
	assert.deepEqual(drop(rootNote(10), at('e:4')), { folderId: 4, index: null, line: null });
	assert.deepEqual(drop(rootNote(11), 'root'), { folderId: null, index: null, line: null }, 'own folder: to the top');
	assert.deepEqual(drop({ kind: 'note', id: 30, folderId: 2 }, 'root'), { folderId: null, index: null, line: null });
	assert.equal(drop({ kind: 'note', id: 30, folderId: 2 }, at('f:2')), undefined, 'own folder, already on top');
});

test('dropTarget: a folder row\'s top/bottom quarter inserts among siblings, the middle moves into it', () => {
	assert.deepEqual(drop(folderItem(1), at('f:4', 0.1)), {
		folderId: null,
		index: 0,
		line: { key: 'f:4', edge: 'before', depth: 0 }
	});
	assert.deepEqual(drop(folderItem(4), at('f:3', 0.9)), {
		folderId: null,
		index: 1,
		line: { key: 'f:3', edge: 'after', depth: 0 }
	});
	assert.deepEqual(drop(folderItem(4), at('f:3', 0.5)), { folderId: 3, index: null, line: null });
	// The bottom of an expanded folder is the start of its children.
	assert.deepEqual(drop(folderItem(3), at('f:1', 0.9)), {
		folderId: 1,
		index: 0,
		line: { key: 'f:1', edge: 'after', depth: 1 }
	});
	// Over notes: the end of their folder's subfolders.
	assert.deepEqual(drop(folderItem(3), at('n:30')), { folderId: 2, index: null, line: null });
	assert.deepEqual(
		drop(folderItem(4), at('n:10')),
		{ folderId: null, index: 2, line: { key: 'n:60', edge: 'before', depth: 0 } },
		'own parent: a reorder to the end, the line below the last subfolder (above the first note)'
	);
	assert.deepEqual(
		drop(folderItem(3), at('n:12', 0.9)),
		{ folderId: null, index: 2, line: { key: 'n:60', edge: 'before', depth: 0 } },
		'any note row of the parent'
	);
	assert.equal(drop(folderItem(1), at('n:10')), undefined, 'already last');
});

test('dropTarget: a folder never goes into itself or a descendant, and no-op drops are undefined', () => {
	assert.equal(drop(folderItem(1), at('f:1', 0.5)), undefined, 'itself');
	assert.equal(drop(folderItem(1), at('f:2', 0.5)), undefined, 'a descendant');
	assert.equal(drop(folderItem(1), at('f:2', 0.1)), undefined, 'a slot inside itself');
	assert.equal(drop(folderItem(1), at('f:1', 0.9)), undefined, 'its own children');
	assert.equal(drop(folderItem(1), at('f:3', 0.9)), undefined, 'just below its (collapsed) previous sibling');
	assert.equal(drop(folderItem(3), at('f:1', 0.1)), undefined, 'just above its next sibling');
	assert.equal(drop(folderItem(4), 'root'), undefined, 'its own parent');
	assert.equal(drop(folderItem(4), at('e:4')), undefined, 'its own Empty row');
	assert.deepEqual(drop(folderItem(2, 1), 'root'), { folderId: null, index: null, line: null });
});

test('dropTarget: empty space is the root for notes, the end of the root folders for folders', () => {
	assert.deepEqual(drop(rootNote(11), 'end'), { folderId: null, index: null, line: null }, 'note: today\'s root drop');
	assert.equal(drop(rootNote(10), 'end'), undefined, 'note already on top of its group');
	// Root has notes: the line closes the folder block, above the first root note.
	assert.deepEqual(drop(folderItem(4), 'end'), { folderId: null, index: 2, line: { key: 'n:60', edge: 'before', depth: 0 } });
	assert.deepEqual(drop(folderItem(2, 1), 'end'), { folderId: null, index: 3, line: { key: 'n:60', edge: 'before', depth: 0 } });
	assert.equal(drop(folderItem(1), 'end'), undefined, 'already the last root folder');
	assert.equal(drop(folderItem(1), 'root'), undefined, 'the Notes row stays "into root" (a no-op here)');

	// No root notes, last root folder expanded: the line goes after the last row of its subtree.
	const noNotes = buildTree(folders.slice(0, 4), [note(30, 2)]);
	const rows = visibleRows(noNotes, new Set([1, 2])); // f:4 f:3 f:1 f:2 n:30
	assert.deepEqual(dropTarget(noNotes, rows, folderItem(4), 'end'), {
		folderId: null,
		index: 2,
		line: { key: 'n:30', edge: 'after', depth: 0 }
	});
	const lastEmpty = visibleRows(buildTree(folders.slice(0, 4), []), new Set([1, 2])); // ends with e:2
	assert.deepEqual(dropTarget(buildTree(folders.slice(0, 4), []), lastEmpty, folderItem(3), 'end'), {
		folderId: null,
		index: 2,
		line: { key: 'e:2', edge: 'after', depth: 0 }
	});
});

test('keyboardMove: one step among siblings, within the pin group; null at the edges', () => {
	assert.deepEqual(keyboardMove(dndTree, rootNote(11), -1), { folderId: null, index: 0 });
	assert.deepEqual(keyboardMove(dndTree, rootNote(11), 1), { folderId: null, index: 2 });
	assert.equal(keyboardMove(dndTree, rootNote(10), -1), null, 'top of the unpinned group');
	assert.equal(keyboardMove(dndTree, rootNote(61), 1), null, 'bottom of the pinned group');
	assert.deepEqual(keyboardMove(dndTree, rootNote(61), -1), { folderId: null, index: 0 });
	assert.equal(keyboardMove(dndTree, rootNote(99), 1), null, 'unknown note');
	assert.deepEqual(keyboardMove(dndTree, folderItem(3), -1), { folderId: null, index: 0 });
	assert.deepEqual(keyboardMove(dndTree, folderItem(3), 1), { folderId: null, index: 2 });
	assert.equal(keyboardMove(dndTree, folderItem(4), -1), null);
	assert.equal(keyboardMove(dndTree, folderItem(1), 1), null);
	assert.equal(keyboardMove(dndTree, folderItem(2, 1), 1), null, 'only child');
});
