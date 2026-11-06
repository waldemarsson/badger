// Shared command palette state: the active shortcut, change signals for pages
// that show palette-created data, and the toast message.
import { DEFAULT_SHORTCUT } from './palette';

export const palette = $state({
	shortcut: DEFAULT_SHORTCUT,
	/** True while Settings records a new shortcut, so the palette ignores keys. */
	recording: false
});

/** Bumped after the palette creates data; pages showing it reload. */
export const changes = $state({ timeEntries: 0, todos: 0 });

export const toast = $state({ message: '', id: 0 });

export function showToast(message: string): void {
	toast.message = message;
	toast.id += 1;
}
