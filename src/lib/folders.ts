// Pure helpers for the notes folder tree: manual order, grouping, the flattened visible
// rows the list renders and navigates, folder paths, move targets, free-name suffixing,
// and where a drag or ⌥↑/⌥↓ puts an item.
import type { Folder, NoteSummary } from '#lib/types.js';

// ── Sorting and grouping ──────────────────────────────────────────────────────

/** Manual order (same as list_folders): position, then id. Names never re-sort. */
export function compareFolders(a: Folder, b: Folder): number {
	return a.position - b.position || a.id - b.id;
}

export function sortFolders(folders: Folder[]): Folder[] {
	return [...folders].sort(compareFolders);
}

/**
 * Mirrors move_folder: puts folder `id` under `parentId` at `index` among its siblings
 * there (clamped; null: the end) and renumbers them. Rust may also suffix the name on a
 * clash; the reload that follows picks that up.
 */
export function moveFolderLocally(folders: Folder[], id: number, parentId: number | null, index: number | null): Folder[] {
	const folder = folders.find((f) => f.id === id);
	if (!folder) return folders;
	const siblings = sortFolders(folders.filter((f) => f.parent_id === parentId && f.id !== id));
	siblings.splice(Math.max(0, Math.min(index ?? siblings.length, siblings.length)), 0, { ...folder, parent_id: parentId });
	const placed = new Map(siblings.map((f, i) => [f.id, { ...f, position: i }]));
	return folders.map((f) => placed.get(f.id) ?? f);
}

/** Folders and notes grouped by parent (root = null). */
export interface FolderTree {
	byId: Map<number, Folder>;
	/** Child folders per parent, in manual order (`compareFolders`). */
	folders: Map<number | null, Folder[]>;
	/** Notes per folder, in input order (pass `sortNotes` output: pinned first). */
	notes: Map<number | null, NoteSummary[]>;
}

/**
 * Groups folders and notes by parent. Unknown parents/folders fall back to root, and so do
 * folders in a parent cycle (legacy data), so they and their notes still show; `byId`
 * holds those with `parent_id: null`, so paths and drag targets agree with the tree.
 */
export function buildTree(folders: Folder[], notes: NoteSummary[]): FolderTree {
	const cyclic = cycleMembers(new Map(folders.map((f) => [f.id, f])));
	const fixed = folders.map((f) => (cyclic.has(f.id) ? { ...f, parent_id: null } : f));
	const byId = new Map(fixed.map((f) => [f.id, f]));
	const known = (id: number | null) => (id !== null && byId.has(id) ? id : null);
	const childFolders = new Map<number | null, Folder[]>();
	for (const f of sortFolders(fixed)) push(childFolders, known(f.parent_id), f);
	const childNotes = new Map<number | null, NoteSummary[]>();
	for (const n of notes) push(childNotes, known(n.folder_id), n);
	return { byId, folders: childFolders, notes: childNotes };
}

/** Ids of folders that are their own ancestor (on a ParentId cycle, self-parent included). */
function cycleMembers(byId: ReadonlyMap<number, Folder>): Set<number> {
	const members = new Set<number>();
	const done = new Set<number>();
	for (const start of byId.keys()) {
		const path: number[] = [];
		const onPath = new Set<number>();
		let id: number | null = start;
		while (id !== null && byId.has(id) && !done.has(id) && !onPath.has(id)) {
			path.push(id);
			onPath.add(id);
			id = byId.get(id)!.parent_id;
		}
		if (id !== null && onPath.has(id)) for (const m of path.slice(path.indexOf(id))) members.add(m);
		for (const m of path) done.add(m);
	}
	return members;
}

function push<K, V>(map: Map<K, V[]>, key: K, value: V): void {
	const list = map.get(key);
	if (list) list.push(value);
	else map.set(key, [value]);
}

// ── Visible rows ──────────────────────────────────────────────────────────────

interface RowBase {
	/** `f:<id>`, `n:<id>` or `e:<folder id>`: unique across kinds for keyed `{#each}`. */
	key: string;
	/** 0 at root (aria-level = depth + 1). */
	depth: number;
	/** Containing folder, null at root. */
	parentId: number | null;
}

export interface FolderRow extends RowBase {
	kind: 'folder';
	folder: Folder;
	expanded: boolean;
	/** Has child folders or notes. */
	hasChildren: boolean;
}

export interface NoteRow extends RowBase {
	kind: 'note';
	note: NoteSummary;
}

/** The dimmed "Empty" placeholder inside an expanded folder with no children. */
export interface EmptyRow extends RowBase {
	kind: 'empty';
	parentId: number;
}

export type TreeRow = FolderRow | NoteRow | EmptyRow;

/**
 * The rows to draw, top to bottom: per level folders first, then notes; children only
 * inside expanded folders (whose ancestors are expanded too). Each folder appears once
 * even if corrupt data has a parent cycle.
 */
export function visibleRows(tree: FolderTree, expanded: ReadonlySet<number>): TreeRow[] {
	const rows: TreeRow[] = [];
	const seen = new Set<number>();
	const walk = (parentId: number | null, depth: number) => {
		for (const folder of tree.folders.get(parentId) ?? []) {
			if (seen.has(folder.id)) continue;
			seen.add(folder.id);
			const hasChildren = tree.folders.has(folder.id) || tree.notes.has(folder.id);
			const open = expanded.has(folder.id);
			rows.push({ kind: 'folder', key: `f:${folder.id}`, depth, parentId, folder, expanded: open, hasChildren });
			if (!open) continue;
			if (hasChildren) walk(folder.id, depth + 1);
			else rows.push({ kind: 'empty', key: `e:${folder.id}`, depth: depth + 1, parentId: folder.id });
		}
		for (const note of tree.notes.get(parentId) ?? []) {
			rows.push({ kind: 'note', key: `n:${note.id}`, depth, parentId, note });
		}
	};
	walk(null, 0);
	return rows;
}

// ── Paths and subtrees ────────────────────────────────────────────────────────

/** `folderId` and its ancestors, innermost first; [] for root or an unknown folder. */
export function folderChain(byId: ReadonlyMap<number, Folder>, folderId: number | null): number[] {
	const chain: number[] = [];
	let current = folderId === null ? undefined : byId.get(folderId);
	while (current && !chain.includes(current.id)) {
		chain.push(current.id);
		current = current.parent_id === null ? undefined : byId.get(current.parent_id);
	}
	return chain;
}

/** "Projects / Archive"; '' for root or an unknown folder. */
export function folderPath(byId: ReadonlyMap<number, Folder>, folderId: number | null): string {
	return folderChain(byId, folderId)
		.reverse()
		.map((id) => byId.get(id)?.name ?? '')
		.join(' / ');
}

/** `folderId` plus every folder below it: the invalid targets when moving it. */
export function descendantIds(tree: FolderTree, folderId: number): Set<number> {
	const ids = new Set<number>();
	if (!tree.byId.has(folderId)) return ids;
	const stack = [folderId];
	while (stack.length) {
		const id = stack.pop()!;
		if (ids.has(id)) continue;
		ids.add(id);
		for (const child of tree.folders.get(id) ?? []) stack.push(child.id);
	}
	return ids;
}

// ── Move-to picker ────────────────────────────────────────────────────────────

/** Combobox value of the root target ("Notes (root)"). */
export const ROOT_TARGET = 'root';

export interface MoveTarget {
	value: number | typeof ROOT_TARGET;
	label: string;
}

/**
 * Combobox items for "Move to…": root first, then every folder by full path in tree
 * order. When moving a folder, pass its id to leave out itself and its descendants.
 */
export function moveTargets(tree: FolderTree, movingFolderId: number | null = null): MoveTarget[] {
	const excluded = movingFolderId === null ? new Set<number>() : descendantIds(tree, movingFolderId);
	const targets: MoveTarget[] = [{ value: ROOT_TARGET, label: 'Notes (root)' }];
	const seen = new Set<number>();
	const walk = (parentId: number | null, prefix: string) => {
		for (const folder of tree.folders.get(parentId) ?? []) {
			if (excluded.has(folder.id) || seen.has(folder.id)) continue;
			seen.add(folder.id);
			const label = prefix + folder.name;
			targets.push({ value: folder.id, label });
			walk(folder.id, `${label} / `);
		}
	};
	walk(null, '');
	return targets;
}

/** A picker value back to a folder id: null for root, undefined when not a target. */
export function targetFolderId(value: string): number | null | undefined {
	if (value === ROOT_TARGET) return null;
	return /^\d+$/.test(value) ? Number(value) : undefined;
}

// ── Names and sets ────────────────────────────────────────────────────────────

/** `base`, else the first free `base 2`, `base 3`, … (case-insensitive, like Rust). */
export function firstFreeName(base: string, taken: Iterable<string>): string {
	const lower = new Set([...taken].map((t) => t.toLowerCase()));
	if (!lower.has(base.toLowerCase())) return base;
	for (let n = 2; ; n++) {
		const candidate = `${base} ${n}`;
		if (!lower.has(candidate.toLowerCase())) return candidate;
	}
}

/** True when both hold the same ids (order and duplicates ignored). */
export function sameIds(a: Iterable<number>, b: Iterable<number>): boolean {
	const left = new Set(a);
	const right = new Set(b);
	return left.size === right.size && [...left].every((id) => right.has(id));
}

// ── Keyboard (ARIA tree) ──────────────────────────────────────────────────────

/** What a key on the row at `index` does. Empty placeholder rows are never focused. */
export type TreeKeyAction =
	| { kind: 'focus'; index: number }
	| { kind: 'expand'; folderId: number }
	| { kind: 'collapse'; folderId: number }
	| { kind: 'activate' }
	| { kind: 'rename'; folderId: number }
	/** ↑ on the first row: out of the tree (to the "Notes" root row). */
	| { kind: 'exit' };

/**
 * ↑/↓ move between visible rows, Home/End jump to the ends, → expands a folder or steps
 * into its first child, ← collapses or steps to the parent, Enter/Space open or toggle,
 * F2 renames a folder. Null: the key does nothing here.
 */
export function treeKeyAction(rows: readonly TreeRow[], index: number, key: string): TreeKeyAction | null {
	const row = rows[index];
	if (!row) return null;
	const focusable = (i: number) => rows[i] !== undefined && rows[i].kind !== 'empty';
	const scan = (from: number, step: 1 | -1) => {
		for (let i = from; i >= 0 && i < rows.length; i += step) if (focusable(i)) return i;
		return -1;
	};
	const focus = (i: number): TreeKeyAction | null => (i === -1 || i === index ? null : { kind: 'focus', index: i });
	switch (key) {
		case 'ArrowDown':
			return focus(scan(index + 1, 1));
		case 'ArrowUp': {
			const prev = scan(index - 1, -1);
			return prev === -1 ? { kind: 'exit' } : { kind: 'focus', index: prev };
		}
		case 'Home':
			return focus(scan(0, 1));
		case 'End':
			return focus(scan(rows.length - 1, -1));
		case 'ArrowRight':
			if (row.kind !== 'folder') return null;
			if (!row.expanded) return { kind: 'expand', folderId: row.folder.id };
			return row.hasChildren && focusable(index + 1) ? { kind: 'focus', index: index + 1 } : null;
		case 'ArrowLeft':
			if (row.kind === 'folder' && row.expanded) return { kind: 'collapse', folderId: row.folder.id };
			if (row.parentId === null) return null;
			return focus(rows.findIndex((r) => r.kind === 'folder' && r.folder.id === row.parentId));
		case 'Enter':
		case ' ':
			return row.kind === 'empty' ? null : { kind: 'activate' };
		case 'F2':
			return row.kind === 'folder' ? { kind: 'rename', folderId: row.folder.id } : null;
		default:
			return null;
	}
}

/**
 * The open note's replacement after deleting it: the next note in the same folder (list
 * order), else the previous one there, else none. Collapse state doesn't matter.
 */
export function noteNeighbour(tree: FolderTree, id: number): number | null {
	for (const list of tree.notes.values()) {
		const index = list.findIndex((n) => n.id === id);
		if (index !== -1) return (list[index + 1] ?? list[index - 1])?.id ?? null;
	}
	return null;
}

// ── Drag and drop ─────────────────────────────────────────────────────────────

/** What is being dragged, with where it lives now. */
export type DragItem =
	| { kind: 'note'; id: number; folderId: number | null }
	| { kind: 'folder'; id: number; parentId: number | null };

/** Where a move puts an item: into `folderId` at `index` among its group there (a note's
 * own pin group, or the subfolders); null: the default (a note at the top of its group, a
 * folder at the end). The `move_note` / `move_folder` arguments. */
export interface MovePlan {
	folderId: number | null;
	index: number | null;
}

/** A drop: the move, plus the insertion line to draw (null: highlight the folder instead). */
export interface DropTarget extends MovePlan {
	line: { key: string; edge: 'before' | 'after'; depth: number } | null;
}

/** What is under the pointer: a visible row with the pointer's relative height in it
 * (0 top … 1 bottom), the "Notes" root row, the empty space below the rows, or nothing. */
export type DropHover = { index: number; fraction: number } | 'root' | 'end' | null;

/** The dragged note's summary and its siblings (list order) in its folder. */
function noteContext(tree: FolderTree, item: Extract<DragItem, { kind: 'note' }>) {
	const list = tree.notes.get(item.folderId) ?? [];
	const note = list.find((n) => n.id === item.id);
	if (!note) return null;
	const group = list.filter((n) => n.pinned === note.pinned);
	return { note, index: group.indexOf(note) };
}

/** Moving `item` into `folderId` with the default placement; undefined when it's a no-op
 * or invalid (unknown folder; a folder into itself, a descendant, or its own parent). */
function intoTarget(tree: FolderTree, item: DragItem, folderId: number | null): DropTarget | undefined {
	if (folderId !== null && !tree.byId.has(folderId)) return undefined;
	if (item.kind === 'note') {
		// Into its own folder: the top of its group, unless it is already there.
		if (folderId === item.folderId && (noteContext(tree, item)?.index ?? 0) === 0) return undefined;
	} else {
		if (folderId === item.parentId) return undefined;
		if (folderId !== null && descendantIds(tree, item.id).has(folderId)) return undefined;
	}
	return { folderId, index: null, line: null };
}

/** A note dropped at the `edge` of note row `row`: snapped into the dragged note's own pin
 * group in that folder. */
function noteSlot(tree: FolderTree, item: Extract<DragItem, { kind: 'note' }>, row: NoteRow, edge: 'before' | 'after'): DropTarget | undefined {
	const ctx = noteContext(tree, item);
	if (!ctx) return undefined;
	const list = tree.notes.get(row.parentId) ?? [];
	const gap = list.indexOf(row.note) + (edge === 'after' ? 1 : 0);
	const others = list.filter((n) => n.id !== item.id);
	if (!others.length) return undefined;
	const pinnedCount = others.filter((n) => n.pinned).length;
	const [lo, hi] = ctx.note.pinned ? [0, pinnedCount] : [pinnedCount, others.length];
	const at = Math.max(lo, Math.min(hi, list.slice(0, gap).filter((n) => n.id !== item.id).length));
	const index = at - lo;
	if (row.parentId === item.folderId && index === ctx.index) return undefined;
	const line =
		at < others.length
			? { key: `n:${others[at].id}`, edge: 'before' as const, depth: row.depth }
			: { key: `n:${others[others.length - 1].id}`, edge: 'after' as const, depth: row.depth };
	return { folderId: row.parentId, index, line };
}

/** A folder dropped into `parentId` at sibling gap `gap` (counted with itself). */
function folderSlot(
	tree: FolderTree,
	item: Extract<DragItem, { kind: 'folder' }>,
	parentId: number | null,
	gap: number,
	line: DropTarget['line']
): DropTarget | undefined {
	if (parentId !== null && (!tree.byId.has(parentId) || descendantIds(tree, item.id).has(parentId))) return undefined;
	const siblings = tree.folders.get(parentId) ?? [];
	const index = siblings.slice(0, gap).filter((f) => f.id !== item.id).length;
	if (parentId === item.parentId && index === siblings.findIndex((f) => f.id === item.id)) return undefined;
	return { folderId: parentId, index, line };
}

/**
 * Where dropping `item` at `hover` puts it, or undefined when that would do nothing.
 * Notes: a note row's upper/lower half inserts before/after it, snapped into the dragged
 * note's pin group; a folder row (or its "Empty" placeholder, or the root) moves it into
 * that folder. Folders: a folder row's top/bottom quarter inserts before/after it (the
 * bottom of an expanded folder is the start of its children), the middle moves it into
 * it; over notes it goes to the end of their folder's subfolders (into another folder, or
 * a reorder with a line when that folder is its own parent). Empty space below the rows
 * is the root for notes, and the end of the root folders for folders.
 */
export function dropTarget(tree: FolderTree, rows: readonly TreeRow[], item: DragItem, hover: DropHover): DropTarget | undefined {
	if (hover === null) return undefined;
	if (hover === 'root') return intoTarget(tree, item, null);
	if (hover === 'end') {
		if (item.kind === 'note') return intoTarget(tree, item, null);
		// The line closes the root folders' block: above the first root note, else after
		// the last row (the last root folder's subtree when it is expanded).
		const first = tree.notes.get(null)?.[0];
		const last = rows.at(-1);
		const line: DropTarget['line'] = first
			? { key: `n:${first.id}`, edge: 'before', depth: 0 }
			: last
				? { key: last.key, edge: 'after', depth: 0 }
				: null;
		return folderSlot(tree, item, null, (tree.folders.get(null) ?? []).length, line);
	}
	const row = rows[hover.index];
	if (!row) return undefined;
	if (row.kind === 'empty') return intoTarget(tree, item, row.parentId);
	if (item.kind === 'note') {
		if (row.kind === 'folder') return intoTarget(tree, item, row.folder.id);
		return noteSlot(tree, item, row, hover.fraction < 0.5 ? 'before' : 'after');
	}
	if (row.kind === 'note') {
		if (row.parentId !== item.parentId) return intoTarget(tree, item, row.parentId);
		// Its own parent: a reorder to the end of the subfolders. The line goes above the
		// folder's first note, i.e. below the last subfolder and everything shown under it.
		const siblings = tree.folders.get(row.parentId) ?? [];
		const first = tree.notes.get(row.parentId)?.[0] ?? row.note;
		return folderSlot(tree, item, row.parentId, siblings.length, { key: `n:${first.id}`, edge: 'before', depth: row.depth });
	}
	const siblings = tree.folders.get(row.parentId) ?? [];
	const at = siblings.findIndex((f) => f.id === row.folder.id);
	if (hover.fraction < 0.25) return folderSlot(tree, item, row.parentId, at, { key: row.key, edge: 'before', depth: row.depth });
	if (hover.fraction >= 0.75) {
		if (row.expanded) return folderSlot(tree, item, row.folder.id, 0, { key: row.key, edge: 'after', depth: row.depth + 1 });
		return folderSlot(tree, item, row.parentId, at + 1, { key: row.key, edge: 'after', depth: row.depth });
	}
	return intoTarget(tree, item, row.folder.id);
}

/** ⌥↑ (-1) / ⌥↓ (+1): one step among the item's siblings (a note within its pin group);
 * null at the group's edge. */
export function keyboardMove(tree: FolderTree, item: DragItem, step: -1 | 1): MovePlan | null {
	if (item.kind === 'note') {
		const ctx = noteContext(tree, item);
		if (!ctx) return null;
		const size = (tree.notes.get(item.folderId) ?? []).filter((n) => n.pinned === ctx.note.pinned).length;
		const index = ctx.index + step;
		return index < 0 || index >= size ? null : { folderId: item.folderId, index };
	}
	const siblings = tree.folders.get(item.parentId) ?? [];
	const index = siblings.findIndex((f) => f.id === item.id) + step;
	if (index - step === -1 || index < 0 || index >= siblings.length) return null;
	return { folderId: item.parentId, index };
}

/**
 * Pixels to scroll the list this frame while dragging at `y`: negative near the top edge,
 * positive near the bottom, faster the closer to (or past) the edge; 0 in between.
 */
export function autoScrollStep(y: number, top: number, bottom: number, edge = 40, max = 14): number {
	const speed = (depth: number) => Math.ceil(max * Math.min(1, depth / edge));
	if (y < top + edge) return -speed(top + edge - y);
	if (y > bottom - edge) return speed(y - (bottom - edge));
	return 0;
}
