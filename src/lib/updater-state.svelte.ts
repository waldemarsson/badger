// Shared update status. The root layout checks once on launch (release builds only) and
// marks the Settings gear; Settings checks on demand and installs.
import { invoke } from '@tauri-apps/api/core';
import type { UpdateInfo } from '#lib/types.js';

export const updater = $state<{ available: UpdateInfo | null }>({ available: null });

/** Asks Rust for a newer release and records the answer. Throws the user-facing error. */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
	const update = await invoke<UpdateInfo | null>('check_for_update');
	updater.available = update;
	return update;
}
