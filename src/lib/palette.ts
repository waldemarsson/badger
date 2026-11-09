// Pure helpers for the command palette: shortcuts, filtering, hours and dates.

export const DEFAULT_SHORTCUT = 'Meta+K';

const MODIFIERS = ['Meta', 'Ctrl', 'Alt', 'Shift'] as const;
type Modifier = (typeof MODIFIERS)[number];

/** Shortcuts macOS or the WebView already own; refused when recording. */
const RESERVED = ['Meta+Q', 'Meta+W', 'Meta+H', 'Meta+M', 'Meta+C', 'Meta+V', 'Meta+X', 'Meta+A', 'Meta+Z', 'Meta+Comma'];

const MODIFIER_CODES = new Set([
	'MetaLeft', 'MetaRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'CapsLock', 'Fn'
]);

const KEY_LABELS: Record<string, string> = {
	Space: 'Space', Comma: ',', Period: '.', Slash: '/', Semicolon: ';', Quote: "'", Backquote: '`',
	BracketLeft: '[', BracketRight: ']', Backslash: '\\', Minus: '-', Equal: '=',
	Enter: '↩', Escape: 'Esc', Tab: '⇥', Backspace: '⌫',
	ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→'
};

const MODIFIER_SYMBOLS: Record<Modifier, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Meta: '⌘' };

export interface KeyLike {
	code: string;
	metaKey: boolean;
	ctrlKey: boolean;
	altKey: boolean;
	shiftKey: boolean;
}

/** `KeyK` → `K`, `Digit1` → `1`, other codes unchanged. Uses the physical key so ⌥ combos stay readable. */
function keyName(code: string): string {
	if (/^Key[A-Z]$/.test(code)) return code.slice(3);
	if (/^Digit[0-9]$/.test(code)) return code.slice(5);
	return code;
}

/** Canonical shortcut string (`Meta+Shift+K`) for a key event, or null for a bare modifier press. */
export function shortcutFromEvent(e: KeyLike): string | null {
	if (!e.code || MODIFIER_CODES.has(e.code)) return null;
	const mods: Modifier[] = [];
	if (e.metaKey) mods.push('Meta');
	if (e.ctrlKey) mods.push('Ctrl');
	if (e.altKey) mods.push('Alt');
	if (e.shiftKey) mods.push('Shift');
	return [...mods, keyName(e.code)].join('+');
}

/** Returns a user-facing error, or null when the shortcut may be used. */
export function validateShortcut(shortcut: string): string | null {
	const parts = shortcut.split('+');
	const key = parts.at(-1) ?? '';
	const mods = parts.slice(0, -1);
	if (!key || (MODIFIERS as readonly string[]).includes(key)) return 'Press a key together with the modifiers';
	if (mods.some((m) => !(MODIFIERS as readonly string[]).includes(m))) return 'Unrecognised shortcut';
	if (!mods.some((m) => m === 'Meta' || m === 'Ctrl' || m === 'Alt')) return 'Include ⌘, ⌃ or ⌥ in the shortcut';
	if (RESERVED.includes(shortcut)) return `${formatShortcut(shortcut)} is reserved by macOS`;
	return null;
}

/**
 * The Settings recorder's check: `validateShortcut`, and ⌘J, which opens the scratchpad.
 * Only the recorder rejects ⌘J, so a palette shortcut saved as ⌘J before keeps working.
 */
export function recorderProblem(shortcut: string): string | null {
	if (shortcut === SCRATCHPAD_SHORTCUT) return `${formatShortcut(SCRATCHPAD_SHORTCUT)} opens the Scratchpad`;
	return validateShortcut(shortcut);
}

/** Stored values that are missing or invalid fall back to ⌘K. */
export function normalizeShortcut(stored: unknown): string {
	return typeof stored === 'string' && validateShortcut(stored) === null ? stored : DEFAULT_SHORTCUT;
}

export function matchesShortcut(e: KeyLike, shortcut: string): boolean {
	return shortcutFromEvent(e) === shortcut;
}

/** Toggles the scratchpad drawer on every page. Fixed, not configurable. */
export const SCRATCHPAD_SHORTCUT = 'Meta+J';

/**
 * True when `e` should toggle the scratchpad: ⌘J, unless the palette is open or recording
 * (its keys win), or the palette's own saved shortcut is ⌘J (the palette wins then).
 */
export function togglesScratchpad(
	e: KeyLike,
	ctx: { paletteOpen: boolean; recording: boolean; paletteShortcut: string }
): boolean {
	if (ctx.paletteOpen || ctx.recording || ctx.paletteShortcut === SCRATCHPAD_SHORTCUT) return false;
	return matchesShortcut(e, SCRATCHPAD_SHORTCUT);
}

/** `Meta+Shift+K` → `⇧⌘K` (macOS modifier order). */
export function formatShortcut(shortcut: string): string {
	const parts = shortcut.split('+');
	const key = parts.at(-1) ?? '';
	const mods = parts.slice(0, -1);
	const order: Modifier[] = ['Ctrl', 'Alt', 'Shift', 'Meta'];
	const symbols = order.filter((m) => mods.includes(m)).map((m) => MODIFIER_SYMBOLS[m]).join('');
	return symbols + (KEY_LABELS[key] ?? key);
}

function fold(text: string): string {
	return text.normalize('NFC').toLocaleLowerCase();
}

/** Keeps items whose text contains every whitespace-separated query token (case-insensitive). Order is preserved. */
export function filterItems<T>(items: readonly T[], query: string, text: (item: T) => string): T[] {
	const tokens = fold(query).split(/\s+/).filter(Boolean);
	if (tokens.length === 0) return [...items];
	return items.filter((item) => {
		const haystack = fold(text(item));
		return tokens.every((t) => haystack.includes(t));
	});
}

/** Plain decimal hours, 0 < h ≤ 24. */
export function parseHours(input: string): { hours: number } | { error: string } {
	const value = input.trim();
	if (!value) return { error: 'Hours are required' };
	if (!/^\d+(\.\d+)?$|^\.\d+$/.test(value)) return { error: 'Hours must be a number, e.g. 1.5' };
	const hours = Number(value);
	if (hours <= 0) return { error: 'Hours must be greater than 0' };
	if (hours > 24) return { error: 'Hours cannot exceed 24' };
	return { hours };
}

export function isValidDate(ymd: string): boolean {
	if (!/^\d{4}-\d{2}-\d{2}$/.test(ymd)) return false;
	const [y, m, d] = ymd.split('-').map(Number);
	const date = new Date(y, m - 1, d);
	return date.getFullYear() === y && date.getMonth() === m - 1 && date.getDate() === d;
}

/** Moves a YYYY-MM-DD date by whole days in local time. */
export function shiftDate(ymd: string, days: number): string {
	const [y, m, d] = ymd.split('-').map(Number);
	const date = new Date(y, m - 1, d + days);
	const mm = String(date.getMonth() + 1).padStart(2, '0');
	const dd = String(date.getDate()).padStart(2, '0');
	return `${date.getFullYear()}-${mm}-${dd}`;
}

/** Index of the first id found in `preferred` order, else 0. */
export function initialIndex<T extends { id: number }>(items: readonly T[], ...preferred: (number | null | undefined)[]): number {
	for (const id of preferred) {
		if (id == null) continue;
		const index = items.findIndex((item) => item.id === id);
		if (index >= 0) return index;
	}
	return 0;
}
