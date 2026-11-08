<script lang="ts" generics="Href extends string">
	import { invoke } from '@tauri-apps/api/core';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount, tick, untrack } from 'svelte';
	import { fade, scale } from 'svelte/transition';
	import Search from '@lucide/svelte/icons/search';
	import ListTodo from '@lucide/svelte/icons/list-todo';
	import Clock from '@lucide/svelte/icons/clock';
	import ArrowRight from '@lucide/svelte/icons/arrow-right';
	import Plus from '@lucide/svelte/icons/plus';
	import Calendar from '@lucide/svelte/icons/calendar';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import NotebookPen from '@lucide/svelte/icons/notebook-pen';
	import FileText from '@lucide/svelte/icons/file-text';
	import Sparkles from '@lucide/svelte/icons/sparkles';
	import { today } from '#lib/date-ranges.js';
	import {
		filterItems,
		initialIndex,
		isValidDate,
		matchesShortcut,
		normalizeShortcut,
		parseHours,
		shiftDate
	} from '#lib/palette.js';
	import { changes, logTimeRequest, palette, showToast, type LogTimePrefill } from '#lib/palette-state.svelte.js';
	import { splitHighlights } from '#lib/notes.js';
	import { splitSearchQuery, tagTerm } from '#lib/note-links.js';
	import { HYBRID_DEBOUNCE_MS, keepActiveIndex, qualifiesForSemantic, semanticReady, shouldApply, type SearchResponse } from '#lib/semantic.js';
	import { semantic } from '#lib/semantic-state.svelte.js';
	import type { Customer, Issue, NoteHit, PaletteSettings, Project, TimeEntryRow } from '#lib/types.js';

	let {
		pages,
		navigate
	}: {
		pages: readonly { href: Href; label: string }[];
		navigate: (href: Href) => void;
	} = $props();

	type Step = 'root' | 'todo' | 'project' | 'issue' | 'hours' | 'notes';
	type Command = { id: string; label: string; group: 'Actions' | 'Go to'; run: () => void };
	type ProjectOption = { id: number; label: string; isDefault: boolean };
	type IssueOption = { id: number | null; title: string };

	let open = $state(false);
	let step = $state<Step>('root');
	let texts = $state<Record<Step, string>>(emptyTexts());
	let highlight = $state(0);
	let error = $state('');
	let busy = $state(false);
	let loading = $state(false);

	let projects = $state<ProjectOption[]>([]);
	let issues = $state<Issue[]>([]);
	let lastUsed = $state<{ projectId: number | null; issueId: number | null }>({ projectId: null, issueId: null });

	let project = $state<ProjectOption | null>(null);
	let issue = $state<IssueOption | null>(null);
	let date = $state(today());
	let dateText = $state(today());

	let input = $state<HTMLInputElement>();
	let dateInput = $state<HTMLInputElement>();
	let list = $state<HTMLElement>();
	let returnFocus: HTMLElement | null = null;

	function emptyTexts(): Record<Step, string> {
		return { root: '', todo: '', project: '', issue: '', hours: '', notes: '' };
	}

	// ── Items per step ─────────────────────────────────────────────────────────────

	const commands: Command[] = $derived([
		{ id: 'new-todo', label: 'New todo', group: 'Actions', run: () => goTo('todo') },
		{ id: 'log-time', label: 'Log time', group: 'Actions', run: () => void startLogTime() },
		{ id: 'new-note', label: 'New note', group: 'Actions', run: createNote },
		...pages.map((p): Command => ({
			id: `page:${p.href}`,
			label: p.label,
			group: 'Go to',
			run: () => {
				close();
				navigate(p.href);
			}
		}))
	]);

	const filteredCommands = $derived(filterItems(commands, texts.root, (c) => c.label));

	// Notes matching the root query, from the search index; listed after the commands.
	const NOTE_HIT_LIMIT = 5;
	// Shorter queries are for commands ("ne" → New note), and would scan every note. A `#tag`
	// term (search_notes filters by tag) searches at any length: commands never start with `#`.
	const MIN_NOTE_QUERY_CHARS = 3;
	let noteResults = $state<{ term: string; hits: NoteHit[] }>({ term: '', hits: [] });
	let noteSeq = 0;
	const noteTerm = $derived.by(() => {
		const term = open && step === 'root' ? texts.root.trim() : '';
		return [...term].length >= MIN_NOTE_QUERY_CHARS || term.split(/\s+/).some((t) => tagTerm(t) !== null)
			? term
			: '';
	});
	// Only hits for the current text: stale ones could be picked with Enter.
	const noteHits = $derived(noteResults.term === noteTerm ? noteResults.hits : []);
	const notesPending = $derived(noteTerm !== '' && noteResults.term !== noteTerm);
	let noteShown: SearchResponse | null = null;
	const hybridNotes = $derived(semanticReady(semantic.status));
	// Keyword hits at once; hybrid hits (with related notes) after a pause in typing.
	$effect(() => {
		const term = noteTerm;
		const hybrid = hybridNotes;
		const seq = ++noteSeq;
		if (!term) return;
		const apply = (hits: NoteHit[], response: SearchResponse) => {
			if (!shouldApply(noteSeq, noteShown, response)) return;
			// Keep the active row on the same item when the list changes under it.
			const commandIds = filteredCommands.map((c) => c.id);
			const before = [...commandIds, ...noteHits.map((h) => `note:${h.id}`)];
			const after = [...commandIds, ...hits.map((h) => `note:${h.id}`)];
			highlight = keepActiveIndex(before, after, highlight);
			noteShown = response;
			noteResults = { term, hits };
		};
		invoke<NoteHit[]>('search_notes', { query: term, limit: NOTE_HIT_LIMIT })
			.then((hits) => apply(hits, { seq, hybrid: false }))
			.catch(() => {
				// commands still work; the notes page shows search errors
				if (seq === noteSeq) noteResults = { term, hits: [] };
			});
		if (!hybrid || !qualifiesForSemantic(splitSearchQuery(term).text)) return;
		const timer = setTimeout(() => {
			invoke<NoteHit[]>('search_notes', { query: term, limit: NOTE_HIT_LIMIT, hybrid: true })
				.then((hits) => apply(hits, { seq, hybrid: true }))
				.catch(() => {}); // keep the keyword hits
		}, HYBRID_DEBOUNCE_MS);
		return () => clearTimeout(timer);
	});
	const filteredProjects = $derived(filterItems(projects, texts.project, (p) => p.label));
	const issueOptions: IssueOption[] = $derived.by(() => {
		const typed = texts.issue.trim();
		const matches: IssueOption[] = filterItems(issues, typed, (i) => i.title).map((i) => ({ id: i.id, title: i.title }));
		if (typed && !issues.some((i) => i.title === typed)) matches.push({ id: null, title: typed });
		return matches;
	});

	const listLength = $derived(
		step === 'root' ? filteredCommands.length + noteHits.length
		: step === 'project' ? filteredProjects.length
		: step === 'issue' ? issueOptions.length
		: 0
	);

	const placeholder = $derived(
		{
			root: 'Type a command…',
			todo: 'Todo title…',
			project: 'Search projects…',
			issue: 'Search or create an issue…',
			hours: 'Hours, e.g. 1.5',
			notes: 'Notes (optional) — Enter to save'
		}[step]
	);

	const StepIcon = $derived(step === 'todo' ? ListTodo : step === 'hours' || step === 'notes' ? Clock : Search);

	const dateLabel = $derived(
		date === today() ? 'Today' : date === shiftDate(today(), -1) ? 'Yesterday' : date
	);

	// ── Open / close ───────────────────────────────────────────────────────────────

	async function openPalette(): Promise<void> {
		returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		step = 'root';
		texts = emptyTexts();
		highlight = 0;
		error = '';
		busy = false;
		project = null;
		issue = null;
		date = today();
		dateText = date;
		open = true;
		await tick();
		input?.focus();
	}

	function close(): void {
		open = false;
		returnFocus?.focus();
		returnFocus = null;
	}

	async function goTo(next: Step, index = 0): Promise<void> {
		step = next;
		highlight = index;
		error = '';
		await tick();
		input?.focus();
	}

	function back(): void {
		const previous: Record<Step, Step | null> = {
			root: null,
			todo: 'root',
			project: 'root',
			issue: 'project',
			hours: 'issue',
			notes: 'hours'
		};
		const target = previous[step];
		if (!target) return;
		const index =
			target === 'root' ? Math.max(0, filteredCommands.findIndex((c) => c.id === (step === 'todo' ? 'new-todo' : 'log-time')))
			: target === 'project' ? Math.max(0, filteredProjects.findIndex((p) => p.id === project?.id))
			: target === 'issue' ? Math.max(0, issueOptions.findIndex((i) => i.title === issue?.title))
			: 0;
		void goTo(target, index);
	}

	// ── Log time flow ──────────────────────────────────────────────────────────────

	/** With a prefill whose project still exists, continues straight to the hours step. */
	async function startLogTime(prefill: LogTimePrefill | null = null): Promise<void> {
		loading = true;
		texts.project = '';
		await goTo('project');
		try {
			const [settings, customers] = await Promise.all([
				invoke<PaletteSettings>('get_palette_settings'),
				invoke<Customer[]>('list_customers')
			]);
			lastUsed = { projectId: settings.last_project_id, issueId: settings.last_issue_id };
			const perCustomer = await Promise.all(
				customers.map(async (c) => ({ c, ps: await invoke<Project[]>('list_projects', { customerId: c.id }) }))
			);
			projects = perCustomer.flatMap(({ c, ps }) =>
				ps.map((p) => ({ id: p.id, label: `${c.name} / ${p.name}`, isDefault: c.use_as_default && p.use_as_default }))
			);
			const fallback = projects.find((p) => p.isDefault)?.id;
			highlight = initialIndex(projects, prefill?.projectId ?? lastUsed.projectId, fallback);
			const match = prefill && projects.find((p) => p.id === prefill.projectId);
			if (prefill && match) {
				loading = false;
				await pickProject(match, { id: prefill.issueId, title: prefill.issueTitle });
			}
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	/** A `preset` issue skips the issue step; issues still load so Back lands on a full list. */
	async function pickProject(p: ProjectOption, preset: IssueOption | null = null): Promise<void> {
		if (project?.id !== p.id || preset) texts.issue = '';
		project = p;
		loading = true;
		issues = [];
		if (preset) issue = preset;
		await goTo(preset ? 'hours' : 'issue');
		try {
			issues = await invoke<Issue[]>('list_issues_by_recent', { projectId: p.id });
			if (!preset && !texts.issue) {
				highlight = p.id === lastUsed.projectId ? initialIndex(issues, lastUsed.issueId) : 0;
			}
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	function pickIssue(i: IssueOption): void {
		issue = i;
		void goTo('hours');
	}

	function hoursToNotes(): void {
		const parsed = parseHours(texts.hours);
		if ('error' in parsed) {
			error = parsed.error;
			return;
		}
		if (!commitDate()) return;
		void goTo('notes');
	}

	async function submitTimeEntry(): Promise<void> {
		const parsed = parseHours(texts.hours);
		if (busy || !project || !issue || 'error' in parsed) return;
		busy = true;
		error = '';
		try {
			const row = await invoke<TimeEntryRow>('create_time_entry', {
				projectId: project.id,
				issueIdentifier: issue.title,
				date,
				hours: parsed.hours,
				notes: texts.notes.trim() || null
			});
			// Remembering the selection is best-effort; the entry is already saved.
			await invoke('set_palette_last_used', { projectId: row.project_id, issueId: row.issue_id }).catch(() => {});
			changes.timeEntries += 1;
			showToast(`Logged ${parsed.hours}h on ${row.issue_title}`);
			close();
		} catch (e) {
			error = String(e);
		} finally {
			busy = false;
		}
	}

	/** Applies the typed date; invalid input keeps the previous date. */
	function commitDate(): boolean {
		if (dateText === date) return true;
		if (!isValidDate(dateText)) {
			error = 'Date must be YYYY-MM-DD';
			dateText = date;
			return false;
		}
		date = dateText;
		return true;
	}

	function shiftBy(days: number): void {
		date = shiftDate(isValidDate(dateText) ? dateText : date, days);
		dateText = date;
		error = '';
	}

	// ── New todo ───────────────────────────────────────────────────────────────────

	async function submitTodo(): Promise<void> {
		const title = texts.todo.trim();
		if (busy || !title) return;
		busy = true;
		error = '';
		try {
			await invoke('create_todo', { title, notes: null, dueDate: null, status: 'not_started', issueId: null });
			changes.todos += 1;
			showToast('Todo added');
			close();
		} catch (e) {
			error = String(e);
		} finally {
			busy = false;
		}
	}

	// ── New note ───────────────────────────────────────────────────────────────────

	/** The notes page creates the note on `?new=1`, so ⌘N and the palette share one path. */
	function createNote(): void {
		close();
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
		void goto(`${resolve('/notes')}?new=1`);
	}

	function openNote(id: number): void {
		close();
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
		void goto(`${resolve('/notes')}?id=${id}`);
	}

	// ── Keyboard ───────────────────────────────────────────────────────────────────

	function pick(index: number): void {
		if (step === 'root') {
			if (index < filteredCommands.length) filteredCommands[index].run();
			else if (noteHits[index - filteredCommands.length]) openNote(noteHits[index - filteredCommands.length].id);
		}
		else if (step === 'project' && filteredProjects[index]) void pickProject(filteredProjects[index]);
		else if (step === 'issue' && issueOptions[index]) pickIssue(issueOptions[index]);
	}

	function oninput(): void {
		error = '';
		if (step === 'issue') {
			const exact = issueOptions.findIndex((i) => i.id !== null && i.title === texts.issue.trim());
			highlight = Math.max(0, exact);
		} else {
			highlight = 0;
		}
	}

	function onInputKeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			if (listLength === 0) return;
			e.preventDefault();
			const delta = e.key === 'ArrowDown' ? 1 : -1;
			highlight = Math.min(listLength - 1, Math.max(0, highlight + delta));
		} else if (e.key === 'Enter') {
			e.preventDefault();
			if (step === 'todo') void submitTodo();
			else if (step === 'hours') hoursToNotes();
			else if (step === 'notes') void submitTimeEntry();
			else pick(highlight);
		} else if (e.key === 'Backspace' && texts[step] === '' && !busy) {
			e.preventDefault();
			back();
		} else if (e.key === 'Tab' && step !== 'hours') {
			e.preventDefault();
		}
	}

	function onDateKeydown(e: KeyboardEvent): void {
		if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
			e.preventDefault();
			shiftBy(e.key === 'ArrowUp' ? 1 : -1);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			hoursToNotes();
		} else if (e.key === 'Tab') {
			e.preventDefault();
			if (commitDate()) error = '';
			input?.focus();
		}
	}

	/** Capture phase so Esc closes only the palette, before page or modal handlers see it. */
	function onWindowKeydown(e: KeyboardEvent): void {
		if (palette.recording || e.repeat) return;
		if (matchesShortcut(e, normalizeShortcut(palette.shortcut))) {
			e.preventDefault();
			e.stopPropagation();
			if (open) close();
			else void openPalette();
		} else if (open && e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			close();
		}
	}

	// Pages (e.g. Today's [Log]) ask for the Log time flow through `requestLogTime`.
	let seenLogTimeRequest = logTimeRequest.id;
	$effect(() => {
		if (logTimeRequest.id === seenLogTimeRequest) return;
		seenLogTimeRequest = logTimeRequest.id;
		const prefill = untrack(() => (logTimeRequest.prefill ? { ...logTimeRequest.prefill } : null));
		void openPalette().then(() => startLogTime(prefill));
	});

	onMount(() => {
		window.addEventListener('keydown', onWindowKeydown, true);
		return () => window.removeEventListener('keydown', onWindowKeydown, true);
	});

	$effect(() => {
		void highlight;
		void step;
		list?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' });
	});
</script>

{#snippet row(index: number, label: string, icon: typeof Search | null, hint = '')}
	{@const active = index === highlight}
	<button
		type="button"
		tabindex="-1"
		data-active={active}
		class="flex w-full min-w-0 items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors {active
			? 'bg-sky-50 text-sky-900 dark:bg-sky-500/10 dark:text-sky-300'
			: 'text-slate-700 dark:text-neutral-300'}"
		onmousemove={() => (highlight = index)}
		onclick={() => pick(index)}
	>
		{#if icon}
			{@const Icon = icon}
			<Icon size={16} class="shrink-0 {active ? 'text-sky-500 dark:text-sky-400' : 'text-slate-400 dark:text-neutral-500'}" />
		{/if}
		<span class="min-w-0 flex-1 truncate">{label}</span>
		{#if hint}
			<span class="shrink-0 text-xs text-slate-400 dark:text-neutral-500">{hint}</span>
		{/if}
	</button>
{/snippet}

{#snippet marked(text: string)}
	{#each splitHighlights(text) as seg, i (i)}{#if seg.hit}<mark class="search-hit">{seg.text}</mark>{:else}{seg.text}{/if}{/each}
{/snippet}

{#snippet noteRow(index: number, hit: NoteHit)}
	{@const active = index === highlight}
	<button
		type="button"
		tabindex="-1"
		data-active={active}
		class="flex w-full min-w-0 items-start gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors {active
			? 'bg-sky-50 text-sky-900 dark:bg-sky-500/10 dark:text-sky-300'
			: 'text-slate-700 dark:text-neutral-300'}"
		onmousemove={() => (highlight = index)}
		onclick={() => pick(index)}
	>
		<FileText size={16} class="mt-0.5 shrink-0 {active ? 'text-sky-500 dark:text-sky-400' : 'text-slate-400 dark:text-neutral-500'}" />
		<span class="min-w-0 flex-1">
			<span class="flex min-w-0 items-center gap-2">
				<span class="min-w-0 truncate">{@render marked(hit.title_marked)}</span>
				{#if hit.related}
					<span class="note-related mt-0 shrink-0" title="Found by meaning: no exact word match"><Sparkles size={10} class="shrink-0" />Related</span>
				{/if}
			</span>
			{#if hit.snippet || hit.excerpt}
				<span class="block truncate text-xs text-slate-500 dark:text-neutral-400">
					{#if hit.snippet}{@render marked(hit.snippet)}{:else}{hit.excerpt}{/if}
				</span>
			{/if}
		</span>
	</button>
{/snippet}

{#snippet empty(message: string)}
	<p class="px-3 py-6 text-center text-sm text-slate-500 dark:text-neutral-400">{message}</p>
{/snippet}

{#if open}
	<div
		class="fixed inset-0 z-[60] flex items-start justify-center bg-slate-950/40 p-4 pt-[15vh] backdrop-blur-[2px] dark:bg-black/60"
		transition:fade={{ duration: 150 }}
		onmousedown={(e) => e.target === e.currentTarget && close()}
		role="presentation"
	>
		<div
			class="w-full max-w-xl overflow-hidden rounded-2xl bg-white shadow-2xl ring-1 ring-slate-200 dark:bg-neutral-900 dark:ring-neutral-800"
			transition:scale={{ start: 0.96, duration: 150 }}
			role="dialog"
			aria-modal="true"
			aria-label="Command palette"
		>
			{#if step !== 'root'}
				<div class="flex flex-wrap items-center gap-1 border-b border-slate-100 px-4 pb-2 pt-3 text-xs dark:border-neutral-800">
					<span class="palette-chip">{step === 'todo' ? 'New todo' : 'Log time'}</span>
					{#if project && step !== 'project'}
						<ChevronRight size={12} class="text-slate-300 dark:text-neutral-600" />
						<span class="palette-chip max-w-[14rem] truncate">{project.label}</span>
					{/if}
					{#if issue && (step === 'hours' || step === 'notes')}
						<ChevronRight size={12} class="text-slate-300 dark:text-neutral-600" />
						<span class="palette-chip max-w-[12rem] truncate">{issue.title}</span>
					{/if}
					{#if step === 'notes'}
						<ChevronRight size={12} class="text-slate-300 dark:text-neutral-600" />
						<span class="palette-chip">{texts.hours.trim()}h · {dateLabel}</span>
					{/if}
				</div>
			{/if}

			<div class="flex items-center gap-3 px-4 py-3">
				<StepIcon size={18} class="shrink-0 text-slate-400 dark:text-neutral-500" />
				<input
					bind:this={input}
					bind:value={texts[step]}
					{placeholder}
					{oninput}
					onkeydown={onInputKeydown}
					inputmode={step === 'hours' ? 'decimal' : 'text'}
					class="min-w-0 flex-1 border-0 bg-transparent p-0 text-base text-slate-900 placeholder:text-slate-400 focus:outline-none focus:ring-0 dark:text-neutral-100 dark:placeholder:text-neutral-500"
					aria-label={placeholder}
					autocomplete="off"
					spellcheck="false"
				/>
				{#if step === 'hours'}
					<label class="palette-chip flex shrink-0 items-center gap-1.5 focus-within:ring-2 focus-within:ring-sky-400">
						<Calendar size={12} />
						<input
							bind:this={dateInput}
							bind:value={dateText}
							onkeydown={onDateKeydown}
							onblur={() => commitDate()}
							class="w-[5rem] border-0 bg-transparent p-0 text-xs focus:outline-none focus:ring-0"
							aria-label="Date (↑/↓ to change by a day)"
							autocomplete="off"
						/>
						{#if dateLabel === 'Today' || dateLabel === 'Yesterday'}
							<span class="text-slate-400 dark:text-neutral-500">{dateLabel}</span>
						{/if}
					</label>
				{/if}
			</div>

			{#if error}
				<p class="form-error px-4 pb-2" role="alert">{error}</p>
			{/if}

			{#if step === 'root' || step === 'project' || step === 'issue'}
				<div bind:this={list} class="max-h-96 overflow-y-auto border-t border-slate-100 p-2 dark:border-neutral-800">
					{#if step === 'root'}
						{#if filteredCommands.length === 0 && noteHits.length === 0 && !notesPending}
							{@render empty('No matching commands or notes')}
						{/if}
						{#each ['Actions', 'Go to'] as const as group (group)}
							{@const items = filteredCommands.filter((c) => c.group === group)}
							{#if items.length}
								<p class="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-slate-400 dark:text-neutral-500">{group}</p>
								{#each items as c (c.id)}
									{@render row(
										filteredCommands.indexOf(c),
										c.label,
										c.id === 'new-todo' ? ListTodo
										: c.id === 'log-time' ? Clock
										: c.id === 'new-note' ? NotebookPen
										: ArrowRight
									)}
								{/each}
							{/if}
						{/each}
						{#if noteHits.length}
							<p class="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-slate-400 dark:text-neutral-500">Notes</p>
							{#each noteHits as hit, i (hit.id)}
								{@render noteRow(filteredCommands.length + i, hit)}
							{/each}
						{/if}
					{:else if step === 'project'}
						{#if loading}
							{@render empty('Loading…')}
						{:else if projects.length === 0}
							{@render empty('No projects — add one in Settings')}
						{:else if filteredProjects.length === 0}
							{@render empty('No matching projects')}
						{/if}
						{#each filteredProjects as p, i (p.id)}
							{@render row(i, p.label, null, p.id === lastUsed.projectId ? 'Last used' : '')}
						{/each}
					{:else}
						{#if loading}
							{@render empty('Loading…')}
						{:else if issueOptions.length === 0}
							{@render empty('No issues yet — type a title to create one')}
						{/if}
						{#each issueOptions as option, i (option.id ?? `new:${option.title}`)}
							{#if option.id === null}
								{@render row(i, `Create "${option.title}"`, Plus)}
							{:else}
								{@render row(i, option.title, null, project?.id === lastUsed.projectId && option.id === lastUsed.issueId ? 'Last used' : '')}
							{/if}
						{/each}
					{/if}
				</div>
			{/if}

			<div class="flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-slate-100 bg-slate-50 px-4 py-2 text-[11px] text-slate-500 dark:border-neutral-800 dark:bg-neutral-950/40 dark:text-neutral-400">
				{#if step === 'hours'}
					<span><kbd class="palette-kbd">⇥</kbd> date</span>
					<span><kbd class="palette-kbd">↩</kbd> next</span>
				{:else if step === 'todo' || step === 'notes'}
					<span><kbd class="palette-kbd">↩</kbd> save</span>
				{:else}
					<span><kbd class="palette-kbd">↑</kbd><kbd class="palette-kbd">↓</kbd> navigate</span>
					<span><kbd class="palette-kbd">↩</kbd> select</span>
				{/if}
				{#if step !== 'root'}
					<span><kbd class="palette-kbd">⌫</kbd> back</span>
				{/if}
				<span class="ml-auto"><kbd class="palette-kbd">esc</kbd> close</span>
			</div>
		</div>
	</div>
{/if}
