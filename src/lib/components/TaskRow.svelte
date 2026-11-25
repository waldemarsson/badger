<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Task } from '#lib/types.js';
	import { completedLabel, needsAttention, scheduleInput, schedulePresets, taskChips, taskLink, type SchedulePreset } from '#lib/tasks.js';
	import { deleteTask, updateTask } from '#lib/tasks-state.svelte.js';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import TaskCheck from '#lib/components/TaskCheck.svelte';
	import TaskChips from '#lib/components/TaskChips.svelte';
	import ArrowRight from '@lucide/svelte/icons/arrow-right';
	import Ban from '@lucide/svelte/icons/ban';
	import Calendar from '@lucide/svelte/icons/calendar';
	import CalendarDays from '@lucide/svelte/icons/calendar-days';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Ellipsis from '@lucide/svelte/icons/ellipsis';
	import Link from '@lucide/svelte/icons/link';
	import PanelRight from '@lucide/svelte/icons/panel-right';
	import Sun from '@lucide/svelte/icons/sun';
	import Sunrise from '@lucide/svelte/icons/sunrise';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	// One task: the check, then title, a notes line and chips (open), or when it was
	// completed (done). Hover or focus shows the row actions at the right: the page's own
	// (`actions`), Schedule, Details and More. Used by /tasks and the Today card.
	let {
		task,
		now,
		selected = false,
		ticking = false,
		syncOn = false,
		menus = false,
		onselect,
		ontoggle,
		ondetails,
		ondelete,
		onerror,
		keepfocus,
		actions
	}: {
		task: Task;
		now: Date;
		selected?: boolean;
		/** Mid-completion: the check pops and the title strikes through. */
		ticking?: boolean;
		/** Apple Reminders sync is on: show "Mac only". */
		syncOn?: boolean;
		/** Show Schedule (open tasks) and More (Complete, Copy link, Delete). */
		menus?: boolean;
		onselect: () => void;
		ontoggle: () => void;
		/** Shows a Details button (opens the panel). */
		ondetails?: () => void;
		/** After More › Delete removed the task. */
		ondelete?: () => void;
		/** A row action failed ('' when one starts): the page shows it near the list. */
		onerror?: (message: string) => void;
		/** Before Schedule or Delete writes; the returned function runs after it succeeded (focus follows a row that left). */
		keepfocus?: () => () => void;
		/** Row buttons before Schedule (e.g. Today's Log time), given the task. */
		actions?: Snippet<[Task]>;
	} = $props();

	const chips = $derived(task.done ? [] : taskChips(task, now, syncOn));

	// ── Menus ───────────────────────────────────────────────────────────────────

	type MenuKind = 'schedule' | 'more';
	let menu = $state<{ kind: MenuKind; x: number; y: number; h: number; button: HTMLElement } | null>(null);
	/** The outside click that closed a menu also clicks its button: don't reopen it. */
	let dismissed: { kind: MenuKind; at: number } | null = null;

	const SCHEDULE_ICONS: Record<SchedulePreset, typeof Sun> = {
		today: Sun,
		tomorrow: Sunrise,
		weekend: CalendarDays,
		nextweek: ArrowRight,
		none: Ban
	};

	function toggleMenu(kind: MenuKind, button: HTMLElement): void {
		if (menu) {
			closeMenu(false);
			return;
		}
		if (dismissed?.kind === kind && performance.now() - dismissed.at < 300) return;
		const rect = button.getBoundingClientRect();
		menu = { kind, x: rect.right, y: rect.bottom + 4, h: rect.height + 4, button };
	}

	function closeMenu(refocus: boolean): void {
		const m = menu;
		menu = null;
		dismissed = m && !refocus ? { kind: m.kind, at: performance.now() } : null;
		if (refocus && m?.button.isConnected) m.button.focus();
	}

	// ContextMenu calls onclose before an item's run, so items must not read `menu`.
	const scheduleItems = $derived<MenuItem[]>(
		schedulePresets(now).map((p) => ({
			label: p.label,
			icon: SCHEDULE_ICONS[p.key],
			hint: p.hint,
			separator: p.key === 'none',
			run: () => void schedule(p.key)
		}))
	);

	const moreItems = $derived<MenuItem[]>([
		{ label: task.done ? 'Mark as not done' : 'Complete', icon: CircleCheck, run: ontoggle },
		{ label: 'Copy link', icon: Link, hint: taskLink(task.id), run: () => void copyLink() },
		{ label: 'Delete', icon: Trash2, danger: true, confirm: 'Delete?', run: () => void remove() }
	]);

	async function act(action: () => Promise<unknown>): Promise<boolean> {
		onerror?.('');
		try {
			await action();
			return true;
		} catch (e) {
			onerror?.(String(e));
			return false;
		}
	}

	/** Moves the due date only (the time stays); a preset on the current date writes nothing. */
	async function schedule(preset: SchedulePreset): Promise<void> {
		const input = scheduleInput(task, preset, new Date());
		if (!input) return;
		const after = keepfocus?.();
		if (await act(() => updateTask(task.id, input))) after?.();
	}

	async function remove(): Promise<void> {
		const after = keepfocus?.();
		if (!(await act(() => deleteTask(task.id)))) return;
		ondelete?.();
		after?.();
	}

	async function copyLink(): Promise<void> {
		onerror?.('');
		try {
			await navigator.clipboard.writeText(taskLink(task.id));
		} catch {
			onerror?.('Could not copy the link');
		}
	}
</script>

<div class="task-row group" class:task-row-selected={selected} class:task-row-menu-open={menu !== null} data-task-row={task.id}>
	<TaskCheck
		done={task.done}
		attention={needsAttention(task, now)}
		{ticking}
		label={task.done ? `Mark ${task.title} as not done` : `Complete ${task.title}`}
		onclick={ontoggle}
	/>
	<button type="button" class="task-row-button" aria-current={selected ? 'true' : undefined} onclick={onselect}>
		<span class="task-title" class:task-title-done={task.done && !ticking} class:task-title-striking={ticking} title={task.title.length > 80 ? task.title : undefined}>{task.title}</span>
		{#if task.done}
			<span class="task-notes">{completedLabel(task, now)}</span>
		{:else}
			{#if task.notes}
				<span class="task-notes">{task.notes}</span>
			{/if}
			{#if chips.length}
				<span class="task-chips"><TaskChips {chips} variant="bare" /></span>
			{/if}
		{/if}
	</button>
	{#if actions || menus || ondetails}
		<div class="task-row-actions" class:task-row-actions-open={menu !== null}>
			{#if actions}
				{@render actions(task)}
			{/if}
			{#if menus && !task.done}
				<button
					type="button"
					class="task-row-action"
					class:task-row-action-on={menu?.kind === 'schedule'}
					title="Schedule"
					aria-label="Schedule {task.title}"
					aria-haspopup="menu"
					aria-expanded={menu?.kind === 'schedule'}
					onclick={(e) => toggleMenu('schedule', e.currentTarget)}
				>
					<Calendar size={16} />
				</button>
			{/if}
			{#if ondetails}
				<button type="button" class="task-row-action" title="Details" aria-label="Details for {task.title}" onclick={ondetails}>
					<PanelRight size={16} />
				</button>
			{/if}
			{#if menus}
				<button
					type="button"
					class="task-row-action"
					class:task-row-action-on={menu?.kind === 'more'}
					title="More"
					aria-label="More actions for {task.title}"
					aria-haspopup="menu"
					aria-expanded={menu?.kind === 'more'}
					onclick={(e) => toggleMenu('more', e.currentTarget)}
				>
					<Ellipsis size={16} />
				</button>
			{/if}
		</div>
	{/if}
	{#if menu}
		<ContextMenu
			x={menu.x}
			y={menu.y}
			anchorHeight={menu.h}
			align="end"
			label={menu.kind === 'schedule' ? `Schedule ${task.title}` : `Actions for ${task.title}`}
			items={menu.kind === 'schedule' ? scheduleItems : moreItems}
			onclose={closeMenu}
		/>
	{/if}
</div>
