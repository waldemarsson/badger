<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { WeeklyReportRow } from '#lib/types.js';
  import {
    dayLabelsForWeek,
    getCurrentIsoWeek,
    getMondayIsoWeek,
    isoWeekToMonday,
    weekLabel
  } from '#lib/weekly-report.js';
  import ChevronLeft  from 'lucide-svelte/icons/chevron-left';
  import ChevronRight from 'lucide-svelte/icons/chevron-right';

  // ── Hours display ──────────────────────────────────────────────────────
  function formatHours(h: number): string {
    const r = Math.round(h * 10000) / 10000;
    return Number.isInteger(r) ? String(Math.round(r)) : String(r);
  }

  // ── State ──────────────────────────────────────────────────────────────
  const init       = getCurrentIsoWeek();
  let isoYear      = $state(init.year);
  let isoWeek      = $state(init.week);
  let rows         = $state<WeeklyReportRow[]>([]);
  let loading      = $state(false);

  // ── Derived ────────────────────────────────────────────────────────────
  const label = $derived(weekLabel(isoYear, isoWeek));

  // Ordered list of [label, fieldName] for the 7 day columns — label computed per week
  const DAYS: [string, keyof WeeklyReportRow][] = [
    ['Mon', 'mon'], ['Tue', 'tue'], ['Wed', 'wed'], ['Thu', 'thu'],
    ['Fri', 'fri'], ['Sat', 'sat'], ['Sun', 'sun'],
  ];

  /** Returns the date for each day column as "Mon\n30" style labels */
  const dayLabels = $derived((() => {
    return dayLabelsForWeek(isoYear, isoWeek, DAYS.map(([name]) => name));
  })());

  // Day-column totals (sum of all rows per day, ignoring adjacent days)
  const dayTotals = $derived(
    DAYS.map(([, field]) =>
      rows.reduce((acc, r) => acc + (r[field] as number), 0)
    )
  );
  const grandTotal = $derived(rows.reduce((a, r) => a + r.total, 0));

  // Group rows: Customer → Project → Issue
  type ProjectGroup = { projectId: number; projectName: string; rows: WeeklyReportRow[] };
  type CustomerGroup = { customerId: number; customerName: string; projects: ProjectGroup[] };

  const grouped = $derived((() => {
    const customers: CustomerGroup[] = [];
    for (const row of rows) {
      let customer = customers.find((entry) => entry.customerId === row.customer_id);
      if (!customer) {
        customer = { customerId: row.customer_id, customerName: row.customer_name, projects: [] };
        customers.push(customer);
      }
      let proj = customer.projects.find(p => p.projectId === row.project_id);
      if (!proj) {
        proj = { projectId: row.project_id, projectName: row.project_name, rows: [] };
        customer.projects.push(proj);
      }
      proj.rows.push(row);
    }
    return customers;
  })());

  // ── Data loading ───────────────────────────────────────────────────────
  async function loadWeek() {
    loading = true;
    try {
      rows = await invoke<WeeklyReportRow[]>('weekly_report', { year: isoYear, week: isoWeek });
    } finally {
      loading = false;
    }
  }

  onMount(() => loadWeek());

  function prevWeek() {
    const monday = isoWeekToMonday(isoYear, isoWeek);
    monday.setDate(monday.getDate() - 7);
    const { year, week } = getMondayIsoWeek(monday);
    isoYear = year;
    isoWeek = week;
    loadWeek();
  }

  function nextWeek() {
    const monday = isoWeekToMonday(isoYear, isoWeek);
    monday.setDate(monday.getDate() + 7);
    const { year, week } = getMondayIsoWeek(monday);
    isoYear = year;
    isoWeek = week;
    loadWeek();
  }

  // Cell display helper: returns "" (blank), "—", or the formatted hours string
  function cellValue(row: WeeklyReportRow, field: keyof WeeklyReportRow): string {
    const h = row[field] as number;
    if (h > 0) return formatHours(h);
    const dayName = (field as string);
    if (row.adjacent_days.includes(dayName)) return '—';
    return '';
  }

  function cellClass(row: WeeklyReportRow, field: keyof WeeklyReportRow): string {
    const dayName = (field as string);
    const h = row[field] as number;
    if (h === 0 && row.adjacent_days.includes(dayName)) return 'table-cell-muted text-right';
    return 'table-cell text-right text-slate-950 dark:text-neutral-100';
  }
</script>

<div class="page-shell">
  <div class="page-view">

    <section class="page-hero">
      <h1 class="page-hero-title">Weekly Report</h1>
    </section>

    <div class="table-card">
      <!-- Week navigator toolbar -->
      <div class="table-toolbar flex items-center gap-3">
        <button class="btn-icon-secondary" onclick={prevWeek} disabled={loading} title="Previous week">
          <ChevronLeft size={16} />
        </button>
        <span class="min-w-[16rem] text-center text-sm font-semibold text-slate-700 dark:text-neutral-300">
          {label}
        </span>
        <button class="btn-icon-secondary" onclick={nextWeek} disabled={loading} title="Next week">
          <ChevronRight size={16} />
        </button>
      </div>

      <div class="table-scroll-region">
        {#if rows.length === 0 && !loading}
          <div class="px-4 py-6">
            <div class="empty-state">
              <p class="font-semibold text-slate-700 dark:text-neutral-300">No entries this week</p>
              <p>No time was logged for {label}.</p>
            </div>
          </div>
        {:else}
          <table class="table-base">
            <thead class="table-head">
              <tr>
                <th class="table-heading min-w-[12rem] flex-1">Issue</th>
                {#each dayLabels as col (col.name)}
                  <th class="table-heading-right w-20 leading-tight">
                    <span class="block">{col.name}</span>
                    <span class="block text-[0.65rem] font-normal text-slate-400 dark:text-neutral-500">{col.month} {col.date}</span>
                  </th>
                {/each}
                <th class="table-heading-right w-20">Total</th>
              </tr>
            </thead>
            <tbody>
              {#each grouped as custGroup (custGroup.customerId)}
                <!-- Customer header row -->
                <tr class="border-t border-slate-300 dark:border-neutral-700 bg-slate-200/60 dark:bg-neutral-800/60">
                  <td colspan="9" class="px-3 py-1 text-[0.72rem] font-semibold uppercase tracking-[0.22em] text-slate-600 dark:text-neutral-400">
                    {custGroup.customerName}
                  </td>
                </tr>
                {#each custGroup.projects as group (group.projectId)}
                  <!-- Project sub-header row -->
                  <tr class="bg-slate-100/60 dark:bg-neutral-800/60">
                    <td colspan="9" class="pl-6 pr-3 py-1 text-[0.72rem] font-semibold uppercase tracking-[0.18em] text-slate-500 dark:text-neutral-400">
                      {group.projectName}
                    </td>
                  </tr>
                  <!-- Issue rows -->
                  {#each group.rows as row (row.issue_id)}
                    <tr class="table-row">
                      <td class="table-cell pl-8">{row.issue_title}</td>
                      {#each DAYS as [, field] (field)}
                        <td class={cellClass(row, field)}>{cellValue(row, field)}</td>
                      {/each}
                      <td class="table-cell text-right font-medium text-slate-950 dark:text-neutral-100">
                        {row.total > 0 ? formatHours(row.total) : ''}
                      </td>
                    </tr>
                  {/each}
                {/each}
              {/each}

              <!-- Totals row -->
              {#if rows.length > 0}
                <tr class="border-t-2 border-slate-300 dark:border-neutral-700">
                  <td class="table-cell-strong">Total</td>
                  {#each dayTotals as total, index (DAYS[index][1])}
                    <td class="table-cell text-right font-semibold text-slate-950 dark:text-neutral-100">
                      {total > 0 ? formatHours(total) : ''}
                    </td>
                  {/each}
                  <td class="table-cell text-right font-semibold text-slate-950 dark:text-neutral-100">
                    {formatHours(grandTotal)}
                  </td>
                </tr>
              {/if}
            </tbody>
          </table>
        {/if}
      </div>
    </div>

  </div>
</div>
