<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade, fly } from 'svelte/transition';
	import { SNOOZE_CHOICES, snoozeUntil } from '#lib/time-phrases.js';
	import { chipWhen, toastStack, type ReminderToast } from '#lib/tasks.js';
	import {
		clock,
		completeTask,
		dismissTaskReminder,
		snoozeTaskReminder,
		tasks,
		toastArea
	} from '#lib/tasks-state.svelte.js';
	import { motionMs } from '#lib/task-transitions.js';
	import { toast } from '#lib/palette-state.svelte.js';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import AlarmClock from '@lucide/svelte/icons/alarm-clock';
	import BellRing from '@lucide/svelte/icons/bell-ring';
	import Check from '@lucide/svelte/icons/check';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import X from '@lucide/svelte/icons/x';

	// Actionable toasts for sent reminders: up to three, then "+N more" → /tasks. Derived from
	// the shared tasks list (so they survive reloads and launch catch-up). Done completes the
	// task, Snooze moves only that reminder, × dismisses it (it stops counting as overdue).
	// Clicking the text opens the task. Shown on every page: the native notification is
	// skipped while the window is focused, so the toast is what announces a reminder.

	const now = $derived(new Date(clock.ms));
	const stack = $derived(toastStack(tasks.list));

	let busy = $state<number | null>(null);
	let errors = $state<Record<number, string>>({});
	let menu = $state<{ t: ReminderToast; x: number; y: number; h: number; button: HTMLElement } | null>(null);
	let container = $state<HTMLElement>();

	async function act(t: ReminderToast, action: () => Promise<unknown>): Promise<void> {
		if (busy !== null) return;
		busy = t.reminder.id;
		let error = '';
		try {
			const result = await action();
			if (typeof result === 'string') error = result;
		} catch (e) {
			error = String(e);
		}
		errors = { ...errors, [t.reminder.id]: error };
		busy = null;
	}

	function openTask(t: ReminderToast): void {
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
		void goto(`${resolve('/tasks')}?id=${t.task.id}`);
	}

	function openSnooze(t: ReminderToast, button: HTMLElement): void {
		const rect = button.getBoundingClientRect();
		menu = { t, x: rect.left, y: rect.bottom + 4, h: rect.height + 4, button };
	}

	// ContextMenu calls onclose before an item's run, so neither may read `menu` (null by then).
	function snoozeItems(t: ReminderToast): MenuItem[] {
		return SNOOZE_CHOICES.map((c) => ({
			label: c.label,
			run: () => void act(t, () => snoozeTaskReminder(t.reminder.id, snoozeUntil(c.choice, new Date())))
		}));
	}

	function closeSnooze(refocus: boolean): void {
		const button = menu?.button;
		menu = null;
		if (refocus && button?.isConnected) button.focus();
	}

	/** × or Esc: dismisses the reminder; keyboard focus moves to the next toast. */
	function dismiss(t: ReminderToast, el: HTMLElement | null): void {
		const toasts = container ? [...container.querySelectorAll<HTMLElement>('[data-toast]')] : [];
		const hadFocus = el?.contains(document.activeElement) ?? false;
		const next = toasts[toasts.indexOf(el as HTMLElement) + 1] ?? toasts[toasts.indexOf(el as HTMLElement) - 1];
		void act(t, () => dismissTaskReminder(t.reminder.id));
		if (hadFocus) next?.focus();
	}

	/**
	 * Esc with focus in a toast dismisses it. Capture phase, so a page or modal behind the toast
	 * doesn't also close; an open snooze menu (not inside a toast) handles its own Esc first.
	 */
	function onWindowKeydown(e: KeyboardEvent): void {
		if (e.key !== 'Escape' || e.isComposing) return;
		const el = document.activeElement instanceof HTMLElement ? document.activeElement.closest<HTMLElement>('[data-toast]') : null;
		const t = el && stack.shown.find((x) => String(x.reminder.id) === el.dataset.toast);
		if (!el || !t) return;
		e.preventDefault();
		e.stopPropagation();
		dismiss(t, el);
	}

	onMount(() => {
		window.addEventListener('keydown', onWindowKeydown, true);
		return () => window.removeEventListener('keydown', onWindowKeydown, true);
	});
</script>

<div
	bind:this={container}
	bind:offsetHeight={toastArea.height}
	class="pointer-events-none fixed right-4 z-[55] flex flex-col gap-2 transition-[bottom] duration-200 {toast.message ? 'bottom-[4.25rem]' : 'bottom-4'}"
	aria-live="polite"
	aria-label="Reminders"
	role="region"
	data-above-scratchpad
>
	{#each stack.shown as t (t.reminder.id)}
		<div
			data-toast={t.reminder.id}
			class="reminder-toast"
			role="group"
			tabindex="-1"
			aria-label="Reminder: {t.task.title}"
			animate:flip={{ duration: motionMs(200) }}
			in:fly={{ y: 12, duration: motionMs(180) }}
			out:fade={{ duration: motionMs(150) }}
		>
			<div class="flex items-start gap-2.5">
				<BellRing size={16} class="mt-0.5 shrink-0 text-red-500 dark:text-red-400" />
				<button type="button" class="min-w-0 flex-1 rounded-md text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-400" title="Open task" onclick={() => openTask(t)}>
					<span class="block text-[11px] font-semibold uppercase tracking-wide text-red-600 dark:text-red-400">
						Reminder · {t.reminder.fire_at ? chipWhen(t.reminder.fire_at, now) : ''}
					</span>
					<span class="mt-0.5 line-clamp-3 block break-words text-sm font-medium text-slate-900 dark:text-neutral-100">{t.task.title}</span>
					{#if t.task.notes}
						<span class="mt-0.5 block truncate text-xs text-slate-500 dark:text-neutral-400">{t.task.notes}</span>
					{/if}
				</button>
				<button
					type="button"
					class="btn-icon-ghost -mr-1 -mt-1 p-1"
					title="Dismiss (Esc)"
					aria-label="Dismiss reminder “{t.task.title}”"
					disabled={busy === t.reminder.id}
					onclick={(e) => dismiss(t, (e.currentTarget as HTMLElement).closest('[data-toast]'))}
				>
					<X size={14} />
				</button>
			</div>
			<div class="mt-2.5 flex items-center gap-2 pl-[1.625rem]">
				<button type="button" class="btn-secondary-sm gap-1" disabled={busy === t.reminder.id} onclick={() => void act(t, () => completeTask(t.task))}>
					<Check size={13} /> Done
				</button>
				<button
					type="button"
					class="btn-secondary-sm gap-1"
					aria-haspopup="menu"
					aria-expanded={menu?.t.reminder.id === t.reminder.id}
					disabled={busy === t.reminder.id}
					onclick={(e) => openSnooze(t, e.currentTarget)}
				>
					<AlarmClock size={13} /> Snooze <ChevronDown size={12} class="-mr-0.5 text-slate-400 dark:text-neutral-500" />
				</button>
			</div>
			{#if errors[t.reminder.id]}
				<p class="form-error mt-1.5 pl-[1.625rem] text-xs" role="alert">{errors[t.reminder.id]}</p>
			{/if}
		</div>
	{/each}
	{#if stack.more > 0}
		<a href={resolve('/tasks')} class="reminder-toast-more" in:fade={{ duration: motionMs(150) }}>+{stack.more} more</a>
	{/if}

	{#if menu}
		<div class="pointer-events-auto">
			<ContextMenu
				x={menu.x}
				y={menu.y}
				anchorHeight={menu.h}
				label="Snooze “{menu.t.task.title}”"
				items={snoozeItems(menu.t)}
				onclose={closeSnooze}
			/>
		</div>
	{/if}
</div>
