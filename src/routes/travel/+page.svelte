<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { fade } from 'svelte/transition';
	import { invoke } from '@tauri-apps/api/core';
	import type { TravelRow } from '#lib/types.js';
	import { MONTH_NAMES, formatNumber, tripTotals } from '#lib/travel.js';
	import MonthNav from '#lib/components/MonthNav.svelte';
	import Check from 'lucide-svelte/icons/check';
	import X from 'lucide-svelte/icons/x';
	import Pencil from 'lucide-svelte/icons/pencil';
	import Trash2 from 'lucide-svelte/icons/trash-2';
	import Repeat from 'lucide-svelte/icons/repeat';

	// Number inputs bind to undefined/null when empty.
	type Draft = { date: string; from: string; to: string; km: number | null | undefined; amount: number | null | undefined };

	const pad = (n: number) => String(n).padStart(2, '0');
	const isoDate = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
	const optional = (n: number | null | undefined) => (n == null || Number.isNaN(n) ? null : n);

	const now = new Date();
	let year = $state(now.getFullYear());
	let month = $state(now.getMonth() + 1);
	let trips = $state<TravelRow[]>([]);
	let places = $state<string[]>([]);
	let loading = $state(false);
	let loadError = $state('');

	let draft = $state<Draft>(emptyDraft());
	let newError = $state('');
	let newSaving = $state(false);
	let newDateInput = $state<HTMLInputElement>();

	let editId = $state<number | null>(null);
	let edit = $state<Draft>(emptyDraft());
	let editError = $state('');
	let editSaving = $state(false);

	let confirmDeleteId = $state<number | null>(null);
	let deleteError = $state('');

	const label = $derived(`${MONTH_NAMES[month - 1]} ${year}`);
	const totals = $derived(tripTotals(trips));

	/** Today when viewing the current month, otherwise the 1st of the viewed month. */
	function defaultDate(): string {
		const today = new Date();
		return today.getFullYear() === year && today.getMonth() + 1 === month
			? isoDate(today)
			: `${year}-${pad(month)}-01`;
	}

	function emptyDraft(): Draft {
		return { date: defaultDate(), from: '', to: '', km: null, amount: null };
	}

	function payload(d: Draft) {
		return { date: d.date, fromPlace: d.from, toPlace: d.to, km: optional(d.km), amount: optional(d.amount) };
	}

	async function load() {
		loading = true;
		try {
			[trips, places] = await Promise.all([
				invoke<TravelRow[]>('list_travel', { year, month }),
				invoke<string[]>('list_travel_places'),
			]);
			loadError = '';
		} catch (e) {
			loadError = String(e);
		} finally {
			loading = false;
		}
	}

	/** Shows the month a trip was saved in, so it never disappears from view. */
	async function showMonthOf(date: string) {
		const [y, m] = date.split('-').map(Number);
		if (y !== year || m !== month) {
			year = y;
			month = m;
		}
		await load();
	}

	function onMonthChange() {
		editId = null;
		confirmDeleteId = null;
		resetNew();
		load();
	}

	onMount(load);

	// ── Add ────────────────────────────────────────────────────────────────
	function resetNew() {
		draft = emptyDraft();
		newError = '';
	}

	async function saveNew() {
		if (newSaving) return;
		newSaving = true;
		newError = '';
		try {
			const saved = await invoke<TravelRow>('create_travel', payload(draft));
			await showMonthOf(saved.date);
			resetNew();
		} catch (e) {
			newError = String(e);
		} finally {
			newSaving = false;
		}
	}

	async function repeatTrip(trip: TravelRow) {
		draft = { date: isoDate(new Date()), from: trip.from_place, to: trip.to_place, km: trip.km, amount: trip.amount };
		newError = '';
		await tick();
		newDateInput?.focus();
	}

	// ── Edit ───────────────────────────────────────────────────────────────
	function startEdit(trip: TravelRow) {
		confirmDeleteId = null;
		editId = trip.id;
		edit = { date: trip.date, from: trip.from_place, to: trip.to_place, km: trip.km, amount: trip.amount };
		editError = '';
	}

	function cancelEdit() {
		editId = null;
		editError = '';
	}

	async function saveEdit() {
		if (editId === null || editSaving) return;
		editSaving = true;
		editError = '';
		try {
			const saved = await invoke<TravelRow>('update_travel', { id: editId, ...payload(edit) });
			editId = null;
			await showMonthOf(saved.date);
		} catch (e) {
			editError = String(e);
		} finally {
			editSaving = false;
		}
	}

	// ── Delete ─────────────────────────────────────────────────────────────
	async function deleteTrip(id: number) {
		deleteError = '';
		try {
			await invoke('delete_travel', { id });
			confirmDeleteId = null;
			await load();
		} catch (e) {
			deleteError = String(e);
		}
	}

	function rowKeys(save: () => void, cancel: () => void) {
		return (e: KeyboardEvent) => {
			if (e.key === 'Enter') {
				e.preventDefault();
				save();
			} else if (e.key === 'Escape') {
				cancel();
			}
		};
	}

	const fmt = (n: number | null) => (n === null ? '—' : formatNumber(n));
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (confirmDeleteId = null)} />

<datalist id="travel-places">
	{#each places as place (place)}
		<option value={place}></option>
	{/each}
</datalist>

{#snippet fields(d: Draft, onkeydown: (e: KeyboardEvent) => void, dateRef: boolean)}
	<td>
		{#if dateRef}
			<input type="date" bind:value={d.date} bind:this={newDateInput} {onkeydown} class="form-input w-full" aria-label="Date" />
		{:else}
			<input type="date" bind:value={d.date} {onkeydown} class="form-input w-full" aria-label="Date" />
		{/if}
	</td>
	<td><input type="text" bind:value={d.from} {onkeydown} list="travel-places" placeholder="From…" class="form-input w-full" aria-label="From" /></td>
	<td><input type="text" bind:value={d.to} {onkeydown} list="travel-places" placeholder="To…" class="form-input w-full" aria-label="To" /></td>
	<td><input type="number" step="any" min="0" bind:value={d.km} {onkeydown} placeholder="km" class="form-input w-full text-right" aria-label="Km" /></td>
	<td><input type="number" step="any" min="0" bind:value={d.amount} {onkeydown} placeholder="kr" class="form-input w-full text-right" aria-label="Amount (kr)" /></td>
{/snippet}

<div class="page-shell">
	<div class="page-view">
		<section class="page-hero">
			<p class="page-hero-kicker">TimeTracker</p>
			<h1 class="page-hero-title">Travel</h1>
			<p class="page-hero-copy">Log business trips by month.</p>
			<div class="mt-4 flex flex-wrap gap-3">
				<div class="hero-stat">
					<span class="hero-stat-label">Trips in {MONTH_NAMES[month - 1]}</span>
					<div class="hero-stat-value">{trips.length}</div>
				</div>
				<div class="hero-stat">
					<span class="hero-stat-label">Distance</span>
					<div class="hero-stat-value">{formatNumber(totals.km)} km</div>
				</div>
				<div class="hero-stat">
					<span class="hero-stat-label">Amount</span>
					<div class="hero-stat-value">{formatNumber(totals.amount)} kr</div>
				</div>
			</div>
		</section>

		<section class="table-card">
			<div class="table-toolbar flex items-center justify-between gap-3">
				<MonthNav bind:year bind:month disabled={loading} onchange={onMonthChange} />
			</div>

			{#if loadError}<div class="stack-error mx-4 mt-3">{loadError}</div>{/if}
			{#if deleteError}<div class="stack-error mx-4 mt-3">{deleteError}</div>{/if}

			<div class="table-scroll-region">
				<table class="table-base">
					<thead class="table-head">
						<tr>
							<th class="table-heading w-36">Date</th>
							<th class="table-heading">From</th>
							<th class="table-heading">To</th>
							<th class="table-heading-right w-28">Km</th>
							<th class="table-heading-right w-32 whitespace-nowrap">Amount (kr)</th>
							<th class="table-heading-right w-32">Actions</th>
						</tr>
					</thead>
					<tbody>
						<tr class="table-row-new">
							{@render fields(draft, rowKeys(saveNew, resetNew), true)}
							<td>
								<div class="flex justify-end gap-1">
									<button onclick={saveNew} disabled={newSaving} title="Add trip" aria-label="Add trip" class="btn-icon-confirm">
										<Check size={14} />
									</button>
									<button onclick={resetNew} title="Clear" aria-label="Clear" class="btn-icon-danger">
										<X size={14} />
									</button>
								</div>
							</td>
						</tr>
						{#if newError}
							<tr class="table-row-new">
								<td colspan="6" class="!pt-0"><div class="stack-error text-xs">{newError}</div></td>
							</tr>
						{/if}

						{#each trips as trip (trip.id)}
							{#if editId === trip.id}
								<tr class="table-row-editing">
									{@render fields(edit, rowKeys(saveEdit, cancelEdit), false)}
									<td>
										<div class="flex justify-end gap-1">
											<button onclick={saveEdit} disabled={editSaving} title="Save changes" aria-label="Save changes" class="btn-icon-confirm">
												<Check size={14} />
											</button>
											<button onclick={cancelEdit} title="Cancel" aria-label="Cancel" class="btn-icon-danger">
												<X size={14} />
											</button>
										</div>
									</td>
								</tr>
								{#if editError}
									<tr class="table-row-editing">
										<td colspan="6" class="!pt-0"><div class="stack-error text-xs">{editError}</div></td>
									</tr>
								{/if}
							{:else}
								<tr class="table-row-history">
									<td class="table-cell-strong">{trip.date}</td>
									<td class="table-cell">{trip.from_place}</td>
									<td class="table-cell">{trip.to_place}</td>
									<td class="table-cell text-right">{fmt(trip.km)}</td>
									<td class="table-cell text-right">{fmt(trip.amount)}</td>
									<td class="table-cell">
										{#if confirmDeleteId === trip.id}
											<div class="flex items-center justify-end gap-1" in:fade={{ duration: 150 }}>
												<span class="mr-1 text-xs font-medium text-red-700 dark:text-red-400">Delete?</span>
												<button onclick={() => deleteTrip(trip.id)} class="btn-danger-sm">Delete</button>
												<button onclick={() => (confirmDeleteId = null)} class="btn-secondary-sm">Cancel</button>
											</div>
										{:else}
											<div class="flex justify-end gap-1">
												<button onclick={() => repeatTrip(trip)} title="Repeat trip today" aria-label="Repeat trip today" class="btn-icon-secondary">
													<Repeat size={14} />
												</button>
												<button onclick={() => startEdit(trip)} title="Edit" aria-label="Edit" class="btn-icon-secondary">
													<Pencil size={14} />
												</button>
												<button
													onclick={() => {
														deleteError = '';
														confirmDeleteId = trip.id;
													}}
													title="Delete"
													aria-label="Delete"
													class="btn-icon-danger"
												>
													<Trash2 size={14} />
												</button>
											</div>
										{/if}
									</td>
								</tr>
							{/if}
						{/each}

						{#if trips.length === 0 && !loading}
							<tr>
								<td colspan="6" class="px-4 py-6">
									<div class="empty-state">
										<p class="font-semibold text-slate-700 dark:text-neutral-300">No trips</p>
										<p>No travel logged for {label}. Add one in the row above.</p>
									</div>
								</td>
							</tr>
						{:else if trips.length > 0}
							<tr class="border-t-2 border-slate-300 dark:border-neutral-700">
								<td class="table-cell-strong" colspan="3">Total</td>
								<td class="table-cell-strong text-right">{formatNumber(totals.km)}</td>
								<td class="table-cell-strong text-right">{formatNumber(totals.amount)}</td>
								<td></td>
							</tr>
						{/if}
					</tbody>
				</table>
			</div>
		</section>
	</div>
</div>
