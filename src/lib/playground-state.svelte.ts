// Playground run state, shared at module level so a run outlives the page: leave
// /playground mid-run and come back, and the run (with Stop) and every script's last output
// are still there. Output is kept for the session only. Transitions are the pure helpers in
// playground.ts; this module adds the Tauri calls.
import { invoke } from '@tauri-apps/api/core';
import { beginRun, emptyRunStore, endRun, noteRunId, resetOutputs, type RunStore } from '#lib/playground.js';
import type { RunResult } from '#lib/types.js';

export const playground = $state<RunStore>(emptyRunStore());

/** Runs `code` (the editor's current text) for `scriptId`; false when a run is in progress. */
export function runScript(scriptId: number, code: string): boolean {
	if (!beginRun(playground, scriptId, Date.now())) return false;
	invoke<RunResult>('run_script', { code })
		.then((ok) => {
			noteRunId(playground, ok.run_id);
			endRun(playground, { ok }, Date.now());
		})
		.catch((e) => endRun(playground, { failure: String(e) }, Date.now()));
	void learnRunId();
	return true;
}

/** Asks the backend for the id of the run in progress (Stop needs it). */
async function learnRunId(): Promise<number | null> {
	try {
		const id = await invoke<number | null>('script_running');
		if (id !== null && playground.running) noteRunId(playground, id);
		return id;
	} catch {
		return null;
	}
}

/** Stops the run in progress. A stale id is ignored by the backend, so this can't stop a later run. */
export async function stopRun(): Promise<void> {
	if (!playground.running) return;
	const id = playground.running.runId ?? (await learnRunId());
	if (id !== null) await invoke('stop_script', { runId: id }).catch(() => {});
}

let polling: ReturnType<typeof setInterval> | null = null;

/**
 * On page mount: a run the backend still has but this session lost track of (the WebView
 * reloaded) shows as running (with Stop) until it ends; its output has nowhere to go.
 */
export async function syncRunning(): Promise<void> {
	if (playground.running) return;
	let id: number | null;
	try {
		id = await invoke<number | null>('script_running');
	} catch {
		return;
	}
	if (id === null || playground.running) return;
	playground.running = { scriptId: null, runId: id, startedAt: Date.now() };
	if (polling !== null) return;
	polling = setInterval(() => {
		invoke<number | null>('script_running')
			.then((now) => {
				if (now === id) return;
				if (playground.running?.runId === id) playground.running = null;
				if (polling !== null) clearInterval(polling);
				polling = null;
			})
			.catch(() => {});
	}, 500);
}

/** Settings calls this after a successful restore: script ids may now mean other scripts. */
export function playgroundRestored(): void {
	resetOutputs(playground);
}
