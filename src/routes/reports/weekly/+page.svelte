<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { WeeklyReportRow } from '$lib/types';
  import ChevronLeft  from 'lucide-svelte/icons/chevron-left';
  import ChevronRight from 'lucide-svelte/icons/chevron-right';

  // ── Date utilities ─────────────────────────────────────────────────────

  const MONTH_ABBR = ['Jan','Feb','Mar','Apr','May','Jun',
                      'Jul','Aug','Sep','Oct','Nov','Dec'];

  /** Monday of the given ISO year+week. */
  function isoWeekToMonday(year: number, week: number): Date {
    const jan4 = new Date(year, 0, 4);
    const jan4Weekday = (jan4.getDay() + 6) % 7; // Mon=0..Sun=6
    const week1Mon = new Date(jan4);
    week1Mon.setDate(jan4.getDate() - jan4Weekday);
    const monday = new Date(week1Mon);
    monday.setDate(week1Mon.getDate() + (week - 1) * 7);
    return monday;
  }

  /** ISO {year, week} for any Monday date. Thursday determines the ISO year. */
  function getMondayIsoWeek(monday: Date): { year: number; week: number } {
    const thu = new Date(monday);
    thu.setDate(monday.getDate() + 3);
    const isoYear = thu.getFullYear();
    const jan4 = new Date(isoYear, 0, 4);
    const jan4Weekday = (jan4.getDay() + 6) % 7;
    const week1Mon = new Date(jan4);
    week1Mon.setDate(jan4.getDate() - jan4Weekday);
    const diffMs  = monday.getTime() - week1Mon.getTime();
    const week = Math.round(diffMs / (7 * 86400000)) + 1;
    return { year: isoYear, week };
  }

  /** ISO year+week for today. */
  function getCurrentIsoWeek(): { year: number; week: number } {
    const today = new Date();
    const weekday = (today.getDay() + 6) % 7; // Mon=0
    const monday  = new Date(today);
    monday.setDate(today.getDate() - weekday);
    return getMondayIsoWeek(monday);
  }

  /** "W15 2026 · Apr 6–12" or "W1 2026 · Dec 29 – Jan 4" for cross-month weeks. */
  function weekLabel(year: number, week: number): string {
    const monday = isoWeekToMonday(year, week);
    const sunday = new Date(monday);
    sunday.setDate(monday.getDate() + 6);
    const mAbbr = MONTH_ABBR[monday.getMonth()];
    const sAbbr = MONTH_ABBR[sunday.getMonth()];
    const range = monday.getMonth() === sunday.getMonth()
      ? `${mAbbr} ${monday.getDate()}–${sunday.getDate()}`
      : `${mAbbr} ${monday.getDate()} – ${sAbbr} ${sunday.getDate()}`;
    return `W${week} ${year} · ${range}`;
  }

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
    const monday = isoWeekToMonday(isoYear, isoWeek);
    return DAYS.map(([name,], i) => {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      return { name, date: d.getDate(), month: MONTH_ABBR[d.getMonth()] };
    });
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
    const customers = new Map<number, CustomerGroup>();
    for (const row of rows) {
      if (!customers.has(row.customer_id)) {
        customers.set(row.customer_id, { customerId: row.customer_id, customerName: row.customer_name, projects: [] });
      }
      const cust = customers.get(row.customer_id)!;
      let proj = cust.projects.find(p => p.projectId === row.project_id);
      if (!proj) {
        proj = { projectId: row.project_id, projectName: row.project_name, rows: [] };
        cust.projects.push(proj);
      }
      proj.rows.push(row);
    }
    return Array.from(customers.values());
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
    return 'table-cell text-right text-slate-950';
  }
</script>

<div class="page-shell">
  <div class="page-view">

    <section class="page-hero">
      <p class="page-hero-kicker">TimeTracker</p>
      <h1 class="page-hero-title">Weekly Report</h1>
      <p class="page-hero-copy">Hours by issue for the selected ISO week.</p>
    </section>

    <div class="table-card">
      <!-- Week navigator toolbar -->
      <div class="table-toolbar flex items-center gap-3">
        <button class="btn-icon-secondary" onclick={prevWeek} disabled={loading} title="Previous week">
          <ChevronLeft size={16} />
        </button>
        <span class="min-w-[16rem] text-center text-sm font-semibold text-slate-700">
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
              <p class="font-semibold text-slate-700">No entries this week</p>
              <p>No time was logged for {label}.</p>
            </div>
          </div>
        {:else}
          <table class="table-base">
            <thead class="table-head">
              <tr>
                <th class="table-heading min-w-[12rem] flex-1">Issue</th>
                {#each dayLabels as col}
                  <th class="table-heading-right w-20 leading-tight">
                    <span class="block">{col.name}</span>
                    <span class="block text-[0.65rem] font-normal text-slate-400">{col.month} {col.date}</span>
                  </th>
                {/each}
                <th class="table-heading-right w-20">Total</th>
              </tr>
            </thead>
            <tbody>
              {#each grouped as custGroup (custGroup.customerId)}
                <!-- Customer header row -->
                <tr class="border-t border-slate-300 bg-slate-200/60">
                  <td colspan="9" class="px-3 py-1 text-[0.72rem] font-semibold uppercase tracking-[0.22em] text-slate-600">
                    {custGroup.customerName}
                  </td>
                </tr>
                {#each custGroup.projects as group (group.projectId)}
                  <!-- Project sub-header row -->
                  <tr class="bg-slate-100/60">
                    <td colspan="9" class="pl-6 pr-3 py-1 text-[0.72rem] font-semibold uppercase tracking-[0.18em] text-slate-500">
                      {group.projectName}
                    </td>
                  </tr>
                  <!-- Issue rows -->
                  {#each group.rows as row (row.issue_id)}
                    <tr class="table-row">
                      <td class="table-cell pl-8">{row.issue_title}</td>
                      {#each DAYS as [, field]}
                        <td class={cellClass(row, field)}>{cellValue(row, field)}</td>
                      {/each}
                      <td class="table-cell text-right font-medium text-slate-950">
                        {row.total > 0 ? formatHours(row.total) : ''}
                      </td>
                    </tr>
                  {/each}
                {/each}
              {/each}

              <!-- Totals row -->
              {#if rows.length > 0}
                <tr class="border-t-2 border-slate-300">
                  <td class="table-cell-strong">Total</td>
                  {#each dayTotals as total}
                    <td class="table-cell text-right font-semibold text-slate-950">
                      {total > 0 ? formatHours(total) : ''}
                    </td>
                  {/each}
                  <td class="table-cell text-right font-semibold text-slate-950">
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
