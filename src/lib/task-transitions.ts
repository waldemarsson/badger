// Row transitions for task lists (the /tasks groups, the Today card). The leaving row's height
// is measured (Svelte `slide`), so rows below close the gap smoothly whatever its chips wrap to.
import { tick } from 'svelte';
import { cubicIn, cubicOut } from 'svelte/easing';
import { slide, type TransitionConfig } from 'svelte/transition';
import { nextFocusId } from '#lib/tasks.js';
import { reducedMotion } from '#lib/tasks-state.svelte.js';

/** `ms`, or 0 when the user asked for less motion. */
export function motionMs(ms: number): number {
	return reducedMotion() ? 0 : ms;
}

/** A completed (or moved, or deleted) row: fades and drifts right while its height folds. */
export function rowLeave(node: Element, { duration = 250 }: { duration?: number } = {}): TransitionConfig {
	const ms = motionMs(duration);
	const s = slide(node, { duration: ms, easing: cubicIn });
	return {
		duration: ms,
		easing: cubicIn,
		css: (t, u) => `${s.css?.(t, u) ?? ''}; opacity: ${t}; transform: translateX(${u * 12}px);`
	};
}

/**
 * The /tasks details panel: from `lg` up it is a column that grows from the right edge and
 * pushes the list (its content keeps its width); below it sits under the list and fades.
 */
export function panelReveal(node: HTMLElement, { duration = 180 }: { duration?: number } = {}): TransitionConfig {
	const ms = motionMs(duration);
	if (!window.matchMedia('(min-width: 1024px)').matches) return { duration: ms, easing: cubicOut, css: (t) => `opacity: ${t};` };
	const width = node.offsetWidth;
	return {
		duration: ms,
		easing: cubicOut,
		css: (t) => `width: ${t * width}px; overflow: hidden; opacity: ${Math.min(1, t * 1.5)};`
	};
}

/** Rows under `root` in list order, without leaving ones (Svelte makes an outroing row inert). */
function liveRows(root: ParentNode): HTMLElement[] {
	return [...root.querySelectorAll<HTMLElement>('[data-task-row]')].filter((el) => !el.closest('[inert]'));
}

/**
 * Called before a row action that may move or remove the row (Schedule, Delete); run what it
 * returns once the action succeeded. When that lost keyboard focus (the row left), focus goes
 * to the task's row where it now shows, else the next row's check (the previous at the end),
 * else `fallback` (quick-add, New task).
 */
export function keepRowFocus(root: ParentNode | undefined, id: number, fallback: () => HTMLElement | null | undefined): () => void {
	const order = root ? liveRows(root).map((el) => Number(el.dataset.taskRow)) : [];
	return () =>
		void tick().then(() => {
			const active = document.activeElement;
			const lost = !active || active === document.body || !active.isConnected || !!active.closest('[inert]');
			if (!root || !lost) return;
			const rows = liveRows(root);
			const byId = (x: number) => rows.find((el) => Number(el.dataset.taskRow) === x);
			const own = byId(id);
			if (own) {
				own.querySelector<HTMLElement>('.task-row-button')?.focus();
				return;
			}
			const next = nextFocusId(
				order.filter((x) => x === id || byId(x)),
				id
			);
			const target = next === null ? fallback() : byId(next)?.querySelector<HTMLElement>('[data-task-check]');
			target?.focus();
		});
}

/** A group whose last row leaves: its header folds with it. */
export function groupLeave(node: Element, { duration = 250 }: { duration?: number } = {}): TransitionConfig {
	return slide(node, { duration: motionMs(duration), easing: cubicIn });
}
