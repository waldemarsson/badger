<script lang="ts">
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade, fly } from 'svelte/transition';
	import type { Reminder } from '#lib/types.js';
	import { SNOOZE_CHOICES, snoozeUntil, formatRemindAt, toastStack } from '#lib/reminders.js';
	import {
		clock,
		completeReminder,
		dismissToast,
		dismissedToasts,
		reminders,
		snoozeReminder,
		toastArea
	} from '#lib/reminders-state.svelte.js';
	import { toast } from '#lib/palette-state.svelte.js';
	import ContextMenu from '#lib/components/ContextMenu.svelte';
	import AlarmClock from '@lucide/svelte/icons/alarm-clock';
	import BellRing from '@lucide/svelte/icons/bell-ring';
	import Check from '@lucide/svelte/icons/check';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import FileText from '@lucide/svelte/icons/file-text';
	import ListTodo from '@lucide/svelte/icons/list-todo';
	import X from '@lucide/svelte/icons/x';

	// Actionable toasts for fired reminders: up to three, then "+N more" → /reminders.
	// Derived from the shared list (so they survive reloads and launch catch-up); Done and
	// Snooze clear them through the next list refresh, × hides one for this session.
	// Shown on every page, /reminders included: the native notification is skipped while the
	// window is focused, so the toast is what announces a firing.

	const now = $derived(new Date(clock.ms));
	const stack = $derived(toastStack(reminders.list, dismissedToasts));

	let busy = $state<number | null>(null);
	let errors = $state<Record<number, string>>({});
	let menu = $state<{ r: Reminder; x: number; y: number; h: number; button: HTMLElement } | null>(null);
	let container = $state<HTMLElement>();

	async function act(r: Reminder, action: () => Promise<string>): Promise<void> {
		if (busy !== null) return;
		busy = r.id;
		const error = await action();
		errors = { ...errors, [r.id]: error };
		busy = null;
	}

	function openSnooze(r: Reminder, button: HTMLElement): void {
		const rect = button.getBoundingClientRect();
		menu = { r, x: rect.left, y: rect.bottom + 4, h: rect.height + 4, button };
	}

	/** × or Esc: hides the toast; keyboard focus moves to the next one. */
	function dismiss(r: Reminder, el: HTMLElement | null): void {
		const toasts = container ? [...container.querySelectorAll<HTMLElement>('[data-toast]')] : [];
		const hadFocus = el?.contains(document.activeElement) ?? false;
		const next = toasts[toasts.indexOf(el as HTMLElement) + 1] ?? toasts[toasts.indexOf(el as HTMLElement) - 1];
		dismissToast(r);
		if (hadFocus) next?.focus();
	}

	/**
	 * Esc with focus in a toast hides it. Capture phase, so a page or modal behind the toast
	 * doesn't also close; an open snooze menu (not inside a toast) handles its own Esc first.
	 */
	function onWindowKeydown(e: KeyboardEvent): void {
		if (e.key !== 'Escape' || e.isComposing) return;
		const el = document.activeElement instanceof HTMLElement ? document.activeElement.closest<HTMLElement>('[data-toast]') : null;
		const r = el && stack.shown.find((x) => String(x.id) === el.dataset.toast);
		if (!el || !r) return;
		e.preventDefault();
		e.stopPropagation();
		dismiss(r, el);
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
>
	{#each stack.shown as r (r.id)}
		<div
			data-toast={r.id}
			class="reminder-toast"
			role="group"
			tabindex="-1"
			aria-label="Reminder: {r.text}"
			animate:flip={{ duration: 200 }}
			in:fly={{ y: 12, duration: 180 }}
			out:fade={{ duration: 150 }}
		>
			<div class="flex items-start gap-2.5">
				<BellRing size={16} class="mt-0.5 shrink-0 text-red-500 dark:text-red-400" />
				<div class="min-w-0 flex-1">
					<p class="text-[11px] font-semibold uppercase tracking-wide text-red-600 dark:text-red-400">
						Reminder · {formatRemindAt(r.remind_at, now)}
					</p>
					<p class="mt-0.5 line-clamp-3 break-words text-sm font-medium text-slate-900 dark:text-neutral-100">{r.text}</p>
					{#if r.todo_id !== null}
						<a href={resolve('/todos')} class="reminder-link mt-1.5 max-w-full">
							<ListTodo size={12} class="shrink-0" /><span class="truncate">{r.todo_title}</span>
						</a>
					{:else if r.note_id !== null}
						<a href="{resolve('/notes')}?id={r.note_id}" class="reminder-link mt-1.5 max-w-full">
							<FileText size={12} class="shrink-0" /><span class="truncate">{r.note_title}</span>
						</a>
					{/if}
				</div>
				<button
					type="button"
					class="btn-icon-ghost -mr-1 -mt-1 p-1"
					title="Hide (Esc)"
					aria-label="Hide reminder “{r.text}”"
					onclick={(e) => dismiss(r, (e.currentTarget as HTMLElement).closest('[data-toast]'))}
				>
					<X size={14} />
				</button>
			</div>
			<div class="mt-2.5 flex items-center gap-2 pl-[1.625rem]">
				<button type="button" class="btn-secondary-sm gap-1" disabled={busy === r.id} onclick={() => void act(r, () => completeReminder(r.id))}>
					<Check size={13} /> Done
				</button>
				<button
					type="button"
					class="btn-secondary-sm gap-1"
					aria-haspopup="menu"
					aria-expanded={menu?.r.id === r.id}
					disabled={busy === r.id}
					onclick={(e) => openSnooze(r, e.currentTarget)}
				>
					<AlarmClock size={13} /> Snooze <ChevronDown size={12} class="-mr-0.5 text-slate-400 dark:text-neutral-500" />
				</button>
			</div>
			{#if errors[r.id]}
				<p class="form-error mt-1.5 pl-[1.625rem] text-xs" role="alert">{errors[r.id]}</p>
			{/if}
		</div>
	{/each}
	{#if stack.more > 0}
		<a href={resolve('/reminders')} class="reminder-toast-more" in:fade={{ duration: 150 }}>+{stack.more} more</a>
	{/if}

	{#if menu}
		{@const m = menu}
		<div class="pointer-events-auto">
			<ContextMenu
				x={m.x}
				y={m.y}
				anchorHeight={m.h}
				label="Snooze “{m.r.text}”"
				items={SNOOZE_CHOICES.map((c) => ({ label: c.label, run: () => void act(m.r, () => snoozeReminder(m.r.id, snoozeUntil(c.choice, new Date()))) }))}
				onclose={(refocus) => {
					menu = null;
					if (refocus && m.button.isConnected) m.button.focus();
				}}
			/>
		</div>
	{/if}
</div>
