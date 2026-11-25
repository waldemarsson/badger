// Row transitions for task lists (the /tasks groups, the Today card). The leaving row's height
// is measured (Svelte `slide`), so rows below close the gap smoothly whatever its chips wrap to.
import { cubicIn } from 'svelte/easing';
import { slide, type TransitionConfig } from 'svelte/transition';
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

/** A group whose last row leaves: its header folds with it. */
export function groupLeave(node: Element, { duration = 250 }: { duration?: number } = {}): TransitionConfig {
	return slide(node, { duration: motionMs(duration), easing: cubicIn });
}
