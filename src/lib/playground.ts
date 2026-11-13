// Pure helpers for the JS playground (no Svelte, no Tauri): the script list, the run
// store's transitions, console output shaping and the editor's completion data.
import type { OutputLevel, RunError, RunErrorKind, RunOutcome, RunResult, ScriptSummary } from '#lib/types.js';

// ── Script list ───────────────────────────────────────────────────────────────

/** Most recently changed first (then newest id), as `list_scripts` returns them. */
export function sortScripts(list: ScriptSummary[]): ScriptSummary[] {
	return [...list].sort((a, b) => (a.updated_at === b.updated_at ? b.id - a.id : a.updated_at < b.updated_at ? 1 : -1));
}

/** Adds or replaces `script`, keeping the list sorted. */
export function upsertScript(list: ScriptSummary[], script: ScriptSummary): ScriptSummary[] {
	const summary = { id: script.id, title: script.title, updated_at: script.updated_at };
	return sortScripts([...list.filter((s) => s.id !== script.id), summary]);
}

/** Scripts whose title contains every word of `query` (case-insensitive); all for a blank query. */
export function filterScripts(list: ScriptSummary[], query: string): ScriptSummary[] {
	const words = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (words.length === 0) return list;
	return list.filter((s) => {
		const title = s.title.toLowerCase();
		return words.every((w) => title.includes(w));
	});
}

/** The row to select after `id` is removed: the next one, else the previous, else null. */
export function neighbourAfterRemoval(ids: number[], id: number): number | null {
	const i = ids.indexOf(id);
	if (i === -1) return ids[0] ?? null;
	return ids[i + 1] ?? ids[i - 1] ?? null;
}

// ── Run store ─────────────────────────────────────────────────────────────────

/** One script's last result, kept for the session (never saved). */
export interface ScriptOutput {
	/** What the run produced; null when the run couldn't start (see `failure`). */
	outcome: RunOutcome | null;
	/** `run_script` itself failed (e.g. another run in progress). */
	failure: string | null;
	/** Epoch ms when the run finished. */
	at: number;
}

export interface ActiveRun {
	/** The script that started the run; null once it was deleted (its output is dropped). */
	scriptId: number | null;
	/** Known once `script_running` reported it; Stop needs it. */
	runId: number | null;
	startedAt: number;
}

export interface RunStore {
	running: ActiveRun | null;
	outputs: Record<number, ScriptOutput>;
}

export function emptyRunStore(): RunStore {
	return { running: null, outputs: {} };
}

/** Starts a run for `scriptId`; false when one is already in progress (one at a time). */
export function beginRun(store: RunStore, scriptId: number, now: number): boolean {
	if (store.running) return false;
	store.running = { scriptId, runId: null, startedAt: now };
	return true;
}

/** Records the backend's id for the run in progress. */
export function noteRunId(store: RunStore, runId: number): void {
	if (store.running && store.running.runId === null) store.running.runId = runId;
}

/**
 * Ends the run in progress with its result (or the error `run_script` failed with). The
 * output goes to the script that started the run, even if another one is shown now; a
 * deleted script's output is dropped.
 */
export function endRun(store: RunStore, result: { ok: RunResult } | { failure: string }, now: number): void {
	const run = store.running;
	store.running = null;
	if (!run || run.scriptId === null) return;
	store.outputs[run.scriptId] =
		'ok' in result ? { outcome: result.ok.outcome, failure: null, at: now } : { outcome: null, failure: result.failure, at: now };
}

/** A deleted script: its output goes, and a run it started keeps going but reports nowhere. */
export function forgetScript(store: RunStore, scriptId: number): void {
	delete store.outputs[scriptId];
	if (store.running?.scriptId === scriptId) store.running.scriptId = null;
}

export function clearOutput(store: RunStore, scriptId: number): void {
	delete store.outputs[scriptId];
}

/** After a restore every script id may mean something else: drop all output. */
export function resetOutputs(store: RunStore): void {
	store.outputs = {};
	if (store.running) store.running.scriptId = null;
}

// ── Output ────────────────────────────────────────────────────────────────────

export type ConsoleEntry =
	| { kind: 'line'; level: OutputLevel; text: string }
	| { kind: 'truncated' }
	| { kind: 'value'; text: string }
	| { kind: 'error'; title: string; message: string; line: number | null; col: number | null; errorKind: RunErrorKind };

const ERROR_TITLES: Record<RunErrorKind, string> = {
	user: 'Error',
	syntax: 'Syntax error',
	stack: 'Stack overflow',
	oom: 'Out of memory',
	timeout: 'Timed out',
	stopped: 'Stopped'
};

/** A short heading for an error kind. */
export function errorTitle(kind: RunErrorKind): string {
	return ERROR_TITLES[kind] ?? 'Error';
}

/** "Line 3, column 5", "Line 3", or null when the error has no position. */
export function errorLocation(error: Pick<RunError, 'line' | 'col'>): string | null {
	if (error.line === null) return null;
	return error.col === null ? `Line ${error.line}` : `Line ${error.line}, column ${error.col}`;
}

/**
 * The console pane's rows for a result: the logged lines, then the "Output truncated"
 * marker, then the last value, then the error (value and error are never cut by the cap).
 */
export function consoleEntries(output: ScriptOutput | null | undefined): ConsoleEntry[] {
	if (!output) return [];
	if (output.outcome === null) {
		return output.failure === null
			? []
			: [{ kind: 'error', title: 'Could not run', message: output.failure, line: null, col: null, errorKind: 'user' }];
	}
	const { lines, truncated, value, error } = output.outcome;
	const entries: ConsoleEntry[] = lines.map((l) => ({ kind: 'line', level: l.level, text: l.text }));
	if (truncated) entries.push({ kind: 'truncated' });
	if (value !== null) entries.push({ kind: 'value', text: value });
	if (error) {
		entries.push({ kind: 'error', title: errorTitle(error.kind), message: error.message, line: error.line, col: error.col, errorKind: error.kind });
	}
	return entries;
}

/** "Ran in 12 ms", "Ran in 1.3 s"; "Stopped after …" or "Failed after …" when it didn't finish. */
export function runSummary(outcome: RunOutcome): string {
	const ms = outcome.duration_ms;
	const time = ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`;
	if (!outcome.error) return `Ran in ${time}`;
	return outcome.error.kind === 'stopped' ? `Stopped after ${time}` : `Failed after ${time}`;
}

// ── Editor: completion data ───────────────────────────────────────────────────

export type Quote = '"' | "'" | '`';

/**
 * Inside the string argument of `note(…)`: where the title text starts (offset into
 * `before`, the line up to the caret), the quote that opened it, and what was typed.
 * Null anywhere else (also after an escape, which a title completion can't continue).
 */
export function noteTitleContext(before: string): { from: number; quote: Quote; typed: string } | null {
	const m = /(?:^|[^\w$.])note\(\s*(["'`])([^"'`\\\n]*)$/.exec(before);
	if (!m) return null;
	const typed = m[2];
	return { from: before.length - typed.length, quote: m[1] as Quote, typed };
}

/** `title` escaped for a string opened with `quote`. */
export function quoteTitle(title: string, quote: Quote): string {
	let out = title.replace(/\\/g, '\\\\').replace(new RegExp(quote, 'g'), `\\${quote}`);
	if (quote === '`') out = out.replace(/\$\{/g, '\\${');
	return out;
}

/** Globals the playground engine has (QuickJS: ECMAScript built-ins, no DOM, timers or network). */
export const PLAYGROUND_GLOBALS: { label: string; type: 'function' | 'variable' | 'class' | 'namespace'; info: string }[] = [
	{ label: 'note', type: 'function', info: 'note(title): a JSON note parsed, a markdown note as text' },
	{ label: 'console', type: 'namespace', info: 'log, info, warn, error, debug' },
	{ label: 'JSON', type: 'namespace', info: 'parse, stringify' },
	{ label: 'Math', type: 'namespace', info: 'Math functions and constants' },
	{ label: 'Object', type: 'class', info: 'keys, values, entries, fromEntries, groupBy…' },
	{ label: 'Array', type: 'class', info: 'from, isArray, of' },
	{ label: 'Map', type: 'class', info: 'Keyed collection' },
	{ label: 'Set', type: 'class', info: 'Unique values' },
	{ label: 'WeakMap', type: 'class', info: 'Object-keyed collection' },
	{ label: 'WeakSet', type: 'class', info: 'Object set' },
	{ label: 'Promise', type: 'class', info: 'all, allSettled, any, race, resolve, reject' },
	{ label: 'Number', type: 'class', info: 'parseFloat, parseInt, isInteger…' },
	{ label: 'String', type: 'class', info: 'fromCharCode, raw…' },
	{ label: 'Boolean', type: 'class', info: 'Boolean wrapper' },
	{ label: 'BigInt', type: 'function', info: 'Arbitrary-precision integers' },
	{ label: 'Symbol', type: 'function', info: 'Unique keys' },
	{ label: 'Date', type: 'class', info: 'Dates and times' },
	{ label: 'RegExp', type: 'class', info: 'Regular expressions' },
	{ label: 'Error', type: 'class', info: 'Throwable error' },
	{ label: 'TypeError', type: 'class', info: 'Error subtype' },
	{ label: 'RangeError', type: 'class', info: 'Error subtype' },
	{ label: 'SyntaxError', type: 'class', info: 'Error subtype' },
	{ label: 'Reflect', type: 'namespace', info: 'Object reflection' },
	{ label: 'Proxy', type: 'class', info: 'Object traps' },
	{ label: 'ArrayBuffer', type: 'class', info: 'Raw bytes' },
	{ label: 'Uint8Array', type: 'class', info: 'Byte array' },
	{ label: 'DataView', type: 'class', info: 'Typed views on bytes' },
	{ label: 'parseInt', type: 'function', info: 'parseInt(text, radix)' },
	{ label: 'parseFloat', type: 'function', info: 'parseFloat(text)' },
	{ label: 'isNaN', type: 'function', info: 'isNaN(value)' },
	{ label: 'isFinite', type: 'function', info: 'isFinite(value)' },
	{ label: 'encodeURIComponent', type: 'function', info: 'Percent-encode' },
	{ label: 'decodeURIComponent', type: 'function', info: 'Percent-decode' },
	{ label: 'queueMicrotask', type: 'function', info: 'Run a callback after the current job' },
	{ label: 'globalThis', type: 'variable', info: 'The global object' }
];

const MEMBERS: Record<string, string[]> = {
	console: ['log', 'info', 'warn', 'error', 'debug'],
	JSON: ['parse', 'stringify'],
	Math: ['abs', 'ceil', 'floor', 'round', 'trunc', 'sign', 'max', 'min', 'pow', 'sqrt', 'cbrt', 'log', 'log10', 'log2', 'exp', 'random', 'hypot', 'sin', 'cos', 'tan', 'atan2', 'PI', 'E'],
	Object: ['keys', 'values', 'entries', 'fromEntries', 'assign', 'groupBy', 'freeze', 'create', 'defineProperty', 'getPrototypeOf', 'hasOwn'],
	Array: ['from', 'isArray', 'of'],
	Map: ['groupBy'],
	Promise: ['all', 'allSettled', 'any', 'race', 'resolve', 'reject', 'withResolvers'],
	Number: ['isInteger', 'isFinite', 'isNaN', 'isSafeInteger', 'parseFloat', 'parseInt', 'MAX_SAFE_INTEGER', 'MIN_SAFE_INTEGER', 'EPSILON'],
	String: ['fromCharCode', 'fromCodePoint', 'raw'],
	Reflect: ['apply', 'construct', 'defineProperty', 'get', 'has', 'ownKeys', 'set']
};

/** Members of a curated global (`console.` → log, info, …); null for anything else. */
export function memberCompletions(object: string): string[] | null {
	return Object.hasOwn(MEMBERS, object) ? MEMBERS[object] : null;
}
