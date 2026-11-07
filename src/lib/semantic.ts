// Pure helpers for semantic (hybrid) note search: when to ask for hybrid results, how to
// order the two responses of one search, and how to describe the model status.
import type { SemanticStatus } from '#lib/types.js';

/** Tauri event sent whenever the semantic status changes (no payload). */
export const SEMANTIC_STATUS_EVENT = 'semantic-status';
/** Hybrid results follow the keyword results after this pause in typing. */
export const HYBRID_DEBOUNCE_MS = 250;
/** Matches Rust's MIN_SEMANTIC_CHARS: shorter text (tags removed) stays keyword-only. */
export const MIN_SEMANTIC_CHARS = 4;

/** Is the query text (with `#tag` terms already removed) long enough for the semantic leg? */
export function qualifiesForSemantic(text: string): boolean {
	return [...text.trim()].length >= MIN_SEMANTIC_CHARS;
}

/** A search response: which search it answers and whether it is the hybrid one. */
export interface SearchResponse {
	seq: number;
	hybrid: boolean;
}

/**
 * Should a response replace the shown results? Only responses to the latest search, and a
 * late keyword response never replaces the hybrid results of the same search.
 */
export function shouldApply(latestSeq: number, shown: SearchResponse | null, incoming: SearchResponse): boolean {
	if (incoming.seq !== latestSeq) return false;
	return !(shown && shown.seq === incoming.seq && shown.hybrid && !incoming.hybrid);
}

/**
 * The active row after a result list is replaced (e.g. hybrid hits arriving after the
 * keyword hits): the same item if it is still listed, otherwise the first row.
 */
export function keepActiveIndex<K>(before: readonly K[], after: readonly K[], active: number): number {
	if (active < 0 || active >= before.length) return 0;
	const index = after.indexOf(before[active]);
	return index >= 0 ? index : 0;
}

/** Whole percent of the download, 0–100. */
export function downloadPercent(status: Pick<SemanticStatus, 'downloaded_bytes' | 'download_bytes'>): number {
	if (status.download_bytes <= 0) return 0;
	return Math.min(100, Math.floor((status.downloaded_bytes / status.download_bytes) * 100));
}

/** Short status text for Settings. */
export function semanticStatusLabel(status: SemanticStatus): string {
	switch (status.state) {
		case 'off':
			return 'Off';
		case 'downloading':
			return `Downloading ${downloadPercent(status)}%`;
		case 'loading':
			return 'Loading model…';
		case 'indexing':
			return `Indexing ${status.indexing_done} of ${status.indexing_total}`;
		case 'ready':
			return 'Ready';
		case 'error':
			return 'Error';
	}
}

/** Searches can ask for hybrid results: the model is loaded (also while indexing). */
export function semanticReady(status: SemanticStatus | null): boolean {
	return status?.state === 'ready' || status?.state === 'indexing';
}

/** Show the "turn on semantic search" hint: off, or broken. Not while it is getting ready. */
export function semanticHintVisible(status: SemanticStatus | null): boolean {
	return status?.state === 'off' || status?.state === 'error';
}
