// Where a context menu goes: at the pointer (or under a row, for keyboard opening),
// flipped and shifted so it stays inside the window. Pure, so node:test covers it.

export interface MenuPlacement {
	left: number;
	top: number;
}

/**
 * Top-left corner for a `w`×`h` menu opened at (`x`, `y`) in a `vw`×`vh` window. It opens
 * down-right of the point; one that would cross the right or bottom edge (minus `margin`)
 * flips to the other side, then everything is clamped inside the margins. A window smaller
 * than the menu pins it to the top-left margin. `anchorHeight` is the height of the row a
 * keyboard-opened menu sits under (`y` is the row's bottom): flipping up clears the row.
 */
export function placeMenu(
	x: number,
	y: number,
	w: number,
	h: number,
	vw: number,
	vh: number,
	margin = 8,
	anchorHeight = 0
): MenuPlacement {
	const left = x + w > vw - margin ? x - w : x;
	const top = y + h > vh - margin ? y - anchorHeight - h : y;
	return { left: clamp(left, margin, vw - w - margin), top: clamp(top, margin, vh - h - margin) };
}

/** `value` within [min, max]; `min` wins when the range is empty. */
function clamp(value: number, min: number, max: number): number {
	return Math.max(min, Math.min(value, max));
}
