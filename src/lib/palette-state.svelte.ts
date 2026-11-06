// Shared command palette state: the active shortcut, change signals for pages
// that show palette-created data, and the toast message.
import { DEFAULT_SHORTCUT } from './palette';

export const palette = $state({
	shortcut: DEFAULT_SHORTCUT,
	/** True while Settings records a new shortcut, so the palette ignores keys. */
	recording: false
});

/** Bumped after the palette creates data; pages showing it reload. */
export const changes = $state({ timeEntries: 0, todos: 0, notes: 0 });

export const toast = $state({ message: '', id: 0 });

/** Log time prefill: the palette selects this project and jumps to the hours step. */
export interface LogTimePrefill {
	projectId: number;
	issueId: number | null;
	issueTitle: string;
}

/** Pending "open the palette at Log time" request; the palette reacts to each new `id`. */
export const logTimeRequest = $state<{ id: number; prefill: LogTimePrefill | null }>({
	id: 0,
	prefill: null
});

/** Opens the palette's Log time flow, at the hours step when `prefill` resolves. */
export function requestLogTime(prefill: LogTimePrefill | null = null): void {
	logTimeRequest.prefill = prefill;
	logTimeRequest.id += 1;
}

export function showToast(message: string): void {
	toast.message = message;
	toast.id += 1;
}
