// Shared semantic search status, kept current from Rust's status events. The root layout
// starts the watch; the notes page, the palette and Settings read `semantic.status`.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { SemanticStatus } from '#lib/types.js';
import { SEMANTIC_STATUS_EVENT } from '#lib/semantic.js';

export const semantic = $state<{ status: SemanticStatus | null }>({ status: null });

let inFlight = false;
let again = false;

/** Re-reads the status; bursts of events coalesce into one trailing read. */
export async function refreshSemanticStatus(): Promise<void> {
	if (inFlight) {
		again = true;
		return;
	}
	inFlight = true;
	try {
		do {
			again = false;
			semantic.status = await invoke<SemanticStatus>('get_semantic_status');
		} while (again);
	} catch {
		// Settings shows command errors; search falls back to keyword-only.
	} finally {
		inFlight = false;
	}
}

/** Loads the status and follows status events. Returns the cleanup. */
export function watchSemanticStatus(): () => void {
	let disposed = false;
	let unlisten: (() => void) | null = null;
	void refreshSemanticStatus();
	listen(SEMANTIC_STATUS_EVENT, () => void refreshSemanticStatus())
		.then((stop) => (disposed ? stop() : (unlisten = stop)))
		.catch(() => {});
	return () => {
		disposed = true;
		unlisten?.();
	};
}
