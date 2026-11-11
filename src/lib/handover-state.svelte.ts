// Shared handover state: the ⌘K "Write handover" focus request. The Today page's
// handover cards react to each new `id`, and take a request left `pending` when they mount
// (the palette navigates to Today first).

export const handoverRequest = $state({ id: 0, pending: false });

/** Asks the Today page to focus today's handover editor. */
export function requestHandoverFocus(): void {
	handoverRequest.pending = true;
	handoverRequest.id += 1;
}

/** Takes a pending request (once); true when there was one. */
export function takeHandoverFocus(): boolean {
	const pending = handoverRequest.pending;
	handoverRequest.pending = false;
	return pending;
}
