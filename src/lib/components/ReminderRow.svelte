<script lang="ts">
	import { resolve } from '$app/paths';
	import type { Reminder } from '#lib/types.js';
	import { SNOOZE_CHOICES, snoozeUntil, formatRemindAt, formatRepeat, splitTimestamp, toLocalTimestamp } from '#lib/reminders.js';
	import { completeReminder, deleteReminder, snoozeReminder } from '#lib/reminders-state.svelte.js';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import AlarmClock from '@lucide/svelte/icons/alarm-clock';
	import Bell from '@lucide/svelte/icons/bell';
	import BellRing from '@lucide/svelte/icons/bell-ring';
	import Check from '@lucide/svelte/icons/check';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Clock from '@lucide/svelte/icons/clock';
	import FileText from '@lucide/svelte/icons/file-text';
	import ListTodo from '@lucide/svelte/icons/list-todo';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Repeat from '@lucide/svelte/icons/repeat';
	import Trash2 from '@lucide/svelte/icons/trash';

	// One reminder: state icon, text, time / repeat / link badges, and Done · Snooze · Edit ·
	// Delete (inline confirm). Errors show under the row. `compact` (Today card, TodoModal)
	// keeps Done and Snooze only and can drop the link chip.

	let {
		r,
		now,
		compact = false,
		showLink = true,
		flash = false,
		onedit
	}: {
		r: Reminder;
		now: Date;
		compact?: boolean;
		showLink?: boolean;
		/** Briefly highlighted (opened from a notification). */
		flash?: boolean;
		/** Shows the Edit button. */
		onedit?: () => void;
	} = $props();

	let error = $state('');
	let busy = $state(false);
	let confirming = $state(false);
	let menu = $state<{ x: number; y: number; h: number } | null>(null);
	let snoozeButton = $state<HTMLButtonElement>();

	const todayKey = $derived(toLocalTimestamp(now).slice(0, 10));
	const kind = $derived(
		r.state === 'done' ? 'done' : r.state === 'fired' ? 'overdue' : splitTimestamp(r.remind_at).date <= todayKey ? 'today' : 'later'
	);
	const timeLabel = $derived(formatRemindAt(r.remind_at, now));
	const repeatLabel = $derived(formatRepeat(r));

	const badgeClasses: Record<string, string> = {
		overdue: 'bg-red-50 text-red-700 ring-red-200 dark:bg-red-500/10 dark:text-red-300 dark:ring-red-500/30',
		today: 'bg-amber-50 text-amber-700 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-300 dark:ring-amber-500/30',
		later: 'bg-slate-50 text-slate-600 ring-slate-200 dark:bg-neutral-800/40 dark:text-neutral-400 dark:ring-neutral-700',
		done: 'bg-slate-50 text-slate-400 ring-slate-200 dark:bg-neutral-800/40 dark:text-neutral-500 dark:ring-neutral-800'
	};

	const snoozeItems: MenuItem[] = SNOOZE_CHOICES.map((c) => ({
		label: c.label,
		run: () => void act(() => snoozeReminder(r.id, snoozeUntil(c.choice, new Date())))
	}));

	async function act(action: () => Promise<string>): Promise<void> {
		if (busy) return;
		busy = true;
		error = await action();
		busy = false;
	}

	function openSnooze(): void {
		if (!snoozeButton) return;
		const rect = snoozeButton.getBoundingClientRect();
		menu = { x: rect.left, y: rect.bottom + 4, h: rect.height + 4 };
	}
</script>

<div class="reminder-row group" class:reminder-row-flash={flash} class:reminder-row-confirm={confirming} data-reminder-id={r.id}>
	<span class="mt-0.5 shrink-0" title={kind === 'overdue' ? 'Overdue' : kind === 'done' ? 'Done' : 'Pending'}>
		{#if kind === 'overdue'}
			<BellRing size={16} class="text-red-500 dark:text-red-400" />
		{:else if kind === 'done'}
			<CircleCheck size={16} class="text-emerald-500 dark:text-emerald-400" />
		{:else}
			<Bell size={16} class={kind === 'today' ? 'text-amber-500 dark:text-amber-400' : 'text-slate-400 dark:text-neutral-500'} />
		{/if}
	</span>

	<div class="min-w-0 flex-1">
		<p
			class="text-sm font-medium leading-snug {kind === 'done'
				? 'text-slate-400 line-through decoration-slate-300 dark:text-neutral-500 dark:decoration-neutral-600'
				: 'text-slate-800 dark:text-neutral-200'} {compact ? 'truncate' : 'break-words'}"
			title={compact ? r.text : undefined}
		>{r.text}</p>
		<div class="mt-1 flex min-w-0 flex-wrap items-center gap-1.5">
			<span class="kanban-badge shrink-0 {badgeClasses[kind]}">
				<Clock size={12} class="shrink-0" />
				{kind === 'overdue' ? `Overdue · ${timeLabel}` : timeLabel}
			</span>
			{#if repeatLabel}
				<span class="kanban-badge shrink-0 {badgeClasses.later}" title="Repeats {repeatLabel}">
					<Repeat size={12} class="shrink-0" />
					{repeatLabel}
				</span>
			{/if}
			{#if showLink && r.todo_id !== null}
				<a href={resolve('/todos')} class="reminder-link min-w-0" title="Todo: {r.todo_title}">
					<ListTodo size={12} class="shrink-0" />
					<span class="truncate">{r.todo_title}</span>
				</a>
			{:else if showLink && r.note_id !== null}
				<a href="{resolve('/notes')}?id={r.note_id}" class="reminder-link min-w-0" title="Note: {r.note_title}">
					<FileText size={12} class="shrink-0" />
					<span class="truncate">{r.note_title}</span>
				</a>
			{/if}
		</div>
		{#if error}
			<p class="form-error mt-1 text-xs" role="alert">{error}</p>
		{/if}
	</div>

	{#if confirming}
		<div class="flex shrink-0 items-center gap-2 self-center">
			<span class="text-xs text-red-700 dark:text-red-300">Delete?</span>
			<button type="button" class="btn-danger-sm" disabled={busy} onclick={() => void act(() => deleteReminder(r.id))}>Delete</button>
			<button type="button" class="btn-secondary-sm" onclick={() => (confirming = false)}>Keep</button>
		</div>
	{:else}
		<div class="flex shrink-0 items-center gap-0.5 self-center">
			{#if !compact}
				<div class="row-actions">
					{#if onedit && kind !== 'done'}
						<button type="button" class="btn-icon-ghost" title="Edit" aria-label="Edit “{r.text}”" onclick={onedit}>
							<Pencil size={15} />
						</button>
					{/if}
					<button type="button" class="btn-icon-ghost-danger" title="Delete" aria-label="Delete “{r.text}”" onclick={() => (confirming = true)}>
						<Trash2 size={15} />
					</button>
				</div>
			{/if}
			{#if kind !== 'done'}
				<button type="button" class="btn-icon-ghost-confirm" title="Done" aria-label="Mark “{r.text}” done" disabled={busy} onclick={() => void act(() => completeReminder(r.id))}>
					<Check size={16} />
				</button>
				<button
					bind:this={snoozeButton}
					type="button"
					class="btn-icon-ghost"
					title="Snooze"
					aria-label="Snooze “{r.text}”"
					aria-haspopup="menu"
					aria-expanded={menu !== null}
					disabled={busy}
					onclick={openSnooze}
				>
					<AlarmClock size={16} />
				</button>
			{/if}
		</div>
	{/if}
</div>

{#if menu}
	<ContextMenu
		x={menu.x}
		y={menu.y}
		anchorHeight={menu.h}
		label="Snooze “{r.text}”"
		items={snoozeItems}
		onclose={(refocus) => {
			menu = null;
			if (refocus) snoozeButton?.focus();
		}}
	/>
{/if}
