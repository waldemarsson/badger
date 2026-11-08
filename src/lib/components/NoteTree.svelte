<script lang="ts">
	import { onDestroy, tick } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import {
		ROOT_TARGET,
		autoScrollStep,
		dropFolder,
		folderChain,
		noteNeighbour,
		targetFolderId,
		treeKeyAction,
		visibleRows,
		type DragItem,
		type FolderTree,
		type TreeRow
	} from '#lib/folders.js';
	import type { Folder, NoteSummary } from '#lib/types.js';
	import NameInput from '#lib/components/NameInput.svelte';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import FilePlus from '@lucide/svelte/icons/file-plus';
	import FileText from '@lucide/svelte/icons/file-text';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderInput from '@lucide/svelte/icons/folder-input';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import PencilLine from '@lucide/svelte/icons/pencil-line';
	import Pin from '@lucide/svelte/icons/pin';
	import PinOff from '@lucide/svelte/icons/pin-off';
	import Trash2 from '@lucide/svelte/icons/trash';

	// The notes folder tree: one flat keyed list of visible rows (so `animate:flip` works
	// across levels), ARIA tree keyboard navigation, inline folder rename/delete, a
	// right-click menu per row, and pointer-event drag and drop (todos-page pattern) with
	// auto-expand and auto-scroll.

	const DRAG_THRESHOLD = 5; // px of movement before a press becomes a drag
	const AUTO_EXPAND_MS = 600;
	const INDENT = 18; // px per level: chevron + gap, so children's chevrons sit under the folder icon
	const MAX_INDENT_DEPTH = 6; // deeper levels stop indenting so titles stay readable
	const ROOT_KEY = 'root'; // the "Notes" root row (tree rows are `f:<id>` / `n:<id>`)

	let {
		tree,
		expanded,
		selectedId,
		currentFolderId,
		renamingId = $bindable(null),
		confirmingNoteId = $bindable(null),
		scrollEl,
		onopen,
		ontoggle,
		onexpand,
		oncurrent,
		onpin,
		onnewfolder,
		onnewnote,
		onrename,
		ondelete,
		ondeletenote,
		onmoveto,
		onmove,
		onexit
	}: {
		tree: FolderTree;
		expanded: ReadonlySet<number>;
		selectedId: number | null;
		currentFolderId: number | null;
		/** Folder whose name is being edited. */
		renamingId?: number | null;
		/** Note whose inline delete confirm is showing. */
		confirmingNoteId?: number | null;
		/** The scrolling list element (auto-scrolled while dragging). */
		scrollEl: HTMLElement | undefined;
		onopen: (noteId: number) => void;
		/** Click / Enter on a folder: toggle it and make it the current folder. */
		ontoggle: (folderId: number) => void;
		/** Keyboard expand/collapse, and folders auto-expanded by a drag that ended in a drop. */
		onexpand: (folderIds: number[], open: boolean) => void;
		/** Click / Enter / Space on the "Notes" root row (null: root becomes current). */
		oncurrent: (folderId: number | null) => void;
		onpin: (note: NoteSummary) => void;
		/** null: at the top level. */
		onnewfolder: (parentId: number | null) => void;
		/** Context menu "New note here" / root "New note" (null): the folder becomes current. */
		onnewnote: (folderId: number | null) => void;
		/** Rejects with the user-facing message (shown under the name input). */
		onrename: (folderId: number, name: string) => Promise<void>;
		ondelete: (folderId: number) => Promise<void>;
		ondeletenote: (noteId: number) => Promise<void>;
		/** Opens the "Move to…" picker for a folder or note. */
		onmoveto: (item: DragItem, name: string) => void;
		onmove: (item: DragItem, folderId: number | null) => void;
		/** ↑ on the "Notes" root row (↑ on the first tree row focuses the root row). */
		onexit: () => void;
	} = $props();

	let rootEl = $state<HTMLElement>();
	let rootRowEl = $state<HTMLElement>();
	let focusKey = $state<string | null>(null); // last focused row (roving tabindex)
	let confirmingId = $state<number | null>(null);
	let flashKey = $state<string | null>(null);
	let flashTimer: ReturnType<typeof setTimeout> | undefined;

	// ── Rows ──────────────────────────────────────────────────────────────────────

	let dragOpened = $state<number[]>([]); // auto-expanded by the current drag; reverted on cancel
	const openSet = $derived(dragOpened.length ? new Set([...expanded, ...dragOpened]) : expanded);
	const rows = $derived(visibleRows(tree, openSet));

	/** The one row in the tab order: the last focused, else the open note, else the first. */
	const activeKey = $derived.by(() => {
		const keys = rows.filter((r) => r.kind !== 'empty').map((r) => r.key);
		if (focusKey && keys.includes(focusKey)) return focusKey;
		const open = selectedId === null ? null : `n:${selectedId}`;
		if (open && keys.includes(open)) return open;
		return keys[0] ?? null;
	});

	function indent(depth: number, note = false): string {
		return `padding-left: ${8 + Math.min(depth, MAX_INDENT_DEPTH) * INDENT + (note ? INDENT : 0)}px`;
	}

	function rowEl(key: string): HTMLElement | null {
		return rootEl?.querySelector<HTMLElement>(`[data-key="${key}"]`) ?? null;
	}

	function focusRow(key: string): void {
		const el = key === ROOT_KEY ? (rootRowEl ?? null) : rowEl(key);
		el?.focus();
		el?.scrollIntoView({ block: 'nearest' });
	}

	/** After an inline edit closes: back to the row, unless the user clicked somewhere else. */
	async function refocus(key: string): Promise<void> {
		await tick();
		const active = document.activeElement;
		if (!active || active === document.body || !active.isConnected) focusRow(key);
	}

	// A focused row that moves (Move to…, a drop) is re-inserted by the keyed each, which
	// drops DOM focus to the body: put it back on the row.
	let focusedBeforeUpdate: string | null = null;
	$effect.pre(() => {
		void rows;
		const el = document.activeElement;
		focusedBeforeUpdate = el instanceof HTMLElement && el.dataset.key && rootEl?.contains(el) ? el.dataset.key : null;
	});
	$effect(() => {
		void rows;
		const key = focusedBeforeUpdate;
		focusedBeforeUpdate = null;
		const active = document.activeElement;
		if (key && (!active || active === document.body)) focusRow(key);
	});

	// A folder just put into rename mode (created from the bar) scrolls into view.
	$effect(() => {
		const id = renamingId;
		if (id === null) return;
		void tick().then(() => rowEl(`f:${id}`)?.scrollIntoView({ block: 'nearest' }));
	});

	// ── Rows: one shared click/key handler for folders and notes ───────────────────

	function activate(row: TreeRow): void {
		if (row.kind === 'note') onopen(row.note.id);
		else if (row.kind === 'folder') ontoggle(row.folder.id);
	}

	/** The row's own controls and inline panels: they neither toggle/open nor start drags. */
	const ROW_CONTROLS = 'button, input, textarea, a, .tree-folder-confirm-bar, .tree-note-confirm-bar, .form-error';

	function onRowClick(e: MouseEvent, row: TreeRow): void {
		if ((e.target as Element).closest(ROW_CONTROLS)) return;
		activate(row);
	}

	function onRowKeydown(e: KeyboardEvent, index: number): void {
		if (e.target !== e.currentTarget || e.isComposing) return; // keys in the row's buttons/input
		const row = rows[index];
		if (isMenuKey(e)) {
			e.preventDefault();
			if (row) openMenuAt(row.key, e.currentTarget as HTMLElement);
			return;
		}
		if (e.metaKey || e.ctrlKey || e.altKey) return;
		if (e.key === 'Escape' && row?.kind === 'folder' && confirmingId === row.folder.id) {
			// Clicking the confirm bar's text focuses the row, so Esc lands here, not in the bar.
			e.preventDefault();
			e.stopPropagation();
			confirmingId = null;
			return;
		}
		if (e.key === 'Escape' && row?.kind === 'note' && confirmingNoteId === row.note.id) {
			e.preventDefault();
			e.stopPropagation();
			confirmingNoteId = null;
			return;
		}
		const action = treeKeyAction(rows, index, e.key);
		if (!action) return;
		e.preventDefault();
		switch (action.kind) {
			case 'focus':
				focusRow(rows[action.index].key);
				break;
			case 'exit':
				rootEl?.querySelector<HTMLElement>('.tree-root')?.focus(); // root's own ↑ goes on to search
				break;
			case 'expand':
			case 'collapse':
				onexpand([action.folderId], action.kind === 'expand');
				break;
			case 'activate':
				activate(rows[index]);
				break;
			case 'rename':
				confirmingId = null;
				renamingId = action.folderId;
				break;
		}
	}

	/** The "Notes" root row is its own Tab stop: ↓ into the tree, ↑ back to the search box. */
	function onRootKeydown(e: KeyboardEvent): void {
		if (isMenuKey(e)) {
			e.preventDefault();
			openMenuAt(ROOT_KEY, e.currentTarget as HTMLElement);
			return;
		}
		if (e.metaKey || e.ctrlKey || e.altKey) return;
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			const first = rows.find((r) => r.kind !== 'empty');
			if (first) focusRow(first.key);
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			onexit();
		}
	}

	// ── Folder actions ────────────────────────────────────────────────────────────

	function startRename(id: number): void {
		confirmingId = null;
		renamingId = id;
	}

	/** One inline delete confirm in the tree at a time. */
	function askDeleteFolder(id: number): void {
		renamingId = null;
		confirmingNoteId = null;
		confirmingId = id;
	}

	function askDeleteNote(id: number): void {
		confirmingId = null;
		confirmingNoteId = id;
	}

	async function saveRename(id: number, name: string): Promise<void> {
		await onrename(id, name); // a rejection keeps the input open with the error
		if (renamingId === id) renamingId = null;
		void refocus(`f:${id}`);
	}

	function cancelRename(id: number): void {
		if (renamingId === id) renamingId = null;
		void refocus(`f:${id}`);
	}

	async function confirmDelete(folder: Folder): Promise<void> {
		confirmingId = null;
		await ondelete(folder.id);
		void refocus(folder.parent_id === null ? (rows[0]?.key ?? '') : `f:${folder.parent_id}`);
	}

	function onConfirmKeydown(e: KeyboardEvent, id: number): void {
		if (e.key !== 'Escape') return;
		e.preventDefault();
		e.stopPropagation();
		confirmingId = null;
		void refocus(`f:${id}`);
	}

	// ── Note delete (from the context menu) ───────────────────────────────────────

	/** Focus then goes to the next/previous note in its folder, else the folder, else root. */
	async function confirmDeleteNote(id: number, parentId: number | null): Promise<void> {
		confirmingNoteId = null;
		const next = noteNeighbour(tree, id);
		const fallback = next !== null ? `n:${next}` : parentId !== null ? `f:${parentId}` : ROOT_KEY;
		await ondeletenote(id);
		await tick();
		const key = `n:${id}`;
		void refocus(rows.some((r) => r.key === key) ? key : fallback); // still there: it failed
	}

	function onNoteConfirmKeydown(e: KeyboardEvent, key: string): void {
		if (e.key !== 'Escape') return;
		e.preventDefault();
		e.stopPropagation();
		confirmingNoteId = null;
		void refocus(key);
	}

	// ── Context menu (right-click, ctrl+click, Shift+F10, the ContextMenu key) ─────

	type OpenMenu = { key: string; x: number; y: number; anchorHeight: number; label: string; items: MenuItem[]; seq: number };
	let menu = $state<OpenMenu | null>(null);
	let menuSeq = 0;
	let keyMenuAt = -Infinity; // when the keyboard last opened one (its own contextmenu event is ignored)
	const KEY_MENU_GRACE_MS = 500;

	/** Inline editors and confirm bars keep the WebView's own menu (copy, paste). */
	const NATIVE_MENU = 'input, textarea, .tree-folder-confirm-bar, .tree-note-confirm-bar';

	function isMenuKey(e: KeyboardEvent): boolean {
		return e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey);
	}

	/** The row's actions, built when the menu opens (null: no menu for this key). */
	function menuFor(key: string): { label: string; items: MenuItem[] } | null {
		if (key === ROOT_KEY) {
			return {
				label: 'Notes actions',
				items: [
					{ label: 'New note', icon: FilePlus, run: () => onnewnote(null) },
					{
						label: 'New folder',
						icon: FolderPlus,
						run: () => {
							oncurrent(null);
							onnewfolder(null);
						}
					}
				]
			};
		}
		const row = rows.find((r) => r.key === key);
		if (row?.kind === 'folder') {
			const f = row.folder;
			return {
				label: `Actions for ${f.name}`,
				items: [
					{ label: 'New note here', icon: FilePlus, run: () => onnewnote(f.id) },
					{ label: 'New subfolder', icon: FolderPlus, run: () => onnewfolder(f.id) },
					{ label: 'Rename', icon: PencilLine, hint: 'F2', run: () => startRename(f.id) },
					{ label: 'Move to…', icon: FolderInput, run: () => onmoveto({ kind: 'folder', id: f.id, parentId: row.parentId }, f.name) },
					{ label: 'Delete', icon: Trash2, danger: true, run: () => askDeleteFolder(f.id) }
				]
			};
		}
		if (row?.kind === 'note') {
			const n = row.note;
			return {
				label: `Actions for ${n.title}`,
				items: [
					{ label: n.pinned ? 'Unpin' : 'Pin', icon: n.pinned ? PinOff : Pin, run: () => onpin(n) },
					{ label: 'Move to…', icon: FolderInput, run: () => onmoveto({ kind: 'note', id: n.id, folderId: row.parentId }, n.title) },
					{ label: 'Delete', icon: Trash2, danger: true, run: () => askDeleteNote(n.id) }
				]
			};
		}
		return null;
	}

	function openMenu(key: string, x: number, y: number, anchorHeight = 0): void {
		if (pending || drag) return; // never mid-press or mid-drag
		const built = menuFor(key);
		if (!built) return;
		confirmingId = null;
		confirmingNoteId = null;
		menu = { key, x, y, anchorHeight, ...built, seq: ++menuSeq };
	}

	/** Keyboard opening: under the row, lined up with its content. */
	function openMenuAt(key: string, el: HTMLElement): void {
		const r = el.getBoundingClientRect();
		const pad = parseFloat(getComputedStyle(el).paddingLeft) || 0;
		keyMenuAt = performance.now();
		openMenu(key, r.left + pad, r.bottom, r.height);
	}

	function onContextMenu(e: MouseEvent, key: string): void {
		if ((e.target as Element).closest(NATIVE_MENU)) return;
		e.preventDefault();
		if (performance.now() - keyMenuAt < KEY_MENU_GRACE_MS) return; // Shift+F10 already opened it
		openMenu(key, e.clientX, e.clientY);
	}

	function closeMenu(refocus: boolean): void {
		const key = menu?.key;
		menu = null;
		if (!refocus || key === undefined) return;
		(key === ROOT_KEY ? rootRowEl : rowEl(key))?.focus({ preventScroll: true });
	}

	// The target row went away (a reload, a collapse elsewhere): close its menu.
	$effect(() => {
		const key = menu?.key;
		if (key === undefined || key === ROOT_KEY || rows.some((r) => r.key === key)) return;
		menu = null;
		if (activeKey !== null) void refocus(activeKey);
	});

	function autofocus(node: HTMLElement): void {
		node.focus();
	}

	// ── Drag and drop (pointer events: no interference from the WebView's file drop) ─

	type Press = { item: DragItem; label: string; startX: number; startY: number; width: number };
	let pending: Press | null = null;
	let drag = $state<(Press & { x: number; y: number; target: number | null | undefined }) | null>(null);
	let dragged = false; // the threshold was crossed: swallow the click that ends this press
	let autoExpandId: number | null = null;
	let autoExpandTimer: ReturnType<typeof setTimeout> | undefined;
	let scrollFrame = 0;

	function dragItem(row: TreeRow): DragItem | null {
		if (row.kind === 'note') return { kind: 'note', id: row.note.id, folderId: row.parentId };
		if (row.kind === 'folder') return { kind: 'folder', id: row.folder.id, parentId: row.parentId };
		return null;
	}

	function onRowPointerDown(e: PointerEvent, row: TreeRow): void {
		if (e.button !== 0 || e.ctrlKey || pending || drag) return; // ctrl+click opens the menu
		if ((e.target as Element).closest(ROW_CONTROLS)) return; // pin, actions, rename, confirm
		const item = dragItem(row);
		if (!item) return;
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const label = row.kind === 'note' ? row.note.title : row.kind === 'folder' ? row.folder.name : '';
		pending = { item, label, startX: e.clientX, startY: e.clientY, width: rect.width };
		dragged = false;
		window.addEventListener('pointermove', onPointerMove);
		window.addEventListener('pointerup', onPointerUp);
		window.addEventListener('pointercancel', onPointerCancel);
		window.addEventListener('keydown', onDragKeydown, true);
	}

	function onPointerMove(e: PointerEvent): void {
		if (e.buttons === 0) {
			// The release never reached us (e.g. it happened outside the window): cancel like Esc.
			onPointerCancel();
			return;
		}
		if (!drag && pending) {
			if (Math.hypot(e.clientX - pending.startX, e.clientY - pending.startY) < DRAG_THRESHOLD) return;
			drag = { ...pending, x: e.clientX, y: e.clientY, target: undefined };
			dragged = true;
			confirmingId = null;
			confirmingNoteId = null;
			document.body.classList.add('is-dragging');
			scrollFrame = requestAnimationFrame(autoScroll);
		}
		if (!drag) return;
		drag.x = e.clientX;
		drag.y = e.clientY;
		updateTarget();
	}

	/** Folder under the pointer (a note row stands for its folder; empty space for root). */
	function updateTarget(): void {
		if (!drag) return;
		const el = document.elementFromPoint(drag.x, drag.y)?.closest<HTMLElement>('[data-drop-folder]');
		const inside = !!el && !!rootEl?.contains(el);
		const target = inside ? dropFolder(tree, drag.item, targetFolderId(el!.dataset.dropFolder ?? '')) : undefined;
		if (target !== drag.target) drag.target = target;
		const collapsed = inside && el!.dataset.autoexpand ? Number(el!.dataset.autoexpand) : null;
		armAutoExpand(collapsed !== null && collapsed === target ? collapsed : null);
	}

	/** A collapsed (valid) folder hovered for 600 ms opens. */
	function armAutoExpand(id: number | null): void {
		if (id === autoExpandId) return;
		clearTimeout(autoExpandTimer);
		autoExpandId = id;
		if (id === null) return;
		autoExpandTimer = setTimeout(() => {
			autoExpandId = null;
			if (!drag || dragOpened.includes(id)) return;
			dragOpened = [...dragOpened, id];
			void tick().then(updateTarget);
		}, AUTO_EXPAND_MS);
	}

	/** Scrolls the list while the pointer is near (or past) its top or bottom edge. */
	function autoScroll(): void {
		if (!drag) return;
		if (scrollEl) {
			const r = scrollEl.getBoundingClientRect();
			const step = drag.x >= r.left && drag.x <= r.right ? autoScrollStep(drag.y, r.top, r.bottom) : 0;
			if (step) {
				const before = scrollEl.scrollTop;
				scrollEl.scrollTop += step;
				if (scrollEl.scrollTop !== before) updateTarget();
			}
		}
		scrollFrame = requestAnimationFrame(autoScroll);
	}

	/** Ends the drag visuals; a cancel also reverts the folders it auto-expanded. */
	function endDrag(): void {
		cancelAnimationFrame(scrollFrame);
		clearTimeout(autoExpandTimer);
		autoExpandId = null;
		document.body.classList.remove('is-dragging');
		pending = null;
		drag = null;
		dragOpened = [];
	}

	function stopListening(): void {
		window.removeEventListener('pointermove', onPointerMove);
		window.removeEventListener('pointerup', onPointerUp);
		window.removeEventListener('pointercancel', onPointerCancel);
		window.removeEventListener('keydown', onDragKeydown, true);
	}

	/** The click that follows a drag (or an Esc-cancelled drag) must not select or toggle. */
	function swallowNextClick(): void {
		const swallow = (e: MouseEvent) => {
			e.preventDefault();
			e.stopPropagation();
		};
		window.addEventListener('click', swallow, { capture: true, once: true });
		setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0);
	}

	function onPointerUp(): void {
		stopListening();
		const dropped = drag;
		const opened = dragOpened;
		endDrag();
		if (dragged) swallowNextClick();
		dragged = false;
		if (!dropped || dropped.target === undefined) return; // plain click, or released on no target
		if (opened.length) onexpand(opened, true); // a drop keeps the auto-expanded folders open
		flash(dropped.target === null ? 'root' : `f:${dropped.target}`);
		onmove(dropped.item, dropped.target);
	}

	function onPointerCancel(): void {
		stopListening();
		endDrag();
		dragged = false;
	}

	/** Esc cancels the drag; the release that follows still swallows its click. */
	function onDragKeydown(e: KeyboardEvent): void {
		if (e.key !== 'Escape' || !drag) return;
		e.preventDefault();
		e.stopPropagation();
		window.removeEventListener('pointermove', onPointerMove);
		endDrag();
	}

	function flash(key: string): void {
		clearTimeout(flashTimer);
		flashKey = key;
		flashTimer = setTimeout(() => (flashKey = null), 600);
	}

	/** Rows inside the highlighted target folder get a light tint (the drop zone). */
	function inDropZone(row: TreeRow): boolean {
		const target = drag?.target;
		if (target === undefined || target === null || row.parentId === null) return false;
		return folderChain(tree.byId, row.parentId).includes(target);
	}

	function isDragged(row: TreeRow): boolean {
		if (!drag) return false;
		if (row.kind === 'note') return drag.item.kind === 'note' && drag.item.id === row.note.id;
		if (row.kind === 'folder') return drag.item.kind === 'folder' && drag.item.id === row.folder.id;
		return false;
	}

	const targetName = $derived.by(() => {
		const target = drag?.target;
		if (target === undefined) return null;
		return target === null ? 'Notes' : (tree.byId.get(target)?.name ?? null);
	});

	onDestroy(() => {
		stopListening();
		endDrag();
		clearTimeout(flashTimer);
	});
</script>

<!-- Empty space below the rows is the root drop target. -->
<!-- Ctrl+click is the macOS secondary click: it only opens the menu, never row or button actions. -->
<div
	bind:this={rootEl}
	class="tree"
	data-drop-folder={ROOT_TARGET}
	onclickcapture={(e) => {
		if (e.ctrlKey) e.stopPropagation();
	}}
>
	<button
		bind:this={rootRowEl}
		type="button"
		class="tree-root"
		class:tree-root-current={currentFolderId === null}
		class:tree-row-drop={drag?.target === null}
		class:tree-row-flash={flashKey === 'root'}
		class:tree-row-menu={menu?.key === ROOT_KEY}
		data-drop-folder={ROOT_TARGET}
		aria-current={currentFolderId === null ? 'true' : undefined}
		title="Top level: new notes and folders go here"
		onclick={() => oncurrent(null)}
		onkeydown={onRootKeydown}
		oncontextmenu={(e) => onContextMenu(e, ROOT_KEY)}
	>
		Notes
	</button>

	<div role="tree" aria-label="Notes">
		{#each rows as row, index (row.key)}
			<div role="none" animate:flip={{ duration: 180 }} in:fade={{ duration: 150 }}>
				{#if row.kind === 'folder'}
					{@const f = row.folder}
					<div
						role="treeitem"
						tabindex={row.key === activeKey ? 0 : -1}
						aria-level={row.depth + 1}
						aria-expanded={row.expanded}
						aria-selected="false"
						data-row
						data-key={row.key}
						data-drop-folder={f.id}
						data-autoexpand={!row.expanded && row.hasChildren ? f.id : undefined}
						class="tree-row tree-folder group"
						class:tree-row-drop={drag?.target === f.id}
						class:tree-row-drop-zone={inDropZone(row)}
						class:tree-row-dragging={isDragged(row)}
						class:tree-row-flash={flashKey === row.key}
						class:tree-folder-confirm={confirmingId === f.id}
						class:tree-row-menu={menu?.key === row.key}
						style={indent(row.depth)}
						onclick={(e) => onRowClick(e, row)}
						onkeydown={(e) => onRowKeydown(e, index)}
						onpointerdown={(e) => onRowPointerDown(e, row)}
						oncontextmenu={(e) => onContextMenu(e, row.key)}
						onfocus={() => (focusKey = row.key)}
					>
						<ChevronRight size={14} class="tree-chevron {row.expanded ? 'rotate-90' : ''} {row.hasChildren ? '' : 'opacity-40'}" />
						<span class="tree-folder-icon" class:tree-folder-icon-current={f.id === currentFolderId}>
							{#if row.expanded}<FolderOpen size={15} />{:else}<FolderIcon size={15} />{/if}
						</span>
						{#if renamingId === f.id}
							<NameInput
								initial={f.name}
								label="Folder name"
								onsave={(name) => saveRename(f.id, name)}
								oncancel={() => cancelRename(f.id)}
							/>
						{:else}
							<span class="tree-folder-name" title={f.name}>{f.name}</span>
							{#if confirmingId === f.id}
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<div class="tree-folder-confirm-bar" onkeydown={(e) => onConfirmKeydown(e, f.id)} in:fade={{ duration: 150 }}>
									<span class="mr-auto">
										<span class="font-medium text-red-700 dark:text-red-300">Delete?</span>
										Contents move to {f.parent_id === null ? 'the top level' : `“${tree.byId.get(f.parent_id)?.name ?? 'the parent'}”`}.
									</span>
									<span class="flex shrink-0 gap-1.5">
										<button
											type="button"
											class="btn-danger-sm"
											aria-label="Confirm delete {f.name}"
											use:autofocus
											onclick={() => void confirmDelete(f)}>Delete</button
										>
										<button
											type="button"
											class="btn-secondary-sm"
											onclick={() => {
												confirmingId = null;
												void refocus(row.key);
											}}>Cancel</button
										>
									</span>
								</div>
							{:else}
								<div class="tree-row-actions">
									<button type="button" class="btn-icon-ghost p-1" title="New subfolder" aria-label="New folder in {f.name}" onclick={() => onnewfolder(f.id)}>
										<FolderPlus size={14} />
									</button>
									<button type="button" class="btn-icon-ghost p-1" title="Rename (F2)" aria-label="Rename {f.name}" onclick={() => startRename(f.id)}>
										<PencilLine size={14} />
									</button>
									<button type="button" class="btn-icon-ghost p-1" title="Move to…" aria-label="Move {f.name} to…" onclick={() => onmoveto({ kind: 'folder', id: f.id, parentId: f.parent_id }, f.name)}>
										<FolderInput size={14} />
									</button>
									<button
										type="button"
										class="btn-icon-ghost-danger p-1"
										title="Delete folder"
										aria-label="Delete {f.name}"
										onclick={() => askDeleteFolder(f.id)}
									>
										<Trash2 size={14} />
									</button>
								</div>
							{/if}
						{/if}
					</div>
				{:else if row.kind === 'note'}
					{@const n = row.note}
					<div
						role="treeitem"
						tabindex={row.key === activeKey ? 0 : -1}
						aria-level={row.depth + 1}
						aria-selected={n.id === selectedId}
						aria-current={n.id === selectedId ? 'true' : undefined}
						data-row
						data-key={row.key}
						data-drop-folder={row.parentId ?? ROOT_TARGET}
						class="tree-row tree-note group"
						class:note-row-active={n.id === selectedId}
						class:tree-row-drop-zone={inDropZone(row)}
						class:tree-row-dragging={isDragged(row)}
						class:tree-note-confirm={confirmingNoteId === n.id}
						class:tree-row-menu={menu?.key === row.key}
						style={indent(row.depth, true)}
						onclick={(e) => onRowClick(e, row)}
						onkeydown={(e) => onRowKeydown(e, index)}
						onpointerdown={(e) => onRowPointerDown(e, row)}
						oncontextmenu={(e) => onContextMenu(e, row.key)}
						onfocus={() => (focusKey = row.key)}
					>
						<p class="note-row-title pr-6">{n.title}</p>
						{#if n.excerpt}
							<p class="note-row-excerpt">{n.excerpt}</p>
						{/if}
						<button
							type="button"
							tabindex="-1"
							class="note-row-pin"
							class:note-row-pin-on={n.pinned}
							title={n.pinned ? 'Unpin' : 'Pin to top'}
							aria-label={n.pinned ? `Unpin ${n.title}` : `Pin ${n.title}`}
							onclick={() => onpin(n)}
						>
							{#if n.pinned}
								<!-- Pinned: a filled pin; hovering the row offers unpin. -->
								<Pin size={13} class="fill-current group-hover:hidden" />
								<PinOff size={13} class="hidden group-hover:block" />
							{:else}
								<Pin size={13} />
							{/if}
						</button>
						{#if confirmingNoteId === n.id}
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<div class="tree-note-confirm-bar" onkeydown={(e) => onNoteConfirmKeydown(e, row.key)} in:fade={{ duration: 150 }}>
								<span class="mr-auto font-medium text-red-700 dark:text-red-300">Delete note?</span>
								<span class="flex shrink-0 gap-1.5">
									<button
										type="button"
										class="btn-danger-sm"
										aria-label="Confirm delete {n.title}"
										use:autofocus
										onclick={() => void confirmDeleteNote(n.id, row.parentId)}>Delete</button
									>
									<button
										type="button"
										class="btn-secondary-sm"
										onclick={() => {
											confirmingNoteId = null;
											void refocus(row.key);
										}}>Cancel</button
									>
								</span>
							</div>
						{/if}
					</div>
				{:else}
					<p
						class="tree-empty"
						class:tree-row-drop-zone={drag?.target === row.parentId}
						style={indent(row.depth, true)}
						data-drop-folder={row.parentId}
						aria-hidden="true"
					>
						Empty
					</p>
				{/if}
			</div>
		{/each}
	</div>
</div>

{#if menu}
	{#key menu.seq}
		<ContextMenu x={menu.x} y={menu.y} anchorHeight={menu.anchorHeight} label={menu.label} items={menu.items} onclose={closeMenu} />
	{/key}
{/if}

{#if drag}
	<!-- Floating label that follows the pointer; never a hit-test target. -->
	<div class="pointer-events-none fixed left-0 top-0 z-50" style="transform: translate({drag.x + 14}px, {drag.y + 10}px);">
		<div class="tree-drag-ghost" style="max-width: {Math.min(drag.width, 260)}px">
			<span class="flex min-w-0 items-center gap-1.5">
				{#if drag.item.kind === 'folder'}<FolderIcon size={14} class="shrink-0" />{:else}<FileText size={14} class="shrink-0" />{/if}
				<span class="truncate">{drag.label}</span>
			</span>
			{#if targetName !== null}
				<span class="tree-drag-ghost-target" in:fade={{ duration: 150 }}>Move to {targetName}</span>
			{/if}
		</div>
	</div>
{/if}
