<script lang="ts">
	// The scratchpad: one persistent buffer in a left slide-over drawer, mounted once in the
	// layout so it works on every page. No backdrop: the page stays usable. Autosaves like
	// notes (own queue, flushed on blur, close and quit). Keys pressed inside it stay inside
	// (⌘N/⌘F/⌘E on /notes don't fire); ⌘K and ⌘J are capture-phase and still work.
	import { invoke } from '@tauri-apps/api/core';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount, tick, untrack } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { NoteSaveQueue } from '#lib/note-save-queue.js';
	import { registerFlushHandler } from '#lib/flush-guard.js';
	import { linkData } from '#lib/editor/links.js';
	import { isBlank, PAD_ID, panelKeyAction } from '#lib/scratchpad.js';
	import { closeScratchpad, scratchpad, takeReturnFocus } from '#lib/scratchpad-state.svelte.js';
	import { palette, showToast } from '#lib/palette-state.svelte.js';
	import { SCRATCHPAD_SHORTCUT } from '#lib/palette.js';
	import { JSON_TOO_LARGE, overJsonLimit } from '#lib/notes.js';
	import type { JsonFormatMode } from '#lib/json-format.js';
	import type { LinkTargets, Note, NoteKind, RenderMode, Scratchpad, TagSuggestion } from '#lib/types.js';
	import NoteEditor from '#lib/components/NoteEditor.svelte';
	import NoteReader from '#lib/components/NoteReader.svelte';
	import BookOpen from '@lucide/svelte/icons/book-open';
	import Braces from '@lucide/svelte/icons/braces';
	import Check from '@lucide/svelte/icons/check';
	import CircleAlert from '@lucide/svelte/icons/circle-exclamation-point';
	import Copy from '@lucide/svelte/icons/copy';
	import Eraser from '@lucide/svelte/icons/eraser';
	import FilePlus from '@lucide/svelte/icons/file-plus';
	import PencilLine from '@lucide/svelte/icons/pencil-line';
	import StickyNote from '@lucide/svelte/icons/sticky-note';
	import X from '@lucide/svelte/icons/x';

	let panel = $state<HTMLElement>();
	let editor = $state<ReturnType<typeof NoteEditor>>();
	let reader = $state<ReturnType<typeof NoteReader>>();

	/** The pad's live text (edits included); the editor is recreated from it on each open. */
	let text = $state('');
	/** Markdown or JSON; kept by Clear and Save as note. */
	let kind = $state<NoteKind>('markdown');
	let loadError = $state('');
	let loading: Promise<void> | null = null;
	let loadSeq = 0;
	const blank = $derived(isBlank(text));

	// ── Autosave ──────────────────────────────────────────────────────────────────

	let queueVersion = $state(0);
	const queue = new NoteSaveQueue<Scratchpad>({
		save: (_id, body) => invoke<Scratchpad>('set_scratchpad_body', { body }),
		onSaved: () => (openedOverLimit = false),
		onChange: () => queueVersion++,
		isGone: () => false,
		// Over the JSON limit: resending the same text can't work; the next edit is tried.
		isPermanent: (e) => String(e) === JSON_TOO_LARGE
	});
	const saveStatus = $derived.by(() => {
		void queueVersion;
		return queue.status(PAD_ID);
	});
	const saveError = $derived.by(() => {
		void queueVersion;
		return queue.error(PAD_ID);
	});

	function onBodyChange(_id: number, body: string): void {
		text = body;
		formatError = '';
		queue.edit(PAD_ID, body);
	}

	// ── JSON mode ─────────────────────────────────────────────────────────────────

	let formatError = $state(''); // "Invalid JSON: …" from Format, Minify or ⇧⌥F
	let switching = $state(false);
	let openedOverLimit = $state(false); // the JSON pad was already over the limit when shown (a restore)
	const overLimit = $derived(kind === 'json' && (openedOverLimit || saveError === JSON_TOO_LARGE));

	/** Switches the pad's editor; its text (unsaved edits too) is stored with the switch, unchanged. */
	async function setKind(next: NoteKind): Promise<void> {
		if (next === kind || switching) return;
		switching = true;
		actionError = '';
		formatError = '';
		try {
			await queue.flush(); // a save in flight lands first; a failed one is carried below
			const body = text;
			const pad = await invoke<Scratchpad>('set_scratchpad_kind', { kind: next, body });
			queue.markSaved(PAD_ID, body);
			kind = pad.kind;
			openedOverLimit = overJsonLimit(kind, body);
			editor?.setKind(kind);
			if (reading && kind === 'markdown') void renderCurrent();
		} catch (e) {
			actionError = String(e);
		} finally {
			switching = false;
		}
	}

	function formatPad(mode: JsonFormatMode): void {
		formatError = editor?.format(mode) ?? '';
	}

	/**
	 * Cancels any pending save (saving it first, so one in flight can't land after the
	 * reset). Returns the body now stored, or null when that save failed.
	 */
	async function resetQueue(): Promise<string | null> {
		const body = text;
		const stored = await queue.flush();
		queue.forget(PAD_ID);
		return stored ? body : null;
	}

	/** After a failed Save as note or Clear: the pad is unchanged, keep autosaving it. */
	function resumeQueue(stored: string | null): void {
		if (stored !== null) queue.track(PAD_ID, stored);
		if (stored !== text) queue.edit(PAD_ID, text);
	}

	/** Reads the stored pad (on launch and after a restore). */
	function load(): Promise<void> {
		const seq = ++loadSeq;
		loading = (async () => {
			try {
				const pad = await invoke<Scratchpad>('get_scratchpad');
				if (seq !== loadSeq) return;
				queue.track(PAD_ID, pad.body);
				text = pad.body;
				kind = pad.kind;
				loadError = '';
			} catch (e) {
				if (seq === loadSeq) loadError = String(e);
			}
		})();
		return loading;
	}

	/** Shows `body` in a fresh editor state (no undo back to the old text). */
	function showText(body: string): void {
		text = body;
		openedOverLimit = overJsonLimit(kind, body);
		editor?.forget(PAD_ID);
		editor?.open(PAD_ID, body, kind);
	}

	// ── Open / close ──────────────────────────────────────────────────────────────

	let reading = $state(false);
	/** JSON has no read view: the editor shows while `reading` stays on for markdown. */
	const readView = $derived(reading && kind === 'markdown');
	let rendered = $state<string | null>(null);
	let renderError = $state('');
	let renderSeq = 0;
	let viewError = $state(''); // copy or open-link failures
	let copied = $state(false);
	let copiedTimer: ReturnType<typeof setTimeout> | undefined;

	let naming = $state(false);
	let titleDraft = $state('');
	let titleError = $state('');
	let savingNote = $state(false);
	let titleInput = $state<HTMLInputElement>();

	let confirmingClear = $state(false);
	let clearing = $state(false);
	let actionError = $state('');

	// While Save as note or Clear commits, the pad is read-only: nothing typed meanwhile
	// can be lost or written back after the pad was cleared.
	$effect(() => {
		editor?.setEditable(!savingNote && !clearing);
	});

	async function opened(): Promise<void> {
		reading = false;
		rendered = null;
		renderError = '';
		viewError = '';
		naming = false;
		titleError = '';
		confirmingClear = false;
		actionError = '';
		formatError = '';
		if (loadError) await load(); // retry: never let typing overwrite a pad we couldn't read
		else if (loading) await loading;
		await tick();
		if (!scratchpad.open) return;
		openedOverLimit = overJsonLimit(kind, text);
		editor?.open(PAD_ID, text, kind);
		editor?.focusEnd();
		void refreshLinks();
	}

	function closed(): void {
		void queue.flush();
		renderSeq++;
		clearTimeout(copiedTimer);
		copied = false;
		const back = takeReturnFocus();
		// Only pull focus back when it was in the drawer (or nowhere), not from the page.
		const active = document.activeElement;
		if (!active || active === document.body || panel?.contains(active)) {
			if (back?.isConnected) back.focus();
			else if (active instanceof HTMLElement && panel?.contains(active)) active.blur();
		}
	}

	let wasOpen = false;
	$effect(() => {
		const isOpen = scratchpad.open;
		untrack(() => {
			if (isOpen && !wasOpen) void opened();
			else if (!isOpen && wasOpen) closed();
		});
		wasOpen = isOpen;
	});

	// A restore replaced the pad: drop local state (pending save included) and reload.
	let seenRestore = scratchpad.restored;
	$effect(() => {
		if (scratchpad.restored === seenRestore) return;
		seenRestore = scratchpad.restored;
		untrack(() => void reloadAfterRestore());
	});

	async function reloadAfterRestore(): Promise<void> {
		queue.forget(PAD_ID);
		await load();
		if (scratchpad.open && !readView) showText(text);
		else editor?.forget(PAD_ID);
		if (readView) void renderCurrent();
	}

	/** What wikilinks and chips resolve to, and the `#` suggestions (following stays off). */
	async function refreshLinks(): Promise<void> {
		try {
			const [targets, suggestions] = await Promise.all([
				invoke<LinkTargets>('get_link_targets'),
				invoke<TagSuggestion[]>('list_tag_suggestions')
			]);
			editor?.setLinks(linkData(targets, suggestions));
		} catch {
			// Links render unresolved; writing still works.
		}
	}

	// ── Read view (⌘E) and copy as HTML ───────────────────────────────────────────

	async function renderCurrent(): Promise<void> {
		const seq = ++renderSeq;
		renderError = '';
		try {
			const html = await invoke<string>('render_markdown', { markdown: text, mode: 'read' satisfies RenderMode });
			if (seq === renderSeq) rendered = html;
		} catch (e) {
			if (seq === renderSeq) renderError = String(e);
		}
	}

	async function toggleReading(): Promise<void> {
		if (naming || kind === 'json') return; // ⌘E does nothing in JSON
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
		if (blank || kind === 'json') return;
		viewError = '';
		const markdown = text;
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

	// ── Save as note ──────────────────────────────────────────────────────────────

	async function startNaming(): Promise<void> {
		if (blank) return;
		confirmingClear = false;
		actionError = '';
		titleDraft = '';
		titleError = '';
		naming = true;
		await tick();
		titleInput?.focus();
	}

	async function cancelNaming(): Promise<void> {
		naming = false;
		titleError = '';
		await tick();
		if (readView) reader?.focus();
		else editor?.focus();
	}

	async function saveAsNote(): Promise<void> {
		if (savingNote || blank) return;
		savingNote = true;
		titleError = '';
		let stored: string | null = null;
		try {
			stored = await resetQueue();
			const note = await invoke<Note>('save_scratchpad_as_note', { title: titleDraft, body: text });
			queue.forget(PAD_ID); // edits typed meanwhile went into the note's text or are dropped
			queue.track(PAD_ID, '');
			showText('');
			naming = false;
			showToast(`Saved as “${note.title}”`);
			closeScratchpad();
			// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
			void goto(`${resolve('/notes')}?id=${note.id}`);
		} catch (e) {
			titleError = String(e);
			resumeQueue(stored);
			await tick();
			titleInput?.focus();
		} finally {
			savingNote = false;
		}
	}

	function onTitleKeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		if (e.key === 'Enter') {
			e.preventDefault();
			void saveAsNote();
		} else if (e.key === 'Escape') {
			e.preventDefault(); // cancels the prompt, not the drawer
			void cancelNaming();
		}
	}

	// ── Clear ─────────────────────────────────────────────────────────────────────

	function startClear(): void {
		if (blank) return;
		naming = false;
		titleError = '';
		actionError = '';
		confirmingClear = true;
	}

	async function cancelClear(): Promise<void> {
		confirmingClear = false;
		await tick();
		if (readView) reader?.focus();
		else editor?.focus();
	}

	async function clearPad(): Promise<void> {
		if (clearing) return;
		clearing = true;
		actionError = '';
		let stored: string | null = null;
		try {
			stored = await resetQueue();
			await invoke<Scratchpad>('set_scratchpad_body', { body: '' });
			queue.track(PAD_ID, '');
			showText('');
			confirmingClear = false;
			if (reading) {
				reading = false;
				rendered = null;
			}
			await tick();
			editor?.focus();
		} catch (e) {
			actionError = `Couldn't clear the scratchpad: ${e}`;
			resumeQueue(stored);
		} finally {
			clearing = false;
		}
	}

	function onConfirmKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			e.preventDefault();
			void cancelClear();
		}
	}

	// ── Keys and focus ────────────────────────────────────────────────────────────

	/** Bubble phase: inner handlers (menus, the title prompt) run first and may claim Esc. */
	function onPanelKeydown(e: KeyboardEvent): void {
		const action = panelKeyAction(e);
		if (action === 'close') {
			e.preventDefault();
			closeScratchpad();
		} else if (action === 'toggle-read' || action === 'repeat') {
			e.preventDefault();
			if (action === 'toggle-read') void toggleReading();
		}
		// Keep page shortcuts (⌘N, ⌘F, ⌘E on /notes) from seeing keys typed here.
		e.stopPropagation();
	}

	function onPanelFocusout(e: FocusEvent): void {
		const next = e.relatedTarget;
		if (!(next instanceof Node) || !panel?.contains(next)) void queue.flush();
	}

	function autofocus(node: HTMLElement): void {
		node.focus();
	}

	onMount(() => {
		void load();
		const unregister = registerFlushHandler(() => queue.flush());
		return () => {
			unregister();
			clearTimeout(copiedTimer);
			void queue.flush();
		};
	});
</script>

<svelte:window onblur={() => void queue.flush()} />

{#snippet dismissible(message: string, onDismiss: () => void)}
	<div class="notes-inline-error mt-3" role="alert" transition:fade={{ duration: 150 }}>
		<CircleAlert size={14} class="mt-0.5 shrink-0" />
		<span class="min-w-0 flex-1">{message}</span>
		<button type="button" class="shrink-0" onclick={onDismiss} title="Dismiss" aria-label="Dismiss"><X size={14} /></button>
	</div>
{/snippet}

{#if scratchpad.open}
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<aside
		bind:this={panel}
		class="scratchpad-drawer"
		aria-label="Scratchpad"
		onkeydown={onPanelKeydown}
		onfocusout={onPanelFocusout}
		transition:fly={{ x: -48, duration: 180 }}
	>
		<header class="scratchpad-header">
			<div class="flex items-center gap-2">
				<span class="scratchpad-icon"><StickyNote size={16} /></span>
				<h2 class="text-sm font-semibold text-slate-900 dark:text-neutral-100">Scratchpad</h2>
				<span class="note-save-state ml-1" aria-live="polite">
					{#if saveStatus === 'error'}
						<span class="text-red-600 dark:text-red-400">Not saved</span>
					{:else if saveStatus === 'saving' || saveStatus === 'pending'}
						<span class="h-1.5 w-1.5 animate-pulse rounded-full bg-sky-500 dark:bg-sky-400"></span>
						Saving…
					{:else if !blank}
						<Check size={13} class="text-emerald-500 dark:text-emerald-400" />
						Saved
					{/if}
				</span>
				<div class="ml-auto flex shrink-0 items-center gap-0.5">
					{#if kind === 'json'}
						<button type="button" class="json-action" title="Format JSON (⇧⌥F)" onclick={() => formatPad('format')}>Format</button>
						<button type="button" class="json-action" title="Minify JSON" onclick={() => formatPad('minify')}>Minify</button>
					{:else}
						<button
							type="button"
							class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40"
							title={copied ? 'Copied' : 'Copy as HTML'}
							aria-label={copied ? 'Copied as HTML' : 'Copy as HTML'}
							disabled={blank}
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
							aria-label={reading ? 'Edit scratchpad' : 'Read scratchpad'}
							aria-pressed={reading}
							onclick={() => void toggleReading()}
						>
							{#if reading}<PencilLine size={16} />{:else}<BookOpen size={16} />{/if}
						</button>
					{/if}
					<button
						type="button"
						class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40 {kind === 'json' ? 'text-sky-600 dark:text-sky-400' : ''}"
						title={kind === 'json' ? 'JSON: switch to Markdown' : 'Switch to JSON'}
						aria-label={kind === 'json' ? 'Switch scratchpad to Markdown' : 'Switch scratchpad to JSON'}
						aria-pressed={kind === 'json'}
						disabled={switching || savingNote || clearing || !!loadError}
						onclick={() => void setKind(kind === 'json' ? 'markdown' : 'json')}
					>
						<Braces size={16} />
					</button>
					<button
						type="button"
						class="btn-icon-ghost disabled:pointer-events-none disabled:opacity-40"
						title="Save as note"
						aria-label="Save as note"
						disabled={blank}
						onclick={() => void startNaming()}
					>
						<FilePlus size={16} />
					</button>
					<button
						type="button"
						class="btn-icon-ghost-danger disabled:pointer-events-none disabled:opacity-40"
						title="Clear"
						aria-label="Clear scratchpad"
						disabled={blank}
						onclick={startClear}
					>
						<Eraser size={16} />
					</button>
					<span class="mx-1 h-4 w-px bg-slate-200 dark:bg-neutral-800" aria-hidden="true"></span>
					<button type="button" class="btn-icon-ghost" title="Close (Esc)" aria-label="Close scratchpad" onclick={closeScratchpad}>
						<X size={16} />
					</button>
				</div>
			</div>

			{#if naming}
				<div class="mt-3" transition:fade={{ duration: 150 }}>
					<div class="flex items-center gap-2">
						<input
							bind:this={titleInput}
							bind:value={titleDraft}
							class="name-input"
							class:name-input-error={titleError}
							placeholder="Note title"
							aria-label="Note title"
							aria-invalid={titleError ? 'true' : undefined}
							aria-describedby={titleError ? 'scratchpad-title-error' : undefined}
							autocomplete="off"
							spellcheck="false"
							oninput={() => (titleError = '')}
							onkeydown={onTitleKeydown}
						/>
						<button type="button" class="btn-primary shrink-0 rounded-lg px-3 py-1.5 text-xs" disabled={savingNote || blank} onclick={() => void saveAsNote()}>Save</button>
						<button type="button" class="btn-secondary-sm shrink-0" onclick={() => void cancelNaming()}>Cancel</button>
					</div>
					{#if titleError}
						<p id="scratchpad-title-error" class="form-error mt-1.5" role="alert">{titleError}</p>
					{:else}
						<p class="mt-1.5 text-xs text-slate-400 dark:text-neutral-500">Creates a note with the whole pad, then clears it.</p>
					{/if}
				</div>
			{/if}

			{#if confirmingClear}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div class="scratchpad-confirm" onkeydown={onConfirmKeydown} transition:fade={{ duration: 150 }}>
					<span class="min-w-0 flex-1 text-xs font-medium text-red-700 dark:text-red-300">Clear the scratchpad? This can't be undone.</span>
					<button type="button" class="btn-danger-sm shrink-0" use:autofocus disabled={clearing} onclick={() => void clearPad()}>Clear</button>
					<button type="button" class="btn-secondary-sm shrink-0" onclick={() => void cancelClear()}>Cancel</button>
				</div>
			{/if}

			{#if loadError}
				<div class="notes-inline-error mt-3" role="alert">
					<CircleAlert size={14} class="mt-0.5 shrink-0" />
					<span class="min-w-0 flex-1">Couldn't load the scratchpad: {loadError}</span>
					<button type="button" class="shrink-0 font-semibold underline-offset-2 hover:underline" onclick={() => void load().then(() => showText(text))}>Retry</button>
				</div>
			{/if}
			{#if overLimit}
				<div class="notes-inline-error mt-3" role="alert" transition:fade={{ duration: 150 }}>
					<CircleAlert size={14} class="mt-0.5 shrink-0" />
					<span class="min-w-0 flex-1">{JSON_TOO_LARGE}, so changes aren't saved. Shorten the text, or switch to Markdown, which has no limit.</span>
					<button type="button" class="shrink-0 font-semibold underline-offset-2 hover:underline" onclick={() => void setKind('markdown')}>Switch to Markdown</button>
				</div>
			{:else if saveError}
				<div class="notes-inline-error mt-3" role="alert" transition:fade={{ duration: 150 }}>
					<CircleAlert size={14} class="mt-0.5 shrink-0" />
					<span class="min-w-0 flex-1">Couldn't save: {saveError}. Your text is kept and saving retries as you type.</span>
					<button type="button" class="shrink-0 font-semibold underline-offset-2 hover:underline" onclick={() => void queue.flush()}>Retry</button>
				</div>
			{/if}
			{#if formatError}
				{@render dismissible(formatError, () => (formatError = ''))}
			{/if}
			{#if actionError}
				{@render dismissible(actionError, () => (actionError = ''))}
			{/if}
			{#if viewError}
				{@render dismissible(viewError, () => (viewError = ''))}
			{/if}
		</header>

		{#if readView}
			{#if renderError}
				<div class="px-5 pt-4">
					<div class="notes-inline-error" role="alert">
						<CircleAlert size={14} class="mt-0.5 shrink-0" />
						<span class="min-w-0 flex-1">Couldn't show the read view: {renderError}</span>
					</div>
				</div>
			{:else if rendered !== null}
				<!-- Internal links (wikilinks, tags, chips) render but are inert here. -->
				<NoteReader bind:this={reader} html={rendered} onerror={(m) => (viewError = m)} onfollow={() => {}} class="min-h-0 flex-1 px-5" />
			{/if}
		{/if}
		<!-- Hidden, not unmounted, while reading: it keeps the undo history. Following links is off. -->
		<NoteEditor
			bind:this={editor}
			onchange={onBodyChange}
			onfollow={() => {}}
			onformat={(error) => (formatError = error ?? '')}
			class="scratchpad-editor {readView || loadError ? 'hidden' : 'min-h-0 flex-1'}"
		/>

		<footer class="scratchpad-footer">
			{#if blank && !readView}
				<span class="min-w-0 truncate">Kept until you clear it. Not in search.</span>
			{:else if kind === 'json'}
				<span><kbd class="palette-kbd">⇧⌥F</kbd> format</span>
			{:else}
				<span><kbd class="palette-kbd">⌘E</kbd> {reading ? 'edit' : 'read'}</span>
			{/if}
			<span class="ml-auto shrink-0 whitespace-nowrap">
				{#if palette.shortcut !== SCRATCHPAD_SHORTCUT}<kbd class="palette-kbd">⌘J</kbd> toggle ·{/if}
				<kbd class="palette-kbd">Esc</kbd> close
			</span>
		</footer>
	</aside>
{/if}
