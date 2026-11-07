// Pure helpers for the notes folder tree: sorting, grouping, the flattened visible rows
// the list renders and navigates, folder paths, move targets and free-name suffixing.
import type { Folder, NoteSummary } from '#lib/types.js';

// ── Sorting and grouping ──────────────────────────────────────────────────────

const collator = new Intl.Collator(undefined, { sensitivity: 'accent', numeric: true });

/** A–Z in the user's locale, ignoring case ("Work 2" before "Work 10"), then id. */
export function compareFolders(a: Folder, b: Folder): number {
	return collator.compare(a.name, b.name) || a.id - b.id;
}

export function sortFolders(folders: Folder[]): Folder[] {
	return [...folders].sort(compareFolders);
}

/** Folders and notes grouped by parent (root = null). */
export interface FolderTree {
	byId: Map<number, Folder>;
	/** Child folders per parent, sorted with `compareFolders`. */
	folders: Map<number | null, Folder[]>;
	/** Notes per folder, in input order (pass `sortNotes` output). */
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
 * The open note's replacement after deleting it: the next note in the same folder (sorted
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

/**
 * The folder a drop moves `item` into (null: root), or undefined when the drop would do
 * nothing: no target, an unknown folder, where it already is, or (for a folder) itself
 * or one of its descendants. Only defined results are highlighted.
 */
export function dropFolder(tree: FolderTree, item: DragItem, target: number | null | undefined): number | null | undefined {
	if (target === undefined) return undefined;
	if (target !== null && !tree.byId.has(target)) return undefined;
	if (item.kind === 'note') return target === item.folderId ? undefined : target;
	if (target === item.parentId) return undefined;
	if (target !== null && descendantIds(tree, item.id).has(target)) return undefined;
	return target;
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
