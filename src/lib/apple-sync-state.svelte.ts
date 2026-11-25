// Whether Apple Reminders sync is on (and its last status), kept current from Rust's status
// events. The root layout starts the watch; task rows and the details panel read
// `appleSync.on` to show the "Keep off phone" switch and "Mac only" chips, and /tasks shows
// the last sync beside its totals.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppleSyncStatus } from '#lib/types.js';
import { APPLE_SYNC_STATUS_EVENT, syncShowsMacOnly } from '#lib/apple-sync.js';

export const appleSync = $state<{ on: boolean; status: AppleSyncStatus | null }>({ on: false, status: null });

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
			const status = await invoke<AppleSyncStatus>('get_apple_sync_status');
			appleSync.status = status;
			appleSync.on = syncShowsMacOnly(status);
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
