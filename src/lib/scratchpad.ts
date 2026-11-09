// Pure helpers for the scratchpad drawer (no Svelte, no Tauri).

/** The editor and the save queue key bodies by note id; the pad uses this pseudo-id. */
export const PAD_ID = 0;

/** Empty or whitespace-only: Save as note, Copy as HTML and Clear are disabled. */
export function isBlank(text: string): boolean {
	return text.trim() === '';
}

export interface PanelKey {
	key: string;
	code: string;
	metaKey: boolean;
	ctrlKey: boolean;
	altKey: boolean;
	shiftKey: boolean;
	repeat: boolean;
	defaultPrevented: boolean;
	isComposing: boolean;
}

/**
 * What a keydown inside the drawer does: Esc closes it unless something inside already
 * handled it (slash/autocomplete menus, the title prompt); ⌘E toggles the read view
 * (`repeat` while held: swallow it, don't toggle again). Anything else: null.
 */
export function panelKeyAction(e: PanelKey): 'close' | 'toggle-read' | 'repeat' | null {
	const plain = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
	if (e.key === 'Escape') return plain && !e.defaultPrevented && !e.isComposing ? 'close' : null;
	if (e.code === 'KeyE' && e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) return e.repeat ? 'repeat' : 'toggle-read';
	return null;
}
