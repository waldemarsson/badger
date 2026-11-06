// Autosave queue for note bodies. Pure (no Svelte, no Tauri): the page passes the save
// call in and re-reads status from `onChange`.
//
// - Edits are debounced; the timer saves every dirty note, so an edit also retries
//   notes whose earlier save failed.
// - One save per note is in flight; edits made meanwhile queue and the latest body wins.
// - A body equal to the last saved body is never sent.
// - Failed saves keep the body dirty (with an error) until a later save succeeds.
// - A "Note not found" response, or any response for a forgotten note, is ignored.

export const SAVE_DELAY_MS = 800;

export type SaveStatus = 'saved' | 'pending' | 'saving' | 'error';

export interface SaveQueueOptions<T> {
	save: (id: number, body: string) => Promise<T>;
	delay?: number;
	/** Called after each successful save with the command's result. */
	onSaved?: (id: number, result: T) => void;
	/** Called whenever any note's status may have changed. */
	onChange?: () => void;
	/** True when the error means the note no longer exists. */
	isGone?: (error: unknown) => boolean;
}

export class NoteSaveQueue<T> {
	private readonly opts: SaveQueueOptions<T>;
	private readonly delay: number;
	/** Last body known to be stored, per note. */
	private readonly saved = new Map<number, string>();
	/** Latest body not yet stored (debouncing, queued behind a save, or failed). */
	private readonly dirty = new Map<number, string>();
	private readonly errors = new Map<number, string>();
	private readonly runners = new Map<number, Promise<void>>();
	/** Bumped by forget(), so responses for an older generation are dropped. */
	private readonly epochs = new Map<number, number>();
	private timer: ReturnType<typeof setTimeout> | null = null;

	constructor(opts: SaveQueueOptions<T>) {
		this.opts = opts;
		this.delay = opts.delay ?? SAVE_DELAY_MS;
	}

	/** Records the stored body of a note that was just loaded. */
	track(id: number, body: string): void {
		if (!this.runners.has(id) && !this.dirty.has(id)) this.saved.set(id, body);
	}

	/** Records an edit and (re)starts the debounce. */
	edit(id: number, body: string): void {
		if (!this.runners.has(id) && body === this.saved.get(id)) {
			// Back to the stored body (e.g. undo after a failed save): nothing left to save.
			this.dirty.delete(id);
			this.errors.delete(id);
		} else {
			this.dirty.set(id, body);
		}
		this.schedule();
		this.changed();
	}

	/** Saves every dirty note now; resolves true when no save failed. */
	async flush(): Promise<boolean> {
		this.clearTimer();
		const ids = new Set([...this.dirty.keys(), ...this.runners.keys()]);
		await Promise.all([...ids].map((id) => this.drain(id)));
		// Errors only exist on dirty notes (a success, revert or forget clears them), so this
		// is "everything flushed was stored". Edits typed during the flush are not failures.
		return this.errors.size === 0;
	}

	/** Drops a deleted note: cancels its pending save and ignores any late response. */
	forget(id: number): void {
		this.epochs.set(id, (this.epochs.get(id) ?? 0) + 1);
		this.saved.delete(id);
		this.dirty.delete(id);
		this.errors.delete(id);
		this.runners.delete(id);
		if (this.dirty.size === 0) this.clearTimer();
		this.changed();
	}

	status(id: number): SaveStatus {
		if (this.runners.has(id)) return 'saving';
		if (this.errors.has(id)) return 'error';
		if (this.dirty.has(id)) return 'pending';
		return 'saved';
	}

	error(id: number): string | null {
		return this.errors.get(id) ?? null;
	}

	failed(): { id: number; error: string }[] {
		return [...this.errors].map(([id, error]) => ({ id, error }));
	}

	/** True while any note has unsaved changes or a save in flight. */
	get busy(): boolean {
		return this.dirty.size > 0 || this.runners.size > 0;
	}

	/** Stops the debounce; unsaved bodies stay dirty until the next edit or flush. */
	pause(): void {
		this.clearTimer();
	}

	dispose(): void {
		this.pause();
	}

	private schedule(): void {
		this.clearTimer();
		this.timer = setTimeout(() => {
			this.timer = null;
			void this.flush();
		}, this.delay);
	}

	private clearTimer(): void {
		if (this.timer !== null) clearTimeout(this.timer);
		this.timer = null;
	}

	private changed(): void {
		this.opts.onChange?.();
	}

	private isGone(error: unknown): boolean {
		return this.opts.isGone ? this.opts.isGone(error) : String(error) === 'Note not found';
	}

	/** Saves the note's latest body until it is stored or a save fails. */
	private drain(id: number): Promise<void> {
		const running = this.runners.get(id);
		if (running) return running;
		if (!this.dirty.has(id)) return Promise.resolve();

		const epoch = this.epochs.get(id) ?? 0;
		const current = () => (this.epochs.get(id) ?? 0) === epoch;
		const runner = (async () => {
			for (;;) {
				const body = this.dirty.get(id);
				if (body === undefined) return;
				if (body === this.saved.get(id)) {
					this.dirty.delete(id);
					this.errors.delete(id);
					return;
				}
				try {
					const result = await this.opts.save(id, body);
					if (!current()) return;
					this.saved.set(id, body);
					if (this.dirty.get(id) === body) this.dirty.delete(id);
					this.errors.delete(id);
					this.opts.onSaved?.(id, result);
				} catch (error) {
					if (!current()) return;
					if (this.isGone(error)) {
						this.forget(id);
						return;
					}
					this.errors.set(id, String(error));
					return;
				} finally {
					this.changed();
				}
			}
		})().finally(() => {
			if (this.runners.get(id) === runner) this.runners.delete(id);
			this.changed();
		});
		this.runners.set(id, runner);
		this.changed();
		return runner;
	}
}
