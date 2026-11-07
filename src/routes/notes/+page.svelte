<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { onMount, tick, untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { formatNoteDate, neighborAfterRemoval, plainRows, searchRows, sortNotes, upsertNote } from '#lib/notes.js';
	import type { Segment } from '#lib/notes.js';
	import { NoteSaveQueue } from '#lib/note-save-queue.js';
	import { registerFlushHandler } from '#lib/flush-guard.js';
	import { splitSearchQuery, type LinkFollow } from '#lib/note-links.js';
	import { linkData } from '#lib/editor/links.js';
	import { buildTree, folderChain, folderPath, moveTargets, noteNeighbour, sameIds, targetFolderId, visibleRows, type DragItem } from '#lib/folders.js';
	import type { Backlink, Folder, LinkTargets, Note, NoteHit, NoteSummary, RenamedNote, RenderMode, TagSuggestion } from '#lib/types.js';
	import NoteEditor from '#lib/components/NoteEditor.svelte';
	import NoteReader from '#lib/components/NoteReader.svelte';
	import NoteTree from '#lib/components/NoteTree.svelte';
	import MoveToDialog from '#lib/components/MoveToDialog.svelte';
	import BookOpen from 'lucide-svelte/icons/book-open';
	import Check from 'lucide-svelte/icons/check';
	import ChevronRight from 'lucide-svelte/icons/chevron-right';
	import CircleAlert from 'lucide-svelte/icons/circle-alert';
	import Copy from 'lucide-svelte/icons/copy';
	import FolderIcon from 'lucide-svelte/icons/folder';
	import FolderInput from 'lucide-svelte/icons/folder-input';
	import FolderPlus from 'lucide-svelte/icons/folder-plus';
	import NotebookPen from 'lucide-svelte/icons/notebook-pen';
	import PencilLine from 'lucide-svelte/icons/pencil-line';
	import Pin from 'lucide-svelte/icons/pin';
	import PinOff from 'lucide-svelte/icons/pin-off';
	import Plus from 'lucide-svelte/icons/plus';
	import Search from 'lucide-svelte/icons/search';
	import Trash2 from 'lucide-svelte/icons/trash-2';
	import X from 'lucide-svelte/icons/x';

	const NOT_FOUND = 'Note not found';

	let notes = $state<NoteSummary[]>([]);
	let folders = $state<Folder[]>([]);
	let loaded = $state(false);
	let loadError = $state('');
	let loadSeq = 0;
	let query = $state('');

	let selectedId = $state<number | null>(null);
	let current = $state<Note | null>(null);
	let selectSeq = 0;

	let titleDraft = $state('');
	let titleError = $state('');
	let renaming: { id: number; title: string; done: Promise<boolean> } | null = null; // rename in flight
	let lastFailedTitle: string | null = null; // skip blur-retry of a title just rejected

	let confirmingDelete = $state(false); // the editor header's delete confirm (open note)
	let treeConfirmNoteId = $state<number | null>(null); // a tree row's delete confirm
	let actionError = $state('');
	let navError = $state('');

	let editor = $state<ReturnType<typeof NoteEditor>>();
	let reader = $state<ReturnType<typeof NoteReader>>();
	let titleInput = $state<HTMLInputElement>();
	let searchInput = $state<HTMLInputElement>();
	let listEl = $state<HTMLElement>();
	let layout = $state<HTMLElement>();
	let paneHeight = $state(560);

	// ── Autosave ──────────────────────────────────────────────────────────────────

	let queueVersion = $state(0);
	const queue = new NoteSaveQueue<NoteSummary>({
		save: (id, body) => invoke<NoteSummary>('update_note_body', { id, body }),
		onSaved: (_id, summary) => applySummary(summary),
		onChange: () => queueVersion++
	});

	const saveStatus = $derived.by(() => {
		void queueVersion;
		return selectedId === null ? 'saved' : queue.status(selectedId);
	});
	const failedSaves = $derived.by(() => {
		void queueVersion;
		return queue.failed();
	});

	function onBodyChange(id: number, body: string): void {
		navError = '';
		queue.edit(id, body);
	}

	function retrySaves(): void {
		navError = '';
		void queue.flush();
	}

	function titleOf(id: number): string {
		return notes.find((n) => n.id === id)?.title ?? 'a note';
	}

	// ── Read view (⌘E) and copy as HTML ───────────────────────────────────────────

	let reading = $state(false); // survives note switches, not page loads
	let rendered = $state<{ id: number; html: string } | null>(null);
	let renderError = $state('');
	let renderSeq = 0;
	let viewError = $state(''); // copy or open-link failures
	let copied = $state(false);
	let copiedTimer: ReturnType<typeof setTimeout> | undefined;

	/** Renders the open note's live text (unsaved edits included) for the read view. */
	async function renderCurrent(): Promise<void> {
		if (!current) return;
		const seq = ++renderSeq;
		const id = current.id;
		renderError = '';
		try {
			const html = await invoke<string>('render_markdown', {
				markdown: editor?.text() ?? '',
				mode: 'read' satisfies RenderMode
			});
			if (seq !== renderSeq) return; // switched notes or toggled meanwhile
			rendered = { id, html };
		} catch (e) {
			if (seq !== renderSeq) return;
			renderError = String(e);
		}
	}

	async function toggleReading(): Promise<void> {
		if (!current) return;
		if (titleInput && document.activeElement === titleInput) await commitTitle(true);
		reading = !reading;
		if (reading) {
			await renderCurrent();
			await tick();
			if (reading) reader?.focus();
		} else {
			renderSeq++;
			rendered = null;
			renderError = '';
			await tick();
			editor?.focus();
		}
	}

	async function copyHtml(): Promise<void> {
		if (!current) return;
		viewError = '';
		const markdown = editor?.text() ?? '';
		const html = invoke<string>('render_markdown', { markdown, mode: 'copy' satisfies RenderMode });
		html.catch(() => {}); // a failure surfaces through clipboard.write below
		try {
			// Promise-valued items keep WebKit's user-gesture check satisfied across the invoke.
			await navigator.clipboard.write([
				new ClipboardItem({
					'text/html': html.then((h) => new Blob([h], { type: 'text/html' })),
					'text/plain': new Blob([markdown], { type: 'text/plain' })
				})
			]);
		} catch {
			viewError = 'Copy failed.';
			return;
		}
		copied = true;
		clearTimeout(copiedTimer);
		copiedTimer = setTimeout(() => (copied = false), 2000);
	}

	// ── Links: tags, wikilinks, backlinks ─────────────────────────────────────────

	let linksSeq = 0;

	/** Refetches what wikilinks/chips resolve to and the `#` suggestions (redraws the editor). */
	async function refreshLinks(): Promise<void> {
		const seq = ++linksSeq;
		try {
			const [targets, suggestions] = await Promise.all([
				invoke<LinkTargets>('get_link_targets'),
				invoke<TagSuggestion[]>('list_tag_suggestions')
			]);
			if (seq === linksSeq) editor?.setLinks(linkData(targets, suggestions));
		} catch {
			// Links keep their last resolution; the next note switch retries.
		}
	}

	let backlinks = $state<{ id: number; items: Backlink[] } | null>(null);
	let backlinksOpen = $state(false); // survives note switches, not page loads
	let backlinksError = $state('');
	let backlinksSeq = 0;

	async function loadBacklinks(id: number): Promise<void> {
		const seq = ++backlinksSeq;
		try {
			const items = await invoke<Backlink[]>('list_backlinks', { id });
			if (seq !== backlinksSeq) return;
			backlinks = { id, items };
			backlinksError = '';
		} catch (e) {
			if (seq !== backlinksSeq) return;
			backlinks = { id, items: [] };
			backlinksError = String(e);
		}
	}

	const currentBacklinks = $derived(backlinks && backlinks.id === current?.id ? backlinks.items : null);

	/** ⌘-click in the editor, click in the read view. */
	function follow(link: LinkFollow): void {
		if (link.kind === 'tag') {
			query = `#${link.tag}`;
			listEl?.scrollTo({ top: 0 });
			return;
		}
		const wanted = link.title.trim().toLowerCase();
		const id = notes.find((n) => n.title.toLowerCase() === wanted)?.id ?? link.id;
		if (id !== null) void select(id);
		else createChain = createChain.then(() => createTitled(link.title.trim()));
	}

	/** An unresolved wikilink was followed: create the note with that title and open it. */
	async function createTitled(title: string): Promise<void> {
		actionError = '';
		const folderId = current?.folder_id ?? null; // the linking note's folder
		try {
			const seq = ++selectSeq;
			await commitTitle(true);
			await queue.flush();
			const note = await invoke<Note>('create_note_titled', { title, folderId });
			notes = upsertNote(notes, summaryOf(note));
			if (seq !== selectSeq) return;
			reading = false; // a new note is for writing
			show(note);
			await tick();
			editor?.focus();
		} catch (e) {
			actionError = String(e);
		}
	}

	/**
	 * After a rename rewrote `[[Old]]` in these notes: drop their cached editor state and
	 * queue entries, so nothing restores the old text, and reload the open one if affected.
	 * The queue was flushed before the rename, so nothing unsaved is dropped.
	 */
	async function reloadRewritten(ids: number[]): Promise<void> {
		const openId = current && ids.includes(current.id) ? current.id : null;
		let fresh: Note | null = null;
		if (openId !== null) {
			try {
				fresh = await invoke<Note>('get_note', { id: openId });
			} catch (e) {
				actionError = `Couldn't reload this note after the rename: ${String(e)}`;
			}
		}
		for (const id of ids) {
			queue.forget(id);
			editor?.forget(id);
		}
		void load(); // their excerpts changed
		if (openId === null || current?.id !== openId) return;
		if (fresh) show(fresh);
		else showNothing();
	}

	// ── List ──────────────────────────────────────────────────────────────────────

	// Search runs in Rust on query change only, so editing a note never drops it from the
	// results. Hits are resolved against `notes`, which edits keep current.
	let hits = $state<NoteHit[]>([]);
	let hitsFor = $state(''); // the query `hits` answer; differs while a search is in flight
	let searchError = $state('');
	let searchSeq = 0;
	const searchTerm = $derived(query.trim());

	$effect(() => {
		const term = searchTerm;
		const seq = ++searchSeq;
		if (!term) {
			searchError = '';
			return;
		}
		invoke<NoteHit[]>('search_notes', { query: term, limit: null })
			.then((rows) => {
				if (seq !== searchSeq) return;
				hits = rows;
				hitsFor = term;
				searchError = '';
			})
			.catch((e) => {
				if (seq !== searchSeq) return;
				hits = []; // don't show the previous query's rows under the error
				searchError = String(e);
			});
	});

	const visible = $derived(searchTerm ? searchRows(notes, hits) : plainRows(notes));
	const tree = $derived(buildTree(folders, notes));
	/** The `#tag` terms of the search, shown above the results. */
	const tagFilter = $derived(splitSearchQuery(searchTerm));
	const searching = $derived(searchTerm !== '' && hitsFor !== searchTerm && !searchError);

	async function load(): Promise<void> {
		const seq = ++loadSeq;
		try {
			const [rows, folderRows] = await Promise.all([invoke<NoteSummary[]>('list_notes'), invoke<Folder[]>('list_folders')]);
			if (seq !== loadSeq) return;
			notes = sortNotes(rows);
			folders = folderRows;
			// A move or folder delete may have changed the open note's folder.
			const open = current && rows.find((n) => n.id === current?.id);
			if (current && open && open.folder_id !== current.folder_id) current = { ...current, folder_id: open.folder_id };
			loadError = '';
			loaded = true;
		} catch (e) {
			if (seq !== loadSeq) return;
			loadError = String(e);
		}
	}

	/** A save, rename or pin result: update the row, and the open note when it is this one. */
	function applySummary(summary: NoteSummary): void {
		notes = upsertNote(notes, summary);
		if (current?.id === summary.id) {
			const { title, pinned, updated_at, folder_id } = summary;
			current = { ...current, title, pinned, updated_at, folder_id };
		}
	}

	/** List row for a just-created (empty) note. */
	function summaryOf(note: Note): NoteSummary {
		const { id, folder_id, title, pinned, created_at, updated_at } = note;
		return { id, folder_id, title, excerpt: '', pinned, created_at, updated_at };
	}

	// ── Folders ───────────────────────────────────────────────────────────────────

	const EXPANDED_SAVE_MS = 500;
	let expanded = $state<ReadonlySet<number>>(new Set());
	let storedExpanded: number[] = []; // what app_settings holds; writes are skipped when unchanged
	let expandedReady = false; // get_expanded_folders settled: no write may clobber the stored set before
	let expandedTimer: ReturnType<typeof setTimeout> | undefined;
	let currentFolderId = $state<number | null>(null); // new notes and folders go here (root: null)
	let renamingFolderId = $state<number | null>(null);

	/** The current folder, or root once it no longer exists. */
	const currentFolder = $derived(currentFolderId !== null && tree.byId.has(currentFolderId) ? currentFolderId : null);
	const currentFolderName = $derived(currentFolder === null ? null : (tree.byId.get(currentFolder)?.name ?? null));
	const treeRows = $derived(visibleRows(tree, expanded));

	async function loadExpanded(): Promise<void> {
		try {
			const ids = await invoke<number[]>('get_expanded_folders');
			storedExpanded = ids;
			expanded = new Set([...ids, ...expanded]); // keep folders opened meanwhile
		} catch {
			// Starts collapsed; the next toggle writes the set again.
		}
		expandedReady = true;
		if (!sameIds(expanded, storedExpanded)) scheduleSaveExpanded(); // folders opened meanwhile
	}

	function scheduleSaveExpanded(): void {
		clearTimeout(expandedTimer);
		expandedTimer = setTimeout(() => void saveExpanded(), EXPANDED_SAVE_MS);
	}

	function setExpanded(ids: number[], open: boolean): void {
		const next = new Set(open ? [...expanded, ...ids] : [...expanded].filter((id) => !ids.includes(id)));
		if (sameIds(next, expanded)) return;
		expanded = next;
		scheduleSaveExpanded();
	}

	async function saveExpanded(): Promise<void> {
		clearTimeout(expandedTimer);
		expandedTimer = undefined;
		if (!loaded || !expandedReady) return; // loadExpanded schedules the write once it settles
		const ids = [...expanded].filter((id) => tree.byId.has(id)).sort((a, b) => a - b);
		if (sameIds(ids, storedExpanded)) return;
		const previous = storedExpanded;
		storedExpanded = ids;
		try {
			storedExpanded = await invoke<number[]>('set_expanded_folders', { ids });
		} catch {
			storedExpanded = previous; // best effort: the next change retries
		}
	}

	// The open note's ancestors always expand (once per note and folder chain, so collapsing
	// them by hand sticks until the note or its place changes).
	let revealed = '';
	$effect(() => {
		const chain = current ? folderChain(tree.byId, current.folder_id) : [];
		const key = `${current?.id}:${chain.join(',')}`;
		if (key === revealed) return;
		revealed = key;
		if (chain.length) untrack(() => setExpanded(chain, true));
	});

	function toggleFolder(id: number): void {
		currentFolderId = id;
		setExpanded([id], !expanded.has(id));
	}

	/** Creates "New folder" (Rust picks a free name) and opens it in rename mode. */
	async function createFolder(parentId: number | null): Promise<void> {
		actionError = '';
		query = ''; // the tree, not search hits, shows the new folder
		try {
			const folder = await invoke<Folder>('create_folder', { name: null, parentId });
			folders = [...folders.filter((f) => f.id !== folder.id), folder];
			if (parentId !== null) setExpanded(folderChain(tree.byId, parentId), true);
			renamingFolderId = folder.id;
		} catch (e) {
			actionError = String(e);
		}
	}

	async function renameFolder(id: number, name: string): Promise<void> {
		const folder = await invoke<Folder>('rename_folder', { id, name });
		folders = folders.map((f) => (f.id === id ? folder : f));
	}

	/** Deletes a folder; Rust moves its notes and folders to its parent. */
	async function deleteFolder(id: number): Promise<void> {
		const parent = tree.byId.get(id)?.parent_id ?? null;
		actionError = '';
		try {
			await queue.flush(); // no save in flight can report a note's old folder afterwards
			await invoke('delete_folder', { id });
		} catch (e) {
			actionError = String(e);
			await load();
			return;
		}
		if (currentFolderId === id) currentFolderId = parent;
		storedExpanded = storedExpanded.filter((x) => x !== id); // Rust dropped it from the stored set
		setExpanded([id], false);
		await load();
	}

	/** Drag and drop or Move to…: moves locally at once, then persists and reloads. */
	async function moveItem(item: DragItem, folderId: number | null): Promise<void> {
		actionError = '';
		await queue.flush(); // a save in flight would report the note's old folder
		if (item.kind === 'note') {
			notes = notes.map((n) => (n.id === item.id ? { ...n, folder_id: folderId } : n));
			if (current?.id === item.id) {
				current = { ...current, folder_id: folderId };
				currentFolderId = folderId;
			}
		} else {
			folders = folders.map((f) => (f.id === item.id ? { ...f, parent_id: folderId } : f));
		}
		try {
			if (item.kind === 'note') applySummary(await invoke<NoteSummary>('move_note', { id: item.id, folderId }));
			else await invoke<Folder>('move_folder', { id: item.id, parentId: folderId });
		} catch (e) {
			actionError = String(e);
		}
		await load();
	}

	// "Move to…" picker (editor header for the open note, folder row actions for folders).
	let moving = $state<{ item: DragItem; name: string } | null>(null);
	let moveReturnFocus: HTMLElement | null = null;

	function openMove(item: DragItem, name: string): void {
		moveReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		moving = { item, name };
	}

	/** Where the item being moved lives now (root for none or an unknown folder). */
	const movingFrom = $derived.by(() => {
		if (!moving) return null;
		const id = moving.item.kind === 'note' ? moving.item.folderId : moving.item.parentId;
		return id !== null && tree.byId.has(id) ? id : null;
	});
	const moveChoices = $derived(
		moving
			? moveTargets(tree, moving.item.kind === 'folder' ? moving.item.id : null).filter(
					(t) => targetFolderId(String(t.value)) !== movingFrom
				)
			: []
	);

	async function closeMove(): Promise<void> {
		moving = null;
		await tick();
		if (moveReturnFocus?.isConnected) moveReturnFocus.focus();
		moveReturnFocus = null;
	}

	function pickMove(folderId: number | null): void {
		const item = moving?.item;
		void closeMove();
		if (item) void moveItem(item, folderId);
	}

	// ── Selection (lives in ?id=) ─────────────────────────────────────────────────

	function setUrl(id: number | null): void {
		const url = new URL(window.location.href);
		url.searchParams.delete('new');
		if (id === null) url.searchParams.delete('id');
		else url.searchParams.set('id', String(id));
		if (url.href === window.location.href) return;
		// Shallow + replace (replaceState): the URL tracks the selection without history entries.
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- the current URL with another ?id=
		void goto(url, { replace: true, shallow: true }).catch(() => {});
	}

	function show(note: Note): void {
		queue.track(note.id, note.body);
		selectedId = note.id;
		current = note;
		currentFolderId = note.folder_id; // opening a note makes its folder current
		titleDraft = note.title;
		titleError = ''; // switching away discards an invalid title
		lastFailedTitle = null;
		confirmingDelete = false;
		viewError = '';
		editor?.open(note.id, note.body);
		setUrl(note.id);
		if (reading) void renderCurrent();
		void loadBacklinks(note.id);
		void refreshLinks(); // other notes' tags and titles may have changed since
	}

	function showNothing(): void {
		selectedId = null;
		current = null;
		titleDraft = '';
		titleError = '';
		confirmingDelete = false;
		viewError = '';
		renderSeq++;
		rendered = null;
		renderError = '';
		backlinksSeq++;
		backlinks = null;
		editor?.close();
		setUrl(null);
	}

	/** Flushes pending saves, then opens `id` (null: nothing). A missing note selects nothing. */
	async function select(id: number | null): Promise<void> {
		if (id === selectedId) return;
		const seq = ++selectSeq;
		await commitTitle(true); // an invalid title is discarded by the switch
		if (seq !== selectSeq) return;
		await queue.flush(); // failures stay dirty and keep retrying
		if (seq !== selectSeq) return;
		if (id === null) return showNothing();
		try {
			const note = await invoke<Note>('get_note', { id });
			if (seq !== selectSeq) return;
			show(note);
		} catch (e) {
			if (seq !== selectSeq) return;
			if (String(e) === NOT_FOUND) {
				notes = notes.filter((n) => n.id !== id);
				showNothing();
			} else {
				actionError = String(e);
			}
		}
	}

	/** Focusable list rows: search hits, or the tree's visible rows. */
	function rowElements(): HTMLElement[] {
		return [...(listEl?.querySelectorAll<HTMLElement>('[data-row]') ?? [])];
	}

	function onRowKeydown(e: KeyboardEvent, id: number): void {
		if (e.target !== e.currentTarget) return; // keys on the row's own buttons
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			void select(id);
		} else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			const rows = rowElements();
			const index = rows.indexOf(e.currentTarget as HTMLElement);
			const next = e.key === 'ArrowDown' ? rows[index + 1] : rows[index - 1];
			// ↑ on the first row goes back to the search box.
			(next ?? (e.key === 'ArrowUp' ? searchInput : undefined))?.focus();
		}
	}

	function onSearchKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			query = '';
		} else if (e.key === 'ArrowDown') {
			e.preventDefault();
			rowElements()[0]?.focus();
		} else if (e.key === 'Enter' && !searching) {
			// Open the top match, or the first visible note of the tree.
			const first = searchTerm ? visible[0]?.note.id : treeRows.find((r) => r.kind === 'note')?.note.id;
			if (first === undefined) return;
			e.preventDefault();
			void select(first);
		}
	}

	// React to ?id= set from outside (palette "New note", deep links) while mounted.
	let seenUrlId: string | null | undefined;
	$effect(() => {
		const raw = page.url.searchParams.get('id');
		if (raw === seenUrlId) return;
		seenUrlId = raw;
		const id = Number(raw);
		if (raw !== null && Number.isInteger(id) && id > 0) untrack(() => void select(id));
	});

	// ── Create ────────────────────────────────────────────────────────────────────

	// The palette's "New note" navigates to ?new=1; create here so the title gets focus.
	$effect(() => {
		if (page.url.searchParams.get('new') !== null) untrack(() => requestCreate());
	});

	let createChain: Promise<void> = Promise.resolve();

	/**
	 * Creates are serialized: each request makes exactly one note, in order. The folder is
	 * taken at request time, so a later folder switch doesn't redirect a queued create.
	 */
	function requestCreate(folderId: number | null = currentFolder): void {
		createChain = createChain.then(() => createNote(folderId));
	}

	/** Tree menu "New note here" (null: root): the folder becomes current, then a create. */
	function newNoteIn(folderId: number | null): void {
		currentFolderId = folderId;
		requestCreate(folderId);
	}

	async function createNote(folderId: number | null): Promise<void> {
		actionError = '';
		try {
			const seq = ++selectSeq;
			await commitTitle(true); // ⌘N while renaming keeps the rename; an invalid title is discarded
			await queue.flush();
			const note = await invoke<Note>('create_note', { folderId });
			notes = upsertNote(notes, summaryOf(note));
			query = '';
			if (seq === selectSeq) {
				reading = false; // a new note is for writing
				show(note);
			}
			await tick();
			titleInput?.focus();
			titleInput?.select();
		} catch (e) {
			actionError = String(e);
		}
	}

	function onWindowKeydown(e: KeyboardEvent): void {
		if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
		if (e.code === 'KeyF') {
			e.preventDefault();
			searchInput?.focus();
			searchInput?.select();
			return;
		}
		if (e.code === 'KeyE') {
			e.preventDefault();
			if (!e.repeat) void toggleReading();
			return;
		}
		if (e.code !== 'KeyN') return;
		e.preventDefault();
		if (e.repeat) return; // held down: one note per press
		requestCreate();
	}

	// ── Title ─────────────────────────────────────────────────────────────────────

	/** Renames the open note to the draft; true when the stored title now matches it. */
	async function commitTitle(fromBlur = false): Promise<boolean> {
		if (!current) return true;
		const id = current.id;
		const title = titleDraft.trim();
		if (title === current.title) {
			titleDraft = current.title;
			titleError = '';
			return true;
		}
		// The same rename already in flight (a row click blurs the title first): wait for it, so
		// callers (switching, quitting) don't move on before the write settles.
		if (renaming && renaming.id === id && renaming.title === title) return renaming.done;
		if (fromBlur && title === lastFailedTitle) return false;
		const done = rename(id, title);
		renaming = { id, title, done };
		void done.finally(() => {
			if (renaming?.done === done) renaming = null;
		});
		return done;
	}

	async function rename(id: number, title: string): Promise<boolean> {
		try {
			// The rename rewrites [[links]] in stored bodies: store every edit first.
			if (!(await queue.flush())) throw new Error("Your notes couldn't be saved, so the title wasn't changed");
			const { note: summary, rewritten_ids } = await invoke<RenamedNote>('rename_note', { id, title });
			applySummary(summary);
			if (current?.id === id) {
				titleDraft = summary.title;
				titleError = '';
				lastFailedTitle = null;
			}
			if (rewritten_ids.length) await reloadRewritten(rewritten_ids);
			void refreshLinks();
			if (current) void loadBacklinks(current.id);
			return true;
		} catch (e) {
			if (current?.id === id) {
				titleError = e instanceof Error ? e.message : String(e);
				lastFailedTitle = title;
			}
			return false;
		}
	}

	async function onTitleKeydown(e: KeyboardEvent): Promise<void> {
		if (e.isComposing) return;
		if (e.key === 'Enter') {
			e.preventDefault();
			if (await commitTitle()) (reading ? reader : editor)?.focus();
		} else if (e.key === 'Escape' && current) {
			e.preventDefault();
			titleDraft = current.title;
			titleError = '';
			lastFailedTitle = null;
		}
	}

	// ── Pin / delete ──────────────────────────────────────────────────────────────

	async function togglePin(note: { id: number; pinned: boolean }): Promise<void> {
		actionError = '';
		try {
			applySummary(await invoke<NoteSummary>('set_note_pinned', { id: note.id, pinned: !note.pinned }));
		} catch (e) {
			actionError = String(e);
		}
	}

	async function deleteCurrent(): Promise<void> {
		if (!current) return;
		const id = current.id;
		const next = searchTerm ? neighborAfterRemoval(visible.map((r) => r.note.id), id) : noteNeighbour(tree, id);
		confirmingDelete = false;
		if (treeConfirmNoteId === id) treeConfirmNoteId = null;
		actionError = '';
		queue.pause(); // no autosave races the delete; the body stays dirty in case it fails
		try {
			await invoke('delete_note', { id });
		} catch (e) {
			if (String(e) !== NOT_FOUND) {
				actionError = String(e);
				void queue.flush(); // the note still exists: save what was pending
				void load();
				return;
			}
		}
		queue.forget(id); // a late save response is ignored
		editor?.forget(id);
		notes = notes.filter((n) => n.id !== id);
		void refreshLinks(); // links to it are unresolved now
		selectedId = null;
		current = null;
		if (next === null) showNothing();
		else await select(next);
	}

	/**
	 * Tree delete: the open note goes through `deleteCurrent`; any other note is deleted
	 * and dropped everywhere it is cached, so no save, banner or link points at it after.
	 */
	async function deleteNote(id: number): Promise<void> {
		if (current?.id === id) return deleteCurrent();
		if (treeConfirmNoteId === id) treeConfirmNoteId = null;
		actionError = '';
		try {
			await invoke('delete_note', { id });
		} catch (e) {
			if (String(e) !== NOT_FOUND) {
				actionError = String(e);
				void load();
				return;
			}
		}
		queue.forget(id); // stops retries of a failed save and clears its banner
		editor?.forget(id);
		notes = notes.filter((n) => n.id !== id);
		void refreshLinks(); // links to it are unresolved now
		if (current) void loadBacklinks(current.id);
	}

	/** One delete confirm per note: the editor header's replaces a tree row's, and back. */
	function confirmDeleteCurrent(): void {
		confirmingDelete = true;
		if (treeConfirmNoteId === current?.id) treeConfirmNoteId = null;
	}

	function setTreeConfirm(id: number | null): void {
		treeConfirmNoteId = id;
		if (id !== null && id === current?.id) confirmingDelete = false;
	}

	function onConfirmKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			e.preventDefault();
			confirmingDelete = false;
		}
	}

	function autofocus(node: HTMLElement): void {
		node.focus();
	}

	// ── Leaving ───────────────────────────────────────────────────────────────────

	let leaving = false;
	beforeNavigate((nav) => {
		if (leaving || nav.to?.route.id === '/notes' || !queue.busy) return;
		if (nav.type === 'leave') {
			void queue.flush(); // can't wait here; the quit/close guard covers app exit
			return;
		}
		nav.cancel();
		const target = nav.to?.url;
		void queue.flush().then((ok) => {
			if (!ok) {
				navError = "Your notes couldn't be saved, so you're still here. Fix the problem and try again.";
				return;
			}
			if (!target) return;
			leaving = true;
			// eslint-disable-next-line svelte/no-navigation-without-resolve -- resuming the navigation SvelteKit resolved
			void goto(target).finally(() => (leaving = false));
		});
	});

	function flushOnHide(): void {
		if (document.visibilityState === 'hidden') void queue.flush();
	}

	// ── Layout ────────────────────────────────────────────────────────────────────

	/** Panes fill the window below the hero and scroll on their own. */
	function measure(): void {
		if (!layout) return;
		const top = layout.getBoundingClientRect().top + window.scrollY;
		paneHeight = Math.max(420, window.innerHeight - top);
	}

	// Re-measure when the load error banner above the panes comes or goes.
	$effect(() => {
		void loadError;
		measure();
	});

	onMount(() => {
		void load();
		void loadExpanded();
		void refreshLinks();
		// Title and body are written concurrently so a slow rename can't eat the quit budget.
		const unregister = registerFlushHandler(async () => {
			const [, saved] = await Promise.all([commitTitle(true), queue.flush()]);
			return saved;
		});
		return () => {
			unregister();
			clearTimeout(copiedTimer);
			if (expandedTimer !== undefined) void saveExpanded();
			void queue.flush();
		};
	});
</script>

<svelte:window onkeydown={onWindowKeydown} onblur={() => void queue.flush()} onresize={measure} />
<svelte:document onvisibilitychange={flushOnHide} />

{#snippet dismissible(message: string, onDismiss: () => void)}
	<div class="notes-inline-error" role="alert" transition:fade={{ duration: 150 }}>
		<CircleAlert size={14} class="mt-0.5 shrink-0" />
		<span class="min-w-0 flex-1">{message}</span>
		<button type="button" class="shrink-0" onclick={onDismiss} title="Dismiss" aria-label="Dismiss"><X size={14} /></button>
	</div>
{/snippet}

{#snippet saveAlerts()}
	{#if navError}
		<div class="mt-2">{@render dismissible(navError, () => (navError = ''))}</div>
	{/if}
	{#each failedSaves as failure (failure.id)}
		<div class="notes-inline-error mt-2" role="alert" transition:fade={{ duration: 150 }}>
			<CircleAlert size={14} class="mt-0.5 shrink-0" />
			<span class="min-w-0 flex-1">
				Couldn't save {failure.id === selectedId ? 'this note' : `“${titleOf(failure.id)}”`}: {failure.error}. Your text is kept and saving retries as you type.
			</span>
			<button type="button" class="shrink-0 font-semibold underline-offset-2 hover:underline" onclick={retrySaves}>Retry</button>
		</div>
	{/each}
{/snippet}

{#snippet marked(segments: Segment[])}
	{#each segments as seg, i (i)}{#if seg.hit}<mark class="search-hit">{seg.text}</mark>{:else}{seg.text}{/if}{/each}
{/snippet}

<div class="page-shell">
	<div class="page-view">
		{#if loadError}
			<div class="alert-panel alert-error">
				<p class="alert-error-title">Could not load notes</p>
				<p class="alert-error-copy">{loadError}</p>
			</div>
		{/if}

		<div bind:this={layout} class="-mx-3 -mb-4 grid grid-cols-[18rem_minmax(0,1fr)] first:-mt-4 sm:-mx-4 lg:-mx-6" style="height: {paneHeight}px">
			<!-- List -->
			<aside class="notes-pane border-r border-slate-200 dark:border-neutral-800">
				<div class="notes-bar gap-2 px-3">
					<label class="relative min-w-0 flex-1">
						<Search size={15} class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400 dark:text-neutral-500" />
						<input
							bind:this={searchInput}
							bind:value={query}
							class="form-input py-1.5 pl-8 pr-8"
							placeholder="Search notes"
							aria-label="Search notes"
							autocomplete="off"
							spellcheck="false"
							onkeydown={onSearchKeydown}
						/>
						{#if query}
							<button
								type="button"
								class="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-md p-1 text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600 dark:text-neutral-500 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
								title="Clear search"
								aria-label="Clear search"
								onclick={() => {
									query = '';
									searchInput?.focus();
								}}
								transition:fade={{ duration: 150 }}
							>
								<X size={14} />
							</button>
						{/if}
					</label>
					<button
						type="button"
						class="btn-icon-secondary"
						title={currentFolderName ? `New folder in “${currentFolderName}”` : 'New folder'}
						aria-label="New folder"
						onclick={() => void createFolder(currentFolder)}
					>
						<FolderPlus size={16} />
					</button>
					<button
						type="button"
						class="btn-icon-secondary"
						title={currentFolderName ? `New note in “${currentFolderName}” (⌘N)` : 'New note (⌘N)'}
						aria-label="New note"
						onclick={() => requestCreate()}
					>
						<Plus size={16} />
					</button>
				</div>

				{#if actionError}
					<div class="px-3 pt-3">{@render dismissible(actionError, () => (actionError = ''))}</div>
				{/if}

				<div bind:this={listEl} class="min-h-0 flex-1 overflow-y-auto p-2">
					{#if !loaded && !loadError}
						<p class="px-3 py-6 text-center text-sm text-slate-400 dark:text-neutral-500">Loading…</p>
					{:else if notes.length === 0 && folders.length === 0}
						<div class="notes-empty" in:fade={{ duration: 150 }}>
							<NotebookPen size={22} class="text-slate-300 dark:text-neutral-600" />
							<p class="font-medium text-slate-600 dark:text-neutral-300">No notes yet</p>
							<p class="text-xs">Create one to start writing.</p>
						</div>
					{:else if !searchTerm}
						<NoteTree
							{tree}
							{expanded}
							{selectedId}
							currentFolderId={currentFolder}
							bind:renamingId={renamingFolderId}
							bind:confirmingNoteId={() => treeConfirmNoteId, setTreeConfirm}
							scrollEl={listEl}
							onopen={(id) => void select(id)}
							ontoggle={toggleFolder}
							onexpand={setExpanded}
							oncurrent={(id) => (currentFolderId = id)}
							onpin={(n) => void togglePin(n)}
							onnewfolder={(id) => void createFolder(id)}
							onnewnote={newNoteIn}
							onrename={renameFolder}
							ondelete={deleteFolder}
							ondeletenote={deleteNote}
							onmoveto={openMove}
							onmove={(item, folderId) => void moveItem(item, folderId)}
							onexit={() => searchInput?.focus()}
						/>
					{:else}
						{#if searchError}
							<div class="notes-inline-error mx-1" role="alert">
								<CircleAlert size={14} class="mt-0.5 shrink-0" />
								<span class="min-w-0 flex-1">Search failed: {searchError}</span>
							</div>
						{:else if visible.length === 0 && !searching}
							<div class="notes-empty" in:fade={{ duration: 150 }}>
								<Search size={20} class="text-slate-300 dark:text-neutral-600" />
								<p class="font-medium text-slate-600 dark:text-neutral-300">No matches</p>
								{#if tagFilter.tags.length && !tagFilter.text}
									<p class="text-xs">No notes are tagged {tagFilter.tags.map((t) => `#${t}`).join(' and ')}.</p>
								{:else}
									<p class="text-xs">Nothing matches “{searchTerm}”.</p>
								{/if}
							</div>
						{:else if tagFilter.tags.length && visible.length}
							<p class="notes-filter-hint" in:fade={{ duration: 150 }}>
								<span>{visible.length} {visible.length === 1 ? 'note' : 'notes'} tagged</span>
								{#each tagFilter.tags as tag (tag)}<span class="note-tag">#{tag}</span>{/each}
								{#if tagFilter.text}<span>with “{tagFilter.text}”</span>{/if}
							</p>
						{/if}
						<div role="list" aria-label="Search results">
							{#each visible as { note: n, title, snippet } (n.id)}
								{@const path = folderPath(tree.byId, n.folder_id)}
								<div role="listitem" class="group relative" animate:flip={{ duration: 180 }}>
									<div
										role="button"
										tabindex="0"
										class="note-row"
										class:note-row-active={n.id === selectedId}
										aria-current={n.id === selectedId ? 'true' : undefined}
										data-row
										onclick={() => void select(n.id)}
										onkeydown={(e) => onRowKeydown(e, n.id)}
									>
										<p class="note-row-title pr-6">{@render marked(title)}</p>
										{#if path}
											<p class="note-row-path"><FolderIcon size={11} class="shrink-0" /><span class="truncate">{path}</span></p>
										{/if}
										{#if snippet}
											<p class="note-row-snippet">{@render marked(snippet)}</p>
										{:else if n.excerpt}
											<p class="note-row-excerpt">{n.excerpt}</p>
										{/if}
									</div>
									<button
										type="button"
										class="note-row-pin"
										class:note-row-pin-on={n.pinned}
										title={n.pinned ? 'Unpin' : 'Pin to top'}
										aria-label={n.pinned ? `Unpin ${n.title}` : `Pin ${n.title}`}
										onclick={() => void togglePin(n)}
									>
										{#if n.pinned}
											<!-- Pinned: a filled pin; hovering the row offers unpin. -->
											<Pin size={13} class="fill-current group-hover:hidden" />
											<PinOff size={13} class="hidden group-hover:block" />
										{:else}
											<Pin size={13} />
										{/if}
									</button>
								</div>
							{/each}
						</div>
					{/if}
				</div>
			</aside>

			<!-- Editor -->
			<section class="notes-pane">
				{#if current}
					<!-- min-h-14 (border included) matches the search bar's h-14. -->
					<header class="flex min-h-14 shrink-0 flex-col justify-center border-b border-slate-100 px-6 dark:border-neutral-800">
						<div class="flex items-center gap-3">
							<input
								bind:this={titleInput}
								bind:value={titleDraft}
								class="note-title-input"
								class:note-title-input-error={titleError}
								placeholder="Untitled"
								aria-label="Note title"
								aria-invalid={titleError ? 'true' : undefined}
								autocomplete="off"
								spellcheck="false"
								onkeydown={onTitleKeydown}
								onblur={() => void commitTitle(true)}
							/>
							<span class="note-dates" title="Created {current.created_at.replace('T', ' ').slice(0, 16)} · Modified {current.updated_at.replace('T', ' ').slice(0, 16)}">
								Created {formatNoteDate(current.created_at, new Date())}
								<span aria-hidden="true">·</span>
								Modified {formatNoteDate(current.updated_at, new Date())}
							</span>
							<span class="note-save-state" aria-live="polite">
								{#if saveStatus === 'error'}
									<span class="text-red-600 dark:text-red-400">Not saved</span>
								{:else if saveStatus === 'saving' || saveStatus === 'pending'}
									<span class="h-1.5 w-1.5 animate-pulse rounded-full bg-sky-500 dark:bg-sky-400"></span>
									Saving…
								{:else}
									<Check size={13} class="text-emerald-500 dark:text-emerald-400" />
									Saved
								{/if}
							</span>
							{#if confirmingDelete}
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<div class="flex shrink-0 items-center gap-1.5" onkeydown={onConfirmKeydown} in:fade={{ duration: 150 }}>
									<span class="text-xs font-medium text-red-700 dark:text-red-300">Delete this note?</span>
									<button type="button" class="btn-danger-sm" use:autofocus onclick={deleteCurrent}>Delete</button>
									<button type="button" class="btn-secondary-sm" onclick={() => (confirmingDelete = false)}>Cancel</button>
								</div>
							{:else}
								<div class="flex shrink-0 items-center gap-0.5">
									<button
										type="button"
										class="btn-icon-ghost"
										title={copied ? 'Copied' : 'Copy as HTML'}
										aria-label={copied ? 'Copied as HTML' : 'Copy as HTML'}
										onclick={copyHtml}
									>
										{#if copied}
											<span class="inline-flex text-emerald-500 dark:text-emerald-400" in:fade={{ duration: 150 }}><Check size={16} /></span>
										{:else}
											<Copy size={16} />
										{/if}
									</button>
									<button
										type="button"
										class="btn-icon-ghost {reading ? 'text-sky-600 dark:text-sky-400' : ''}"
										title={reading ? 'Edit (⌘E)' : 'Read (⌘E)'}
										aria-label={reading ? 'Edit note' : 'Read note'}
										aria-pressed={reading}
										onclick={() => void toggleReading()}
									>
										{#if reading}<PencilLine size={16} />{:else}<BookOpen size={16} />{/if}
									</button>
									<button
										type="button"
										class="btn-icon-ghost"
										title={current.folder_id !== null && tree.byId.has(current.folder_id) ? `Move to… (in ${folderPath(tree.byId, current.folder_id)})` : 'Move to…'}
										aria-label="Move note to…"
										onclick={() => current && openMove({ kind: 'note', id: current.id, folderId: current.folder_id }, current.title)}
									>
										<FolderInput size={16} />
									</button>
									<button
										type="button"
										class="btn-icon-ghost {current.pinned ? 'text-amber-500 dark:text-amber-400' : ''}"
										title={current.pinned ? 'Unpin' : 'Pin to top'}
										aria-label={current.pinned ? 'Unpin note' : 'Pin note'}
										aria-pressed={current.pinned}
										onclick={() => current && void togglePin(current)}
									>
										<Pin size={16} class={current.pinned ? 'fill-current' : ''} />
									</button>
									<button
										type="button"
										class="btn-icon-ghost-danger"
										title="Delete note"
										aria-label="Delete note"
										onclick={confirmDeleteCurrent}
									>
										<Trash2 size={16} />
									</button>
								</div>
							{/if}
						</div>
						{#if titleError}
							<p class="form-error -mt-1.5 pb-2" role="alert" transition:fade={{ duration: 150 }}>{titleError}</p>
						{/if}
						{#if navError || failedSaves.length}
							<div class="pb-3">{@render saveAlerts()}</div>
						{/if}
						{#if viewError}
							<div class="pb-3">{@render dismissible(viewError, () => (viewError = ''))}</div>
						{/if}
					</header>
				{:else}
					{#if navError || failedSaves.length}
						<div class="px-6 pt-4">{@render saveAlerts()}</div>
					{/if}
					<div class="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center" in:fade={{ duration: 150 }}>
						<div class="rounded-2xl bg-slate-100 p-4 text-slate-400 ring-1 ring-slate-200 dark:bg-neutral-800 dark:text-neutral-500 dark:ring-neutral-700">
							<NotebookPen size={28} />
						</div>
						<div>
							<p class="text-base font-semibold text-slate-800 dark:text-neutral-200">
								{notes.length ? 'Select a note' : 'Write your first note'}
							</p>
							<p class="mt-1 text-sm">
								{notes.length ? 'Pick one from the list, or start a new one.' : 'Notes are markdown and save as you type.'}
							</p>
						</div>
						<button type="button" class="btn-primary gap-2" onclick={() => requestCreate()}>
							<Plus size={16} /> New note <kbd class="notes-btn-kbd">⌘N</kbd>
						</button>
					</div>
				{/if}
				{#if current && reading}
					{#if renderError}
						<div class="px-6 pt-4">{@render dismissible(`Couldn't show the read view: ${renderError}`, () => void toggleReading())}</div>
					{:else if rendered?.id === current.id}
						<NoteReader bind:this={reader} html={rendered.html} onerror={(m) => (viewError = m)} onfollow={follow} class="min-h-0 flex-1" />
					{/if}
				{/if}
				<!-- Hidden, not unmounted, while reading: it keeps each note's state and mounts the code colors. -->
				<NoteEditor bind:this={editor} onchange={onBodyChange} onfollow={follow} class={current && !reading ? 'min-h-0 flex-1' : 'hidden'} />
				{#if current}
					<!-- Notes linking here ([[this title]]); collapsed to a one-line bar by default. -->
					<div class="notes-backlinks">
						<button
							type="button"
							class="notes-backlinks-toggle"
							aria-expanded={backlinksOpen}
							aria-controls="notes-backlinks-list"
							onclick={() => (backlinksOpen = !backlinksOpen)}
						>
							<ChevronRight size={14} class="shrink-0 transition-transform duration-150 {backlinksOpen ? 'rotate-90' : ''}" />
							Linked mentions
							{#if currentBacklinks}
								<span class="notes-backlinks-count" in:fade={{ duration: 150 }}>{currentBacklinks.length}</span>
							{/if}
						</button>
						{#if backlinksOpen}
							<div id="notes-backlinks-list" class="notes-backlinks-list" transition:fade={{ duration: 150 }}>
								{#if backlinksError}
									<p class="form-error px-2 pb-2">Couldn't load linked mentions: {backlinksError}</p>
								{:else if currentBacklinks?.length === 0}
									<p class="notes-backlinks-empty">No notes link to “{current.title}” yet. Link one with [[{current.title}]].</p>
								{:else if currentBacklinks}
									<ul>
										{#each currentBacklinks as link (link.id)}
											<li>
												<button type="button" class="notes-backlink" onclick={() => void select(link.id)}>
													<span class="notes-backlink-title">{link.title}</span>
													{#if link.snippet.trim()}
														<span class="notes-backlink-snippet">{link.snippet.trim()}</span>
													{/if}
												</button>
											</li>
										{/each}
									</ul>
								{/if}
							</div>
						{/if}
					</div>
				{/if}
			</section>
		</div>
	</div>
</div>

{#if moving}
	<MoveToDialog
		name={moving.name}
		location={movingFrom === null ? '' : folderPath(tree.byId, movingFrom)}
		targets={moveChoices}
		onpick={pickMove}
		onclose={() => void closeMove()}
	/>
{/if}
