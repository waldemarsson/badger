<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { onMount, tick, untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { filterNotes, neighborAfterRemoval, relativeDate, sortNotes, upsertNote } from '#lib/notes.js';
	import { NoteSaveQueue } from '#lib/note-save-queue.js';
	import { changes } from '#lib/palette-state.svelte.js';
	import { registerFlushHandler } from '#lib/flush-guard.js';
	import type { Note, NoteSummary } from '#lib/types.js';
	import NoteEditor from '#lib/components/NoteEditor.svelte';
	import Check from 'lucide-svelte/icons/check';
	import CircleAlert from 'lucide-svelte/icons/circle-alert';
	import NotebookPen from 'lucide-svelte/icons/notebook-pen';
	import Pin from 'lucide-svelte/icons/pin';
	import PinOff from 'lucide-svelte/icons/pin-off';
	import Plus from 'lucide-svelte/icons/plus';
	import Search from 'lucide-svelte/icons/search';
	import Trash2 from 'lucide-svelte/icons/trash-2';
	import X from 'lucide-svelte/icons/x';

	const NOT_FOUND = 'Note not found';

	let notes = $state<NoteSummary[]>([]);
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

	let confirmingDelete = $state(false);
	let actionError = $state('');
	let navError = $state('');
	let now = $state(new Date());

	let editor = $state<ReturnType<typeof NoteEditor>>();
	let titleInput = $state<HTMLInputElement>();
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

	// ── List ──────────────────────────────────────────────────────────────────────

	const visible = $derived(filterNotes(notes, query));

	async function load(): Promise<void> {
		const seq = ++loadSeq;
		try {
			const rows = await invoke<NoteSummary[]>('list_notes');
			if (seq !== loadSeq) return;
			notes = sortNotes(rows);
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
			current = { ...current, title: summary.title, pinned: summary.pinned, updated_at: summary.updated_at };
		}
	}

	/** List row for a just-created (empty) note. */
	function summaryOf(note: Note): NoteSummary {
		const { id, folder_id, title, pinned, created_at, updated_at } = note;
		return { id, folder_id, title, excerpt: '', pinned, created_at, updated_at };
	}

	// ── Selection (lives in ?id=) ─────────────────────────────────────────────────

	function setUrl(id: number | null): void {
		const url = new URL(window.location.href);
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
		titleDraft = note.title;
		titleError = ''; // switching away discards an invalid title
		lastFailedTitle = null;
		confirmingDelete = false;
		editor?.open(note.id, note.body);
		setUrl(note.id);
	}

	function showNothing(): void {
		selectedId = null;
		current = null;
		titleDraft = '';
		titleError = '';
		confirmingDelete = false;
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

	function onRowKeydown(e: KeyboardEvent, id: number): void {
		if (e.target !== e.currentTarget) return; // keys on the row's own buttons
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			void select(id);
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

	let createChain: Promise<void> = Promise.resolve();

	/** Creates are serialized: each request makes exactly one note, in order. */
	function requestCreate(): void {
		createChain = createChain.then(createNote);
	}

	async function createNote(): Promise<void> {
		actionError = '';
		try {
			const seq = ++selectSeq;
			await commitTitle(true); // ⌘N while renaming keeps the rename; an invalid title is discarded
			await queue.flush();
			const note = await invoke<Note>('create_note');
			notes = upsertNote(notes, summaryOf(note));
			query = '';
			if (seq === selectSeq) show(note);
			await tick();
			titleInput?.focus();
			titleInput?.select();
		} catch (e) {
			actionError = String(e);
		}
	}

	function onWindowKeydown(e: KeyboardEvent): void {
		if (e.code !== 'KeyN' || !e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
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
			const summary = await invoke<NoteSummary>('rename_note', { id, title });
			applySummary(summary);
			if (current?.id === id) {
				titleDraft = summary.title;
				titleError = '';
				lastFailedTitle = null;
			}
			return true;
		} catch (e) {
			if (current?.id === id) {
				titleError = String(e);
				lastFailedTitle = title;
			}
			return false;
		}
	}

	async function onTitleKeydown(e: KeyboardEvent): Promise<void> {
		if (e.isComposing) return;
		if (e.key === 'Enter') {
			e.preventDefault();
			if (await commitTitle()) editor?.focus();
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
		const next = neighborAfterRemoval(visible.map((n) => n.id), id);
		confirmingDelete = false;
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
		selectedId = null;
		current = null;
		if (next === null) showNothing();
		else await select(next);
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
		paneHeight = Math.max(420, window.innerHeight - top - 16);
	}

	// Re-measure when the load error banner above the panes comes or goes.
	$effect(() => {
		void loadError;
		measure();
	});

	onMount(() => {
		void load();
		const clock = setInterval(() => (now = new Date()), 60_000);
		// Title and body are written concurrently so a slow rename can't eat the quit budget.
		const unregister = registerFlushHandler(async () => {
			const [, saved] = await Promise.all([commitTitle(true), queue.flush()]);
			return saved;
		});
		return () => {
			clearInterval(clock);
			unregister();
			void queue.flush();
		};
	});

	// Reload when the command palette creates a note.
	let seenNoteChanges = changes.notes;
	$effect(() => {
		if (changes.notes === seenNoteChanges) return;
		seenNoteChanges = changes.notes;
		void load();
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

<div class="page-shell">
	<div class="page-view">
		<section class="page-hero">
			<p class="page-hero-kicker">TimeTracker</p>
			<h1 class="page-hero-title">Notes</h1>
			<p class="page-hero-copy">
				Markdown notes that save as you type.
				<span class="text-slate-400 dark:text-neutral-400">Press <kbd class="notes-hero-kbd">⌘N</kbd> for a new note.</span>
			</p>
		</section>

		{#if loadError}
			<div class="alert-panel alert-error">
				<p class="alert-error-title">Could not load notes</p>
				<p class="alert-error-copy">{loadError}</p>
			</div>
		{/if}

		<div bind:this={layout} class="grid grid-cols-[18rem_minmax(0,1fr)] gap-4" style="height: {paneHeight}px">
			<!-- List -->
			<aside class="notes-pane">
				<div class="flex items-center gap-2 border-b border-slate-100 p-3 dark:border-neutral-800">
					<label class="relative min-w-0 flex-1">
						<Search size={15} class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400 dark:text-neutral-500" />
						<input
							bind:value={query}
							class="form-input py-1.5 pl-8"
							placeholder="Search notes"
							aria-label="Search notes"
							autocomplete="off"
							spellcheck="false"
							onkeydown={(e) => e.key === 'Escape' && (query = '')}
						/>
					</label>
					<button type="button" class="btn-icon-secondary" title="New note (⌘N)" aria-label="New note" onclick={requestCreate}>
						<Plus size={16} />
					</button>
				</div>

				{#if actionError}
					<div class="px-3 pt-3">{@render dismissible(actionError, () => (actionError = ''))}</div>
				{/if}

				<div class="min-h-0 flex-1 overflow-y-auto p-2" role="list" aria-label="Notes">
					{#if !loaded && !loadError}
						<p class="px-3 py-6 text-center text-sm text-slate-400 dark:text-neutral-500">Loading…</p>
					{:else if notes.length === 0}
						<div class="notes-empty" in:fade={{ duration: 150 }}>
							<NotebookPen size={22} class="text-slate-300 dark:text-neutral-600" />
							<p class="font-medium text-slate-600 dark:text-neutral-300">No notes yet</p>
							<p class="text-xs">Create one to start writing.</p>
						</div>
					{:else if visible.length === 0}
						<div class="notes-empty" in:fade={{ duration: 150 }}>
							<Search size={20} class="text-slate-300 dark:text-neutral-600" />
							<p class="font-medium text-slate-600 dark:text-neutral-300">No matches</p>
							<p class="text-xs">Nothing matches “{query.trim()}”.</p>
						</div>
					{/if}
					{#each visible as n (n.id)}
						<div
							role="listitem"
							class="group relative"
							animate:flip={{ duration: 180 }}
						>
							<div
								role="button"
								tabindex="0"
								class="note-row"
								class:note-row-active={n.id === selectedId}
								aria-current={n.id === selectedId ? 'true' : undefined}
								onclick={() => void select(n.id)}
								onkeydown={(e) => onRowKeydown(e, n.id)}
							>
								<div class="flex min-w-0 items-center gap-1.5 pr-6">
									{#if n.pinned}
										<Pin size={12} class="shrink-0 fill-current text-amber-500 dark:text-amber-400" aria-label="Pinned" />
									{/if}
									<p class="note-row-title">{n.title}</p>
								</div>
								{#if n.excerpt}
									<p class="note-row-excerpt">{n.excerpt}</p>
								{/if}
								<p class="note-row-date">{relativeDate(n.updated_at, now)}</p>
							</div>
							<button
								type="button"
								class="note-row-pin {n.pinned ? 'note-row-pin-on' : ''}"
								title={n.pinned ? 'Unpin' : 'Pin to top'}
								aria-label={n.pinned ? `Unpin ${n.title}` : `Pin ${n.title}`}
								onclick={() => void togglePin(n)}
							>
								{#if n.pinned}<PinOff size={13} />{:else}<Pin size={13} />{/if}
							</button>
						</div>
					{/each}
				</div>
			</aside>

			<!-- Editor -->
			<section class="notes-pane">
				{#if current}
					<header class="border-b border-slate-100 px-6 pb-3 pt-4 dark:border-neutral-800">
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
										onclick={() => (confirmingDelete = true)}
									>
										<Trash2 size={16} />
									</button>
								</div>
							{/if}
						</div>
						{#if titleError}
							<p class="form-error mt-1" role="alert" transition:fade={{ duration: 150 }}>{titleError}</p>
						{/if}
						{@render saveAlerts()}
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
						<button type="button" class="btn-primary gap-2" onclick={requestCreate}>
							<Plus size={16} /> New note <kbd class="notes-btn-kbd">⌘N</kbd>
						</button>
					</div>
				{/if}
				<NoteEditor bind:this={editor} onchange={onBodyChange} class={current ? 'min-h-0 flex-1' : 'hidden'} />
			</section>
		</div>
	</div>
</div>
