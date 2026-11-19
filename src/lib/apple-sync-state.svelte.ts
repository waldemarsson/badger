// Whether Apple Reminders sync is on, kept current from Rust's status events. The root
// layout starts the watch; todo cards, reminder rows and the forms read `appleSync.on`
// to show the "Keep off phone" controls and "Mac only" marks.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppleSyncStatus } from '#lib/types.js';
import { APPLE_SYNC_STATUS_EVENT, syncShowsMacOnly } from '#lib/apple-sync.js';

export const appleSync = $state<{ on: boolean }>({ on: false });

let inFlight = false;
let again = false;

/** Re-reads the status; bursts of events coalesce into one trailing read. */
export async function refreshAppleSync(): Promise<void> {
	if (inFlight) {
		again = true;
		return;
	}
	inFlight = true;
	try {
		do {
			again = false;
			appleSync.on = syncShowsMacOnly(await invoke<AppleSyncStatus>('get_apple_sync_status'));
		} while (again);
	} catch {
		// Settings shows command errors; the controls stay as they were.
	} finally {
		inFlight = false;
	}
}

/** Loads the status and follows status events. Returns the cleanup. */
export function watchAppleSync(): () => void {
	let disposed = false;
	let unlisten: (() => void) | null = null;
	void refreshAppleSync();
	listen(APPLE_SYNC_STATUS_EVENT, () => void refreshAppleSync())
		.then((stop) => (disposed ? stop() : (unlisten = stop)))
		.catch(() => {});
	return () => {
		disposed = true;
		unlisten?.();
	};
}
