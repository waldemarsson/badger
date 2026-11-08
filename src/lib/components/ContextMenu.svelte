<script module lang="ts">
	import type Circle from '@lucide/svelte/icons/circle';

	export interface MenuItem {
		label: string;
		icon?: typeof Circle;
		/** Shortcut shown on the right, e.g. "F2". */
		hint?: string;
		/** Destructive: red, after a separator. */
		danger?: boolean;
		run: () => void;
	}
</script>

<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { placeMenu, type MenuPlacement } from '#lib/context-menu.js';

	// A right-click menu: fixed at a point, kept inside the window, with roving focus over
	// its items. The owner renders one at a time and decides where focus goes on close.

	const MARGIN = 8; // px kept clear of the window edges

	let {
		x,
		y,
		items,
		label,
		anchorHeight = 0,
		onclose
	}: {
		/** Where it opens: the pointer, or a row's bottom-left for keyboard opening. */
		x: number;
		y: number;
		items: MenuItem[];
		/** Accessible name, e.g. "Actions for Projects". */
		label: string;
		/** Height of the row a keyboard-opened menu sits under (flipping up clears it). */
		anchorHeight?: number;
		/** `refocus`: put focus back on what the menu was opened for (Esc, Tab, an item). */
		onclose: (refocus: boolean) => void;
	} = $props();

	let menuEl: HTMLElement;
	let pos = $state<MenuPlacement | null>(null); // null until measured (rendered hidden)
	let closed = false;

	/** Danger items get a separator above them (unless one already leads the list). */
	const separated = $derived(items.map((item, i) => !!item.danger && i > 0 && !items[i - 1].danger));

	function itemEls(): HTMLElement[] {
		return [...menuEl.querySelectorAll<HTMLElement>('[role="menuitem"]')];
	}

	/** Focuses item `index`, wrapping around both ends (-1: the last). */
	function focusItem(index: number): void {
		const els = itemEls();
		if (els.length) els[(index + els.length) % els.length].focus({ preventScroll: true });
	}

	function close(refocus: boolean): void {
		if (closed) return;
		closed = true;
		onclose(refocus);
	}

	/** Closes first (focus back on the row), so an action that moves focus keeps it. */
	function run(item: MenuItem): void {
		close(true);
		item.run();
	}

	function onkeydown(e: KeyboardEvent): void {
		if (e.isComposing) return;
		const index = itemEls().indexOf(document.activeElement as HTMLElement);
		switch (e.key) {
			case 'ArrowDown':
				focusItem(index + 1);
				break;
			case 'ArrowUp':
				focusItem(index <= 0 ? -1 : index - 1);
				break;
			case 'Home':
				focusItem(0);
				break;
			case 'End':
				focusItem(-1);
				break;
			case 'Enter':
			case ' ':
				if (items[index]) run(items[index]);
				break;
			case 'Escape':
			case 'Tab':
				close(true);
				break;
			default:
				return;
		}
		e.preventDefault();
		e.stopPropagation();
	}

	function onfocusout(e: FocusEvent): void {
		const to = e.relatedTarget;
		if (to instanceof Node && menuEl.contains(to)) return;
		// Focus went elsewhere (a shortcut, a click); a window switch keeps the row focused.
		close(!document.hasFocus());
	}

	function onWindowPointerDown(e: PointerEvent): void {
		if (!menuEl.contains(e.target as Node)) close(false);
	}

	const onWindowScroll = () => close(true);

	onMount(() => {
		pos = placeMenu(x, y, menuEl.offsetWidth, menuEl.offsetHeight, window.innerWidth, window.innerHeight, MARGIN, anchorHeight);
		void tick().then(() => {
			if (!closed) focusItem(0);
		});
		window.addEventListener('pointerdown', onWindowPointerDown, true);
		window.addEventListener('scroll', onWindowScroll, true); // capture: the tree's own scrolling
		return () => {
			window.removeEventListener('pointerdown', onWindowPointerDown, true);
			window.removeEventListener('scroll', onWindowScroll, true);
		};
	});
</script>

<svelte:window onblur={() => close(true)} onresize={() => close(true)} />

<!-- Its own clicks never move focus (WebKit doesn't focus clicked buttons); hover does. -->
<div
	bind:this={menuEl}
	role="menu"
	tabindex="-1"
	aria-label={label}
	aria-orientation="vertical"
	class="ctx-menu"
	style="left: {pos?.left ?? x}px; top: {pos?.top ?? y}px; transform-origin: {pos && pos.left < x ? 'right' : 'left'} {pos && pos.top < y ? 'bottom' : 'top'};"
	style:visibility={pos ? null : 'hidden'}
	in:scale={{ start: 0.96, duration: 150, easing: cubicOut }}
	{onkeydown}
	{onfocusout}
	onmousedown={(e) => e.preventDefault()}
	oncontextmenu={(e) => e.preventDefault()}
>
	{#each items as item, i (item.label)}
		{#if separated[i]}
			<div role="separator" class="ctx-menu-separator"></div>
		{/if}
		<button
			type="button"
			role="menuitem"
			tabindex="-1"
			class="ctx-menu-item"
			class:ctx-menu-item-danger={item.danger}
			onclick={() => run(item)}
			onpointermove={(e) => {
				if (document.activeElement !== e.currentTarget) e.currentTarget.focus({ preventScroll: true });
			}}
		>
			{#if item.icon}
				<item.icon size={15} class="ctx-menu-icon" />
			{/if}
			<span class="min-w-0 flex-1 truncate">{item.label}</span>
			{#if item.hint}
				<kbd class="ctx-menu-hint">{item.hint}</kbd>
			{/if}
		</button>
	{/each}
</div>
