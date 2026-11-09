// Pure key check for the task checkbox widget (no CodeMirror), so it can be unit-tested.

export interface TaskKey {
	key: string;
	metaKey: boolean;
	ctrlKey: boolean;
	altKey: boolean;
	shiftKey: boolean;
	isComposing: boolean;
}

/** Space or Enter, unmodified: toggles a focused task checkbox (like a native checkbox). */
export function togglesTask(e: TaskKey): boolean {
	if (e.isComposing || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
	return e.key === ' ' || e.key === 'Enter';
}
