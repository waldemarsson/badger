<script lang="ts">
	// The JS playground: saved scripts (list | editor above console), run in a QuickJS sandbox
	// in Rust. Bodies autosave like notes; Run executes the editor's text, unsaved edits
	// included. Run state and output live in playground-state (they outlive this page).
	import { invoke } from '@tauri-apps/api/core';
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { onMount, tick, untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { NoteSaveQueue } from '#lib/note-save-queue.js';
	import { registerFlushHandler } from '#lib/flush-guard.js';
	import { formatNoteDate } from '#lib/notes.js';
	import { JsEditor, loadJavaScript } from '#lib/editor/js-editor.js';
	import { clearOutput, consoleEntries, errorLocation, filterScripts, forgetScript, neighbourAfterRemoval, runSummary, upsertScript } from '#lib/playground.js';
	import { playground, runScript, stopRun, syncRunning } from '#lib/playground-state.svelte.js';
	import { palette } from '#lib/palette-state.svelte.js';
	import { scratchpad } from '#lib/scratchpad-state.svelte.js';
	import type { NoteSummary, Script, ScriptSummary } from '#lib/types.js';
	import Check from '@lucide/svelte/icons/check';
	import CircleAlert from '@lucide/svelte/icons/circle-exclamation-point';
	import Eraser from '@lucide/svelte/icons/eraser';
	import Play from '@lucide/svelte/icons/play';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import Square from '@lucide/svelte/icons/square';
	import SquareTerminal from '@lucide/svelte/icons/square-terminal';
	import Trash2 from '@lucide/svelte/icons/trash';
	import X from '@lucide/svelte/icons/x';

	const NOT_FOUND = 'Script not found';

	let scripts = $state<ScriptSummary[]>([]);
	let loaded = $state(false);
	let loadError = $state('');
	let query = $state('');

	let selectedId = $state<number | null>(null);
	let current = $state<Script | null>(null);
	let selectSeq = 0;

	let titleDraft = $state('');
	let titleError = $state('');
	let lastFailedTitle: string | null = null; // skip blur-retry of a title just rejected

	let confirmingDelete = $state(false);
	let actionError = $state('');
	let navError = $state('');
	let editorError = $state('');

	let editor = $state.raw<JsEditor>();
	let editorHost = $state<HTMLDivElement>();
	let titleInput = $state<HTMLInputElement>();
	let searchInput = $state<HTMLInputElement>();
	let consoleBody = $state<HTMLElement>();
	let layout = $state<HTMLElement>();
	let paneHeight = $state(560);

	/** Note titles for `note("…")` completion; refreshed on mount and when the window regains focus. */
	let noteTitles: string[] = [];

	const visible = $derived(filterScripts(scripts, query));
	const running = $derived(playground.running);
	const runningHere = $derived(running !== null && selectedId !== null && running.scriptId === selectedId);
	const output = $derived(selectedId === null ? undefined : playground.outputs[selectedId]);
	const entries = $derived(consoleEntries(output));
	const runningTitle = $derived(running?.scriptId != null ? (scripts.find((s) => s.id === running.scriptId)?.title ?? null) : null);

	// "Running… 2.4 s" while a run is in progress.
	let now = $state(Date.now());
	$effect(() => {
		if (!running) return;
		now = Date.now();
		const timer = setInterval(() => (now = Date.now()), 100);
		return () => clearInterval(timer);
	});
	const elapsed = $derived(running ? Math.max(0, now - running.startedAt) / 1000 : 0);

	// ── Autosave ──────────────────────────────────────────────────────────────────

	let queueVersion = $state(0);
	const queue = new NoteSaveQueue<ScriptSummary>({
		save: (id, body) => invoke<ScriptSummary>('update_script_body', { id, body }),
		onSaved: (_id, summary) => applySummary(summary),
		onChange: () => queueVersion++,
		// A deleted script stops its retries.
		isGone: (e) => String(e) === NOT_FOUND
	});
	const saveStatus = $derived.by(() => {
		void queueVersion;
		return selectedId === null ? 'saved' : queue.status(selectedId);
	});
	const saveError = $derived.by(() => {
		void queueVersion;
		return selectedId === null ? null : queue.error(selectedId);
	});

	function onBodyChange(id: number, body: string): void {
		navError = '';
		queue.edit(id, body);
	}

	function applySummary(summary: ScriptSummary): void {
		scripts = upsertScript(scripts, summary);
		if (current?.id === summary.id) {
			current.title = summary.title;
			current.updated_at = summary.updated_at;
		}
	}

	// ── Loading ───────────────────────────────────────────────────────────────────

	async function load(): Promise<void> {
		try {
			scripts = await invoke<ScriptSummary[]>('list_scripts');
			loadError = '';
		} catch (e) {
			loadError = String(e);
		} finally {
			loaded = true;
		}
	}

	async function loadNoteTitles(): Promise<void> {
		try {
			noteTitles = (await invoke<NoteSummary[]>('list_notes')).map((n) => n.title);
		} catch {
			// Completion just offers fewer titles.
		}
	}

	// ── Selection (lives in ?id=) ─────────────────────────────────────────────────

	function setUrl(id: number | null): void {
		const url = new URL(window.location.href);
		url.searchParams.delete('new');
		if (id === null) url.searchParams.delete('id');
		else url.searchParams.set('id', String(id));
		if (url.href === window.location.href) return;
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- the current URL with another ?id=
		void goto(url, { replace: true, shallow: true }).catch(() => {});
	}

	function show(script: Script): void {
		queue.track(script.id, script.body);
		selectedId = script.id;
		current = script;
		titleDraft = script.title;
		titleError = '';
		lastFailedTitle = null;
		confirmingDelete = false;
		editor?.open(script.id, script.body);
		setUrl(script.id);
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

	/** Saves the title and pending edits, then opens `id` (null: nothing). */
	async function select(id: number | null): Promise<void> {
		if (id === selectedId) return;
		const seq = ++selectSeq;
		await commitTitle(true); // an invalid title is discarded by the switch
		if (seq !== selectSeq) return;
		await queue.flush(); // failures stay dirty and keep retrying
		if (seq !== selectSeq) return;
		if (id === null) return showNothing();
		try {
			const script = await invoke<Script>('get_script', { id });
			if (seq !== selectSeq) return;
			show(script);
		} catch (e) {
			if (seq !== selectSeq) return;
			if (String(e) === NOT_FOUND) {
				scripts = scripts.filter((s) => s.id !== id);
				showNothing();
			} else {
				actionError = String(e);
			}
		}
	}

	function rowElements(): HTMLElement[] {
		return [...document.querySelectorAll<HTMLElement>('[data-script-row]')];
	}

	function onRowKeydown(e: KeyboardEvent, id: number): void {
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			void select(id);
		} else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			const rows = rowElements();
			const index = rows.indexOf(e.currentTarget as HTMLElement);
			const next = e.key === 'ArrowDown' ? rows[index + 1] : rows[index - 1];
			(next ?? (e.key === 'ArrowUp' ? searchInput : undefined))?.focus();
		}
	}

	function onSearchKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			query = '';
		} else if (e.key === 'ArrowDown') {
			e.preventDefault();
			rowElements()[0]?.focus();
		} else if (e.key === 'Enter' && visible.length) {
			e.preventDefault();
			void select(visible[0].id);
		}
	}

	// ?id= from outside (deep links) while mounted.
	let seenUrlId: string | null | undefined;
	$effect(() => {
		const raw = page.url.searchParams.get('id');
		if (raw === seenUrlId) return;
		seenUrlId = raw;
		const id = Number(raw);
		if (raw === null || !Number.isInteger(id) || id <= 0) return;
		untrack(() => void select(id));
	});

	// ── Create ────────────────────────────────────────────────────────────────────

	// ⌘K "New script" navigates to ?new=1; create here so the title gets focus.
	$effect(() => {
		if (page.url.searchParams.get('new') !== null) untrack(() => requestCreate());
	});

	let createChain: Promise<void> = Promise.resolve();

	function requestCreate(): void {
		createChain = createChain.then(createScript);
	}

	async function createScript(): Promise<void> {
		actionError = '';
		try {
			const seq = ++selectSeq;
			await commitTitle(true);
			await queue.flush();
			const script = await invoke<Script>('create_script');
			scripts = upsertScript(scripts, script);
			query = '';
			if (seq === selectSeq) show(script);
			await tick();
			titleInput?.focus();
			titleInput?.select();
		} catch (e) {
			actionError = String(e);
		}
	}

	function onWindowKeydown(e: KeyboardEvent): void {
		if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || e.code !== 'KeyN') return;
		e.preventDefault();
		if (!e.repeat) requestCreate();
	}

	// ── Title ─────────────────────────────────────────────────────────────────────

	/** Renames the open script to the draft; true when the stored title now matches it. */
	async function commitTitle(fromBlur = false): Promise<boolean> {
		if (!current) return true;
		const id = current.id;
		const title = titleDraft.trim();
		if (title === current.title) {
			titleDraft = current.title;
			titleError = '';
			return true;
		}
		if (fromBlur && title === lastFailedTitle) return false;
		try {
			const summary = await invoke<ScriptSummary>('update_script_title', { id, title });
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

	// ── Run / Stop ────────────────────────────────────────────────────────────────

	function run(): void {
		if (!current || !editor || playground.running) return;
		void queue.flush(); // not needed for the run (it takes the editor's text), just timely
		runScript(current.id, editor.text());
	}

	/** Esc stops a run only when no popup (⌘K, the scratchpad) has the keys. */
	function escapeStops(): boolean {
		if (!playground.running || palette.open || scratchpad.open) return false;
		void stopRun();
		return true;
	}

	function onConsoleKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape' && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && escapeStops()) {
			e.preventDefault();
		} else if (e.key === 'Enter' && e.metaKey) {
			e.preventDefault();
			run();
		}
	}

	// New output: show its end (the value and error are last).
	$effect(() => {
		void entries;
		void tick().then(() => {
			if (consoleBody) consoleBody.scrollTop = consoleBody.scrollHeight;
		});
	});

	// ── Delete ────────────────────────────────────────────────────────────────────

	async function deleteCurrent(): Promise<void> {
		if (!current) return;
		const id = current.id;
		const next = neighbourAfterRemoval(
			visible.map((s) => s.id),
			id
		);
		confirmingDelete = false;
		actionError = '';
		queue.pause(); // no autosave races the delete; the body stays dirty in case it fails
		try {
			await invoke('delete_script', { id });
		} catch (e) {
			if (String(e) !== NOT_FOUND) {
				actionError = String(e);
				void queue.flush();
				return;
			}
		}
		queue.forget(id);
		editor?.forget(id);
		forgetScript(playground, id); // a run it started keeps going; its output is dropped
		scripts = scripts.filter((s) => s.id !== id);
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
		if (leaving || nav.to?.route.id === '/playground' || !queue.busy) return;
		if (nav.type === 'leave') {
			void queue.flush(); // the quit/close guard covers app exit
			return;
		}
		nav.cancel();
		const target = nav.to?.url;
		void queue.flush().then((ok) => {
			if (!ok) {
				navError = "Your script couldn't be saved, so you're still here. Fix the problem and try again.";
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

	function onWindowFocus(): void {
		void loadNoteTitles();
	}

	// ── Layout ────────────────────────────────────────────────────────────────────

	function measure(): void {
		if (!layout) return;
		const top = layout.getBoundingClientRect().top + window.scrollY;
		paneHeight = Math.max(420, window.innerHeight - top);
	}

	$effect(() => {
		void loadError;
		measure();
	});

	onMount(() => {
		void load();
		void loadNoteTitles();
		void syncRunning();
		let destroyed = false;
		loadJavaScript()
			.then((js) => {
				if (destroyed || !editorHost) return;
				editor = new JsEditor(editorHost, js, {
					onchange: onBodyChange,
					onrun: run,
					onescape: escapeStops,
					noteTitles: () => noteTitles
				});
				if (current) editor.open(current.id, current.body);
			})
			.catch((e) => (editorError = `The editor couldn't load: ${String(e)}`));
		const unregister = registerFlushHandler(async () => {
			const [, saved] = await Promise.all([commitTitle(true), queue.flush()]);
			return saved;
		});
		return () => {
			destroyed = true;
			unregister();
			void queue.flush();
			editor?.destroy();
			editor = undefined;
		};
	});
</script>

<svelte:window onkeydown={onWindowKeydown} onblur={() => void queue.flush()} onfocus={onWindowFocus} onresize={measure} />
<svelte:document onvisibilitychange={flushOnHide} />

{#snippet dismissible(message: string, onDismiss: () => void)}
	<div class="notes-inline-error" role="alert" transition:fade={{ duration: 150 }}>
		<CircleAlert size={14} class="mt-0.5 shrink-0" />
		<span class="min-w-0 flex-1">{message}</span>
		<button type="button" class="shrink-0" onclick={onDismiss} title="Dismiss" aria-label="Dismiss"><X size={14} /></button>
	</div>
{/snippet}

<h1 class="sr-only">Playground</h1>

<div class="page-shell">
	<div class="page-view">
		{#if loadError}
			<div class="alert-panel alert-error">
				<p class="alert-error-title">Could not load scripts</p>
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
							placeholder="Filter scripts"
							aria-label="Filter scripts"
							autocomplete="off"
							spellcheck="false"
							onkeydown={onSearchKeydown}
						/>
						{#if query}
							<button
								type="button"
								class="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-md p-1 text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600 dark:text-neutral-500 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
								title="Clear filter"
								aria-label="Clear filter"
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
					<button type="button" class="btn-icon-secondary" title="New script (⌘N)" aria-label="New script" onclick={requestCreate}>
						<Plus size={16} />
					</button>
				</div>

				{#if actionError}
					<div class="px-3 pt-3">{@render dismissible(actionError, () => (actionError = ''))}</div>
				{/if}

				<div class="min-h-0 flex-1 overflow-y-auto p-2">
					{#if !loaded && !loadError}
						<p class="px-3 py-6 text-center text-sm text-slate-400 dark:text-neutral-500">Loading…</p>
					{:else if scripts.length === 0}
						<div class="notes-empty" in:fade={{ duration: 150 }}>
							<SquareTerminal size={22} class="text-slate-300 dark:text-neutral-600" />
							<p class="font-medium text-slate-600 dark:text-neutral-300">No scripts yet</p>
							<p class="text-xs">Create one to start.</p>
						</div>
					{:else if visible.length === 0}
						<div class="notes-empty" in:fade={{ duration: 150 }}>
							<Search size={20} class="text-slate-300 dark:text-neutral-600" />
							<p class="font-medium text-slate-600 dark:text-neutral-300">No matches</p>
							<p class="text-xs">Nothing matches “{query.trim()}”.</p>
						</div>
					{:else}
						<div role="list" aria-label="Scripts">
							{#each visible as s (s.id)}
								<div role="listitem" animate:flip={{ duration: 180 }}>
									<div
										role="button"
										tabindex="0"
										class="note-row"
										class:note-row-active={s.id === selectedId}
										aria-current={s.id === selectedId ? 'true' : undefined}
										data-script-row
										onclick={() => void select(s.id)}
										onkeydown={(e) => onRowKeydown(e, s.id)}
									>
										<p class="note-row-title">{s.title}</p>
										<p class="script-row-time">
											{#if running?.scriptId === s.id}
												<span class="script-running-dot"></span><span class="text-sky-600 dark:text-sky-400">Running…</span>
											{:else}
												{formatNoteDate(s.updated_at, new Date())}
											{/if}
										</p>
									</div>
								</div>
							{/each}
						</div>
					{/if}
				</div>
			</aside>

			<!-- Editor and console -->
			<section class="notes-pane">
				{#if current}
					<header class="flex min-h-14 shrink-0 flex-col justify-center border-b border-slate-100 px-6 dark:border-neutral-800">
						<div class="flex items-center gap-3">
							<input
								bind:this={titleInput}
								bind:value={titleDraft}
								class="note-title-input"
								class:note-title-input-error={titleError}
								placeholder="Untitled"
								aria-label="Script title"
								aria-invalid={titleError ? 'true' : undefined}
								autocomplete="off"
								spellcheck="false"
								onkeydown={onTitleKeydown}
								onblur={() => void commitTitle(true)}
							/>
							<span class="note-save-state" aria-live="polite">
								{#if saveStatus === 'error'}
									<span class="text-red-600 dark:text-red-400" title={saveError ?? undefined}>Not saved</span>
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
									<span class="text-xs font-medium text-red-700 dark:text-red-300">Delete this script?</span>
									<button type="button" class="btn-danger-sm" use:autofocus onclick={deleteCurrent}>Delete</button>
									<button type="button" class="btn-secondary-sm" onclick={() => (confirmingDelete = false)}>Cancel</button>
								</div>
							{:else}
								<div class="flex shrink-0 items-center gap-1.5">
									{#if running}
										<button type="button" class="playground-stop" title="Stop the running script (Esc)" onclick={() => void stopRun()} in:fade={{ duration: 150 }}>
											<Square size={12} class="fill-current" /> Stop <kbd class="playground-kbd">Esc</kbd>
										</button>
									{:else}
										<button type="button" class="playground-run" title="Run (⌘↵)" onclick={run} disabled={!editor && !!editorError} in:fade={{ duration: 150 }}>
											<Play size={12} class="fill-current" /> Run <kbd class="playground-kbd">⌘↵</kbd>
										</button>
									{/if}
									<button type="button" class="btn-icon-ghost-danger" title="Delete script" aria-label="Delete script" onclick={() => (confirmingDelete = true)}>
										<Trash2 size={16} />
									</button>
								</div>
							{/if}
						</div>
						{#if titleError}
							<p class="form-error -mt-1.5 pb-2" role="alert" transition:fade={{ duration: 150 }}>{titleError}</p>
						{/if}
						{#if navError}
							<div class="pb-3">{@render dismissible(navError, () => (navError = ''))}</div>
						{/if}
						{#if saveStatus === 'error' && saveError}
							<div class="pb-3">{@render dismissible(`Couldn't save this script: ${saveError}`, () => void queue.flush())}</div>
						{/if}
					</header>
				{:else}
					<div class="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center" in:fade={{ duration: 150 }}>
						<div class="rounded-2xl bg-slate-100 p-4 text-slate-400 ring-1 ring-slate-200 dark:bg-neutral-800 dark:text-neutral-500 dark:ring-neutral-700">
							<SquareTerminal size={28} />
						</div>
						<div>
							<p class="text-base font-semibold text-slate-800 dark:text-neutral-200">
								{scripts.length ? 'Select a script' : 'Write your first script'}
							</p>
							<p class="mt-1 max-w-sm text-sm text-slate-500 dark:text-neutral-400">
								JavaScript in a sandbox: no network or files. <code class="font-mono text-[0.85em]">note("Title")</code> reads a note.
							</p>
						</div>
						<button type="button" class="btn-primary gap-2" onclick={requestCreate}>
							<Plus size={16} /> New script <kbd class="notes-btn-kbd">⌘N</kbd>
						</button>
					</div>
				{/if}

				{#if editorError && current}
					<div class="px-6 pt-4">{@render dismissible(editorError, () => (editorError = ''))}</div>
				{/if}
				<!-- Hidden, not unmounted, without a script: it keeps each script's undo history. -->
				<div bind:this={editorHost} class={current ? 'min-h-0 flex-1 overflow-hidden' : 'hidden'}></div>

				{#if current}
					<div class="playground-console">
						<div class="playground-console-bar">
							<span class="font-semibold uppercase tracking-wide text-slate-400 dark:text-neutral-500">Console</span>
							{#if runningHere}
								<span class="inline-flex items-center gap-1.5 text-sky-600 dark:text-sky-400" in:fade={{ duration: 150 }}>
									<span class="script-running-dot"></span> Running… {elapsed.toFixed(1)} s
								</span>
							{:else if running}
								<span class="inline-flex min-w-0 items-center gap-1.5 truncate" in:fade={{ duration: 150 }}>
									<span class="script-running-dot"></span>
									<span class="truncate">{runningTitle ? `Running “${runningTitle}”…` : 'A script is running…'}</span>
								</span>
							{:else if output?.outcome}
								<span in:fade={{ duration: 150 }}>{runSummary(output.outcome)}</span>
							{/if}
							{#if output && !runningHere}
								<button
									type="button"
									class="btn-icon-ghost ml-auto"
									title="Clear output"
									aria-label="Clear output"
									onclick={() => current && clearOutput(playground, current.id)}
								>
									<Eraser size={14} />
								</button>
							{/if}
						</div>
						<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
						<div bind:this={consoleBody} class="playground-console-body" tabindex="0" role="log" aria-label="Console output" onkeydown={onConsoleKeydown}>
							{#if entries.length === 0}
								<div class="console-empty">
									{#if runningHere}
										<p>Waiting for output…</p>
									{:else if output}
										<p>No output</p>
									{:else}
										<p>Run the script (<kbd class="font-sans">⌘↵</kbd>) to see its output here.</p>
										<p>The last expression's value is shown too.</p>
									{/if}
								</div>
							{:else}
								{#each entries as entry, i (i)}
									{#if entry.kind === 'line'}
										<div
											class="console-line"
											class:console-line-info={entry.level === 'info'}
											class:console-line-warn={entry.level === 'warn'}
											class:console-line-error={entry.level === 'error'}
											class:console-line-debug={entry.level === 'debug'}
										>{entry.text === '' ? ' ' : entry.text}</div>
									{:else if entry.kind === 'truncated'}
										<div class="console-truncated">Output truncated: later console output was dropped</div>
									{:else if entry.kind === 'value'}
										<div class="console-value"><span class="console-value-mark" aria-hidden="true">←</span><span class="min-w-0">{entry.text}</span></div>
									{:else}
										{@const location = errorLocation(entry)}
										<div
											class="console-error"
											class:console-error-timeout={entry.errorKind === 'timeout'}
											class:console-error-stopped={entry.errorKind === 'stopped'}
											role="alert"
										>
											<div class="console-error-head">
												<span>{entry.title}</span>
												{#if location && entry.line !== null}
													{@const line = entry.line}
													{@const col = entry.col}
													<button type="button" class="console-error-location" title="Show in the editor" onclick={() => editor?.goTo(line, col)}>{location}</button>
												{/if}
											</div>
											{#if entry.message !== entry.title}<div class="console-error-message">{entry.message}</div>{/if}
										</div>
									{/if}
								{/each}
							{/if}
						</div>
					</div>
				{/if}
			</section>
		</div>
	</div>
</div>
