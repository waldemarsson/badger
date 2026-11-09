// Shared scratchpad drawer state: open/closed and the restore signal. The drawer
// (ScratchpadDrawer.svelte, mounted once in the layout) reacts to both.

export const scratchpad = $state({
	/** Always starts closed on launch. */
	open: false,
	/** Bumped after a backup restore; the drawer drops its state and reloads the pad. */
	restored: 0
});

/** Where focus goes back to when the drawer closes. */
let returnFocus: HTMLElement | null = null;

export function openScratchpad(): void {
	if (scratchpad.open) return;
	returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
	scratchpad.open = true;
}

export function closeScratchpad(): void {
	scratchpad.open = false;
}

export function toggleScratchpad(): void {
	if (scratchpad.open) closeScratchpad();
	else openScratchpad();
}

/** The element focused before the drawer opened (cleared once taken). */
export function takeReturnFocus(): HTMLElement | null {
	const el = returnFocus;
	returnFocus = null;
	return el;
}

/** Settings calls this after a successful restore. */
export function scratchpadRestored(): void {
	scratchpad.restored += 1;
}
