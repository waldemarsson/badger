// Quit/close flush guard. Before the app exits, Rust emits FLUSH_REQUEST_EVENT and waits
// (up to 2 s) for `flush_done`. Pages with unsaved work register a handler that saves it;
// the layout runs all handlers, then replies.

export const FLUSH_REQUEST_EVENT = 'app://flush-request';

type FlushHandler = () => Promise<unknown>;

const handlers = new Set<FlushHandler>();

/** Registers a handler; returns the function that unregisters it. */
export function registerFlushHandler(handler: FlushHandler): () => void {
	handlers.add(handler);
	return () => {
		handlers.delete(handler);
	};
}

/** Runs every handler; failures are ignored (the reply must still go out). */
export async function runFlushHandlers(): Promise<void> {
	await Promise.allSettled([...handlers].map((handler) => handler()));
}
