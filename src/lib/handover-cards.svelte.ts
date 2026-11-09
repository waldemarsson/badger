// Shared state for the Today page's two handover cards, together at the bottom of the main
// column: the incoming/history card (HandoverIncoming.svelte), then today's editor
// (HandoverToday.svelte). The page creates one `HandoverCards` and passes it to both, so
// they share one list and one autosave queue keyed by date (handoverKey): a save always
// lands on the date the text was written for. The queue flushes on blur, focus leaving a
// card, either card unmounting, a day change and quit.
import { invoke } from '@tauri-apps/api/core';
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { tick } from 'svelte';
import { NoteSaveQueue, type SaveStatus } from '#lib/note-save-queue.js';
import { linkData, type LinkData } from '#lib/editor/links.js';
import { handoverKey, isBlank, keyDate, selectIncoming } from '#lib/handovers.js';
import type { LinkFollow } from '#lib/note-links.js';
import { showToast } from '#lib/palette-state.svelte.js';
import type { Handover, LinkTargets, Note, TagSuggestion } from '#lib/types.js';

/** What the cards need from a NoteEditor instance (bound with bind:this). */
export interface HandoverEditor {
	open(id: number, body: string): void;
	focus(): void;
	focusEnd(): void;
	setEditable(editable: boolean): void;
	setLinks(data: LinkData): void;
	text(): string;
}

/** The last unmount's flush: a quick remount waits for it, so it never loads stale text. */
let unmountFlush: Promise<unknown> = Promise.resolve();

export class HandoverCards {
	/** Every handover, newest first. */
	list = $state<Handover[]>([]);
	loadError = $state('');
	/** True once today's editor shows the stored text (typing before that could overwrite it). */
	editorReady = $state(false);
	/** The date today's editor writes to: captured when it opens, switched only after a flush. */
	editorDate = $state('');
	/** Today's live text (edits included). */
	text = $state('');
	/** Index into `past` while browsing history; null shows the incoming card (if any). */
	browsing = $state<number | null>(null);
	dismissing = $state(false);
	pastError = $state('');
	todayError = $state('');
	savingNote = $state<'today' | 'past' | null>(null);

	todayEditor = $state<HandoverEditor>();
	pastEditor = $state<HandoverEditor>();
	todaySection = $state<HTMLElement>();
	pastSection = $state<HTMLElement>();

	readonly blank = $derived(isBlank(this.text));
	readonly todayKey = $derived(handoverKey(this.editorDate));
	readonly past = $derived(this.list.filter((h) => h.date < this.editorDate));
	readonly incoming = $derived(selectIncoming(this.past, this.editorDate));
	readonly historyOpen = $derived(this.browsing !== null);
	readonly shown = $derived(this.browsing === null ? this.incoming : (this.past[this.browsing] ?? null));
	readonly shownIndex = $derived(this.browsing ?? 0);

	links: LinkData | null = null;

	private queueVersion = $state(0);
	readonly queue = new NoteSaveQueue<Handover | null>({
		save: (key, body) => invoke<Handover | null>('save_handover', { date: keyDate(key), body }),
		onSaved: (key, result) => this.applySaved(keyDate(key), result),
		onChange: () => this.queueVersion++,
		isGone: () => false
	});

	private loadSeq = 0;
	/** Loading and opening today's editor; ⌘K focus waits for it. */
	ready: Promise<void> = Promise.resolve();
	private switching: Promise<void> = Promise.resolve();
	/** The latest date the page reported (switches queue up behind each other). */
	private day: string;

	/** Resolves once the page's own first load settled (success or failure); see pageSettled. */
	private settled: Promise<void>;
	private settle: () => void = () => {};

	constructor(day: string) {
		this.settled = new Promise((resolve) => (this.settle = resolve));
		this.day = day;
		this.editorDate = day;
		this.ready = this.start();
	}

	/**
	 * The page calls this once its first load (todos, time entries) settled, whether it
	 * worked or not: until then the sections above the cards can still grow and push them.
	 */
	pageSettled(): void {
		this.settle();
	}

	status(key: number): SaveStatus {
		void this.queueVersion;
		return this.queue.status(key);
	}

	error(key: number): string | null {
		void this.queueVersion;
		return this.queue.error(key);
	}

	flush(): Promise<boolean> {
		return this.queue.flush();
	}

	/** A card unmounted: flush, and make the next mount's load wait for it. */
	flushOnUnmount(): void {
		unmountFlush = this.queue.flush();
	}

	/** Keeps `list` in step with what is stored (null: the day's handover was deleted). */
	applySaved(date: string, result: Handover | null): void {
		if (result === null) this.list = this.list.filter((h) => h.date !== date);
		else if (this.list.some((h) => h.date === date)) this.list = this.list.map((h) => (h.date === date ? result : h));
		else this.list = [...this.list, result].sort((a, b) => b.date.localeCompare(a.date));
	}

	onTodayChange(key: number, body: string): void {
		if (key === this.todayKey) this.text = body;
		this.queue.edit(key, body);
	}

	/** A task checkbox ticked on the incoming or history card. */
	onPastChange(key: number, body: string): void {
		this.queue.edit(key, body);
	}

	// ── Loading and the day ───────────────────────────────────────────────────────

	private async load(): Promise<boolean> {
		const seq = ++this.loadSeq;
		try {
			const all = await invoke<Handover[]>('list_handovers');
			if (seq !== this.loadSeq) return false;
			for (const h of all) this.queue.track(handoverKey(h.date), h.body);
			this.list = all;
			this.loadError = '';
			return true;
		} catch (e) {
			if (seq === this.loadSeq) this.loadError = String(e);
			return false;
		}
	}

	/** Shows the stored text of `editorDate` in today's editor (its cached edits win). */
	private async openToday(): Promise<void> {
		await tick();
		const key = this.todayKey;
		const body = this.list.find((h) => h.date === this.editorDate)?.body ?? '';
		this.queue.track(key, body);
		this.todayEditor?.open(key, body);
		this.text = this.todayEditor?.text() ?? body;
		this.editorReady = true;
	}

	/** Loads, then opens today's editor; never opens it over text it couldn't read. */
	private async start(): Promise<void> {
		await unmountFlush;
		if (await this.load()) await this.openToday();
	}

	retry(): void {
		this.ready = this.start();
	}

	/**
	 * The page noticed a new local date (focus or visibility after midnight): save what was
	 * typed to the old date first, then switch the editor to the new one.
	 */
	setDay(next: string): void {
		this.day = next;
		if (next !== this.editorDate) this.switching = this.switching.then(() => this.switchDay(next));
	}

	private async switchDay(next: string): Promise<void> {
		if (next !== this.day || next === this.editorDate) return; // superseded by a later switch
		this.editorReady = false;
		await this.queue.flush();
		this.editorDate = next;
		this.browsing = null;
		this.ready = this.start();
		await this.ready;
	}

	// ── Incoming and history ──────────────────────────────────────────────────────

	toggleHistory(): void {
		this.pastError = '';
		this.browsing = this.browsing === null ? 0 : null;
	}

	older(): void {
		if (this.shownIndex + 1 < this.past.length) this.browsing = this.shownIndex + 1;
	}

	newer(): void {
		if (this.browsing !== null && this.browsing > 0) this.browsing -= 1;
	}

	async dismiss(): Promise<void> {
		const h = this.shown;
		if (!h || this.dismissing) return;
		this.dismissing = true;
		this.pastError = '';
		try {
			// A checkbox save still in flight must not land after (and undo) the dismissal.
			await this.queue.flush();
			const focused = this.pastSection?.contains(document.activeElement) ?? false;
			this.applySaved(h.date, await invoke<Handover>('dismiss_handover', { date: h.date }));
			// The card (and the focused Dismiss button) fades out: keep focus on the handovers
			// (today's editor, just below), without scrolling the page.
			if (focused) this.focusWithoutScroll();
		} catch (e) {
			this.pastError = `Couldn't dismiss: ${e}`;
		} finally {
			this.dismissing = false;
		}
	}

	/** Copies the stored handover into a root note `Handover YYYY-MM-DD`; the handover stays. */
	async saveAsNote(date: string, which: 'today' | 'past'): Promise<void> {
		if (this.savingNote) return;
		this.savingNote = which;
		// Lock today's text until the copy commits, so nothing typed after the flush is left out.
		if (which === 'today') this.todayEditor?.setEditable(false);
		this.todayError = '';
		this.pastError = '';
		const fail = (message: string) => {
			if (which === 'today') this.todayError = message;
			else this.pastError = message;
		};
		try {
			if (!(await this.queue.flush())) {
				fail("Couldn't save the handover, so it wasn't copied to a note. Try again.");
				return;
			}
			const note = await invoke<Note>('save_handover_as_note', { date });
			showToast(`Saved as “${note.title}”`);
		} catch (e) {
			fail(String(e));
		} finally {
			this.savingNote = null;
			if (which === 'today') this.todayEditor?.setEditable(true);
		}
	}

	private focusWithoutScroll(): void {
		const { scrollX, scrollY } = window;
		this.todayEditor?.focus();
		window.scrollTo(scrollX, scrollY);
	}

	// ── ⌘K, links ────────────────────────────────────────────────────────────────

	/**
	 * Scrolls today's editor into view and puts the caret at the end, once the page's
	 * sections above it have loaded (so they can't push it off-screen afterwards).
	 */
	async focusToday(): Promise<void> {
		await this.switching;
		await this.ready;
		await this.settled;
		await tick();
		if (this.editorReady) this.todayEditor?.focusEnd();
		// After CodeMirror's own caret scroll (it runs in its next measure frame), so that one
		// can't cut the smooth scroll short and leave the card half off-screen.
		await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
		this.todaySection?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
	}

	/** What wikilinks and chips resolve to. */
	async refreshLinks(): Promise<void> {
		try {
			const [targets, suggestions] = await Promise.all([
				invoke<LinkTargets>('get_link_targets'),
				invoke<TagSuggestion[]>('list_tag_suggestions')
			]);
			this.links = linkData(targets, suggestions);
			this.todayEditor?.setLinks(this.links);
			this.pastEditor?.setLinks(this.links);
		} catch {
			// Links render unresolved; writing still works.
		}
	}

	/**
	 * ⌘-click in either card. A resolved [[wikilink]] opens its note on /notes through the
	 * same `?id=` deep link the palette uses, once pending edits are saved (if that fails the
	 * card shows why and stays). Tags, #todo/#issue chips and unresolved wikilinks aren't
	 * followable here (the editors use followScope 'notes'): /notes only follows them in place.
	 */
	async follow(link: LinkFollow, which: 'today' | 'past'): Promise<void> {
		if (link.kind !== 'note' || link.id === null) return;
		const id = link.id;
		if (which === 'today') this.todayError = '';
		else this.pastError = '';
		if (!(await this.queue.flush())) {
			const message = "Couldn't save the handover, so the link wasn't opened. Try again.";
			if (which === 'today') this.todayError = message;
			else this.pastError = message;
			return;
		}
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
		await goto(`${resolve('/notes')}?id=${id}`);
	}
}
