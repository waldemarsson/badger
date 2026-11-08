<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import { slide } from 'svelte/transition';
	import type { NoteSummary, Reminder, ReminderRepeat, TodoRow } from '#lib/types.js';
	import { QUICK_TIMES, formStart, quickTime, snoozeUntil, splitTimestamp, type QuickChoice } from '#lib/reminders.js';
	import { reminders, saveReminder } from '#lib/reminders-state.svelte.js';
	import Combobox from '#lib/components/Combobox.svelte';
	import FileText from '@lucide/svelte/icons/file-text';
	import ListTodo from '@lucide/svelte/icons/list-todo';

	// Create or edit a reminder: text, date, time, repeat rule and quick-pick chips. Used on
	// /reminders (new + inline edit), in TodoModal and in the note "Remind me…" dialog.

	let {
		reminder = null,
		text: initialText = '',
		todoId = null,
		noteId = null,
		linkTitle = null,
		showLink = true,
		bare = false,
		autofocus = true,
		onsaved,
		oncancel
	}: {
		/** Editing this row; null creates. */
		reminder?: Reminder | null;
		/** Prefill for a new reminder. */
		text?: string;
		todoId?: number | null;
		noteId?: number | null;
		/** Title of the prefilled todo/note link. */
		linkTitle?: string | null;
		/** Shows the todo/note link picker (off where the link is implied, e.g. TodoModal). */
		showLink?: boolean;
		/** No panel chrome (inside a dialog). */
		bare?: boolean;
		autofocus?: boolean;
		onsaved: (r: Reminder) => void;
		oncancel: () => void;
	} = $props();

	const REPEATS: { value: ReminderRepeat; label: string }[] = [
		{ value: 'none', label: 'Once' },
		{ value: 'daily', label: 'Every day' },
		{ value: 'weekdays', label: 'Every weekday (Mon–Fri)' },
		{ value: 'weekly', label: 'Weekly on…' },
		{ value: 'monthly', label: 'Monthly on day…' }
	];
	const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

	// Mounted per open form, so it starts from the props at mount. A recurring row starts from
	// its anchor time (not a snoozed occurrence's), so a rule-only change keeps the anchor.
	// svelte-ignore state_referenced_locally
	const start = reminder ? formStart(reminder, new Date()) : splitTimestamp(snoozeUntil('1h', new Date()));
	// svelte-ignore state_referenced_locally
	const original = reminder;

	// svelte-ignore state_referenced_locally
	let text = $state(original?.text ?? initialText);
	let date = $state(start.date);
	let time = $state(start.time);
	let repeat = $state<ReminderRepeat>(original?.repeat ?? 'none');
	let weekdays = $state<number[]>(original?.weekdays.length ? [...original.weekdays] : []);
	let monthDay = $state<number | null>(original?.month_day ?? null);
	type LinkKind = 'none' | 'todo' | 'note';
	type Item = { value: number; label: string };
	// svelte-ignore state_referenced_locally
	const initialLink: { kind: LinkKind; id: number; title: string } | null =
		original?.todo_id != null
			? { kind: 'todo', id: original.todo_id, title: original.todo_title ?? 'Todo' }
			: original?.note_id != null
				? { kind: 'note', id: original.note_id, title: original.note_title ?? 'Note' }
				: todoId !== null
					? { kind: 'todo', id: todoId, title: linkTitle ?? 'Todo' }
					: noteId !== null
						? { kind: 'note', id: noteId, title: linkTitle ?? 'Note' }
						: null;
	const LINK_KINDS: { kind: LinkKind; label: string }[] = [
		{ kind: 'none', label: 'None' },
		{ kind: 'todo', label: 'Todo' },
		{ kind: 'note', label: 'Note' }
	];

	let linkKind = $state<LinkKind>(initialLink?.kind ?? 'none');
	/** Combobox value: the linked id as a string, '' when nothing is picked. */
	let linkTodo = $state(initialLink?.kind === 'todo' ? String(initialLink.id) : '');
	let linkNote = $state(initialLink?.kind === 'note' ? String(initialLink.id) : '');
	let todoItems = $state<Item[] | null>(null);
	let noteItems = $state<Item[] | null>(null);
	let linkError = $state('');
	let linkBox = $state<HTMLElement>();

	/** The current link stays pickable (and labelled) before and after the lists load. */
	function withCurrent(items: Item[] | null, kind: LinkKind): Item[] {
		const list = items ?? [];
		if (initialLink?.kind !== kind || list.some((i) => i.value === initialLink.id)) return list;
		return [{ value: initialLink.id, label: initialLink.title }, ...list];
	}
	const todoOptions = $derived(withCurrent(todoItems, 'todo'));
	const noteOptions = $derived(withCurrent(noteItems, 'note'));

	/** Loads the todo or note titles the first time that kind is picked. */
	async function loadLinkTargets(kind: LinkKind): Promise<void> {
		linkError = '';
		try {
			if (kind === 'todo' && todoItems === null) {
				const rows = await invoke<TodoRow[]>('list_todos');
				todoItems = rows.filter((t) => t.status !== 'done').map((t) => ({ value: t.id, label: t.title }));
			} else if (kind === 'note' && noteItems === null) {
				const rows = await invoke<NoteSummary[]>('list_notes');
				noteItems = rows.map((n) => ({ value: n.id, label: n.title })).sort((a, b) => a.label.localeCompare(b.label));
			}
		} catch (e) {
			linkError = `Could not load ${kind === 'todo' ? 'todos' : 'notes'}: ${String(e)}`;
		}
	}

	function setLinkKind(kind: LinkKind): void {
		linkKind = kind;
		if (kind !== 'none') void loadLinkTargets(kind);
	}

	/** Esc with the link list open closes only the list, not the form. */
	function onLinkKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape' && linkBox?.querySelector('ul')) {
			e.preventDefault();
			e.stopPropagation();
		}
	}
	let error = $state('');
	let saving = $state(false);
	let textInput = $state<HTMLInputElement>();

	onMount(() => {
		if (autofocus) textInput?.focus();
		if (showLink && linkKind !== 'none') void loadLinkTargets(linkKind);
	});

	/** ISO weekday (1 = Mon) and day of month of the picked date. */
	function dateParts(): { wd: number; d: number } {
		const [y, m, d] = date.split('-').map(Number);
		const day = new Date(y, (m || 1) - 1, d || 1);
		return { wd: ((day.getDay() + 6) % 7) + 1, d: day.getDate() };
	}

	function setRepeat(value: ReminderRepeat): void {
		repeat = value;
		if (value === 'weekly' && !weekdays.length) weekdays = [dateParts().wd];
		if (value === 'monthly' && monthDay === null) monthDay = dateParts().d;
	}

	function toggleDay(wd: number): void {
		weekdays = weekdays.includes(wd) ? weekdays.filter((d) => d !== wd) : [...weekdays, wd].sort((a, b) => a - b);
	}

	function pickQuick(choice: QuickChoice): void {
		({ date, time } = splitTimestamp(quickTime(choice, new Date())));
		error = '';
	}

	function sameRule(): boolean {
		if (!original || original.repeat !== repeat) return false;
		if (repeat === 'weekly') return [...original.weekdays].sort().join() === [...weekdays].sort().join();
		if (repeat === 'monthly') return original.month_day === monthDay;
		return true;
	}

	async function submit(): Promise<void> {
		if (saving) return;
		error = '';
		if (!date || !time) {
			error = 'Pick a date and a time';
			return;
		}
		let remindAt = `${date}T${time}:00.000`;
		const unchanged = original && date === start.date && time === start.time && sameRule();
		if (unchanged) {
			// Text- or link-only edit: send the row's current time (the scheduler or a snooze
			// may have moved it since the form opened), so Rust keeps its time and state.
			remindAt = reminders.list.find((r) => r.id === original.id)?.remind_at ?? reminder?.remind_at ?? original.remind_at;
		}
		// A recurring start that has passed is rolled forward by Rust.
		const todoLink = linkKind === 'todo' && linkTodo !== '' ? Number(linkTodo) : null;
		const noteLink = linkKind === 'note' && linkNote !== '' ? Number(linkNote) : null;
		saving = true;
		try {
			const saved = await saveReminder(original?.id ?? null, {
				text,
				remindAt,
				repeat,
				weekdays: repeat === 'weekly' ? weekdays : [],
				monthDay: repeat === 'monthly' ? monthDay : null,
				todoId: todoLink,
				noteId: noteLink
			});
			onsaved(saved);
		} catch (e) {
			error = String(e);
		} finally {
			saving = false;
		}
	}

	function onkeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			oncancel();
		} else if (e.key === 'Enter' && !e.defaultPrevented && e.target instanceof HTMLInputElement && e.target.type !== 'checkbox') {
			e.preventDefault();
			e.stopPropagation(); // not TodoModal's ⌘↩ save
			void submit();
		}
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
	class={bare ? 'space-y-3' : 'reminder-form'}
	aria-label={original ? 'Edit reminder' : 'New reminder'}
	{onkeydown}
	onsubmit={(e) => {
		e.preventDefault();
		void submit();
	}}
>
	<input
		bind:this={textInput}
		bind:value={text}
		class="form-input"
		placeholder="What should I remind you about?"
		aria-label="Reminder text"
		maxlength={500}
		autocomplete="off"
	/>

	<div class="flex flex-wrap items-center gap-1.5">
		{#each QUICK_TIMES as q (q.choice)}
			<button type="button" class="reminder-chip" onclick={() => pickQuick(q.choice)}>{q.label}</button>
		{/each}
	</div>

	<div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_8.5rem_minmax(0,1.2fr)]">
		<label class="block">
			<span class="sr-only">{repeat === 'none' ? 'Date' : 'Starts'}</span>
			<input type="date" class="form-input" bind:value={date} title={repeat === 'none' ? 'Date' : 'Starts'} />
		</label>
		<label class="block">
			<span class="sr-only">Time</span>
			<input type="time" class="form-input" bind:value={time} title="Time" />
		</label>
		<label class="block">
			<span class="sr-only">Repeat</span>
			<select class="form-select" value={repeat} onchange={(e) => setRepeat(e.currentTarget.value as ReminderRepeat)} title="Repeat">
				{#each REPEATS as r (r.value)}
					<option value={r.value}>{r.label}</option>
				{/each}
			</select>
		</label>
	</div>

	{#if repeat === 'weekly'}
		<div class="flex flex-wrap items-center gap-1" role="group" aria-label="Weekdays" transition:slide={{ duration: 150 }}>
			{#each DAYS as label, i (label)}
				{@const on = weekdays.includes(i + 1)}
				<button type="button" class="reminder-day" class:reminder-day-on={on} aria-pressed={on} onclick={() => toggleDay(i + 1)}>{label}</button>
			{/each}
		</div>
	{:else if repeat === 'monthly'}
		<label class="flex items-center gap-2 text-sm text-slate-600 dark:text-neutral-400" transition:slide={{ duration: 150 }}>
			On day
			<input type="number" min="1" max="31" class="form-input w-20" bind:value={monthDay} aria-label="Day of the month" />
			<span class="text-xs text-slate-400 dark:text-neutral-500">Shorter months use their last day</span>
		</label>
	{/if}

	{#if showLink}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="flex flex-wrap items-center gap-2" bind:this={linkBox} onkeydown={onLinkKeydown}>
			<span class="text-xs font-medium text-slate-500 dark:text-neutral-400">Link</span>
			<div class="segmented gap-0.5 p-0.5" role="radiogroup" aria-label="Link to">
				{#each LINK_KINDS as k (k.kind)}
					<label class="segmented-option gap-1 px-2.5 py-1 text-xs" class:segmented-option-active={linkKind === k.kind}>
						<input class="sr-only" type="radio" name="link-kind" value={k.kind} checked={linkKind === k.kind} onchange={() => setLinkKind(k.kind)} />
						{#if k.kind === 'todo'}<ListTodo size={12} />{:else if k.kind === 'note'}<FileText size={12} />{/if}
						{k.label}
					</label>
				{/each}
			</div>
			{#if linkKind === 'todo'}
				<div class="min-w-[12rem] flex-1" transition:slide={{ axis: 'x', duration: 150 }}>
					<Combobox items={todoOptions} bind:value={linkTodo} placeholder="Pick a todo…" picker />
				</div>
			{:else if linkKind === 'note'}
				<div class="min-w-[12rem] flex-1" transition:slide={{ axis: 'x', duration: 150 }}>
					<Combobox items={noteOptions} bind:value={linkNote} placeholder="Pick a note…" picker />
				</div>
			{/if}
		</div>
		{#if linkError}
			<p class="form-error" role="alert">{linkError}</p>
		{/if}
	{/if}

	{#if error}
		<p class="form-error" role="alert">{error}</p>
	{/if}

	<div class="flex items-center justify-end gap-2">
		<button type="button" class="btn-secondary-sm" onclick={oncancel}>Cancel</button>
		<button type="submit" class="btn-primary rounded-lg px-3 py-1 text-xs" disabled={saving}>{original ? 'Save' : 'Set reminder'}</button>
	</div>
</form>
