<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { resolve } from '$app/paths';
  import type { MonthlyReportRow, TravelRow } from '#lib/types.js';
  import { MONTH_NAMES, formatNumber, monthReportText, tripTotals } from '#lib/travel.js';
  import MonthNav from '#lib/components/MonthNav.svelte';
  import ReportSwitch from '#lib/components/ReportSwitch.svelte';
  import Copy from '@lucide/svelte/icons/copy';
  import ClipboardList from '@lucide/svelte/icons/clipboard-list';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ChevronDown  from '@lucide/svelte/icons/chevron-down';
  import Hash from '@lucide/svelte/icons/hash';

  // ── Utilities ──────────────────────────────────────────────────────────

  const formatHours = formatNumber;

  function monthLabel(year: number, month: number): string {
    return `${MONTH_NAMES[month - 1]} ${year}`;
  }

  // ── State ──────────────────────────────────────────────────────────────
  const now      = new Date();
  let year       = $state(now.getFullYear());
  let month      = $state(now.getMonth() + 1); // 1-based
  let rows       = $state<MonthlyReportRow[]>([]);
  let trips      = $state<TravelRow[]>([]);
  let loading    = $state(false);
  let copyMessage = $state('');
  let copyError = $state('');

  // Expansion state: Set of customer_ids that are expanded
  // Set is not deeply reactive with $state — use a plain $state wrapper
  let expandedCustomers = $state<number[]>([]);
  // Expansion state for projects: keyed "customerId-projectId"
  let expandedProjects  = $state<string[]>([]);

  // ── Derived ────────────────────────────────────────────────────────────
  const label = $derived(monthLabel(year, month));

  // Derive Customer→Project→Issue tree from flat rows
  type IssueNode   = { id: number; title: string; hours: number };
  type ProjectNode = { id: number; name: string; hours: number; issues: IssueNode[] };
  type CustomerNode = { id: number; name: string; hours: number; projects: ProjectNode[] };

  const tree = $derived((() => {
    const customers: CustomerNode[] = [];
    for (const row of rows) {
      let customer = customers.find((entry) => entry.id === row.customer_id);
      if (!customer) {
        customer = {
          id: row.customer_id,
          name: row.customer_name,
          hours: 0,
          projects: [],
        };
        customers.push(customer);
      }

      let project = customer.projects.find(p => p.id === row.project_id);
      if (!project) {
        project = { id: row.project_id, name: row.project_name, hours: 0, issues: [] };
        customer.projects.push(project);
      }

      project.issues.push({ id: row.issue_id, title: row.issue_title, hours: row.hours });
      project.hours  += row.hours;
      customer.hours += row.hours;
    }
    return customers;
  })());

  const grandTotal = $derived(tree.reduce((a, c) => a + c.hours, 0));
  const travelTotals = $derived(tripTotals(trips));

  // ── Data loading ───────────────────────────────────────────────────────
  async function loadMonth() {
    loading = true;
    try {
      [rows, trips] = await Promise.all([
        invoke<MonthlyReportRow[]>('monthly_report', { year, month }),
        invoke<TravelRow[]>('list_travel', { year, month }),
      ]);
      copyMessage = '';
      copyError = '';
      // Expand all customers by default on each month load
      expandedCustomers = rows.reduce<number[]>((ids, row) => {
        if (!ids.includes(row.customer_id)) ids.push(row.customer_id);
        return ids;
      }, []);
      expandedProjects  = [];
    } finally {
      loading = false;
    }
  }

  onMount(() => loadMonth());

  // ── Expansion toggles ──────────────────────────────────────────────────
  function toggleCustomer(id: number) {
    expandedCustomers = expandedCustomers.includes(id)
      ? expandedCustomers.filter((entry) => entry !== id)
      : [...expandedCustomers, id];
  }

  function toggleProject(customerId: number, projectId: number) {
    const key = `${customerId}-${projectId}`;
    expandedProjects = expandedProjects.includes(key)
      ? expandedProjects.filter((entry) => entry !== key)
      : [...expandedProjects, key];
  }

  function projectIssueLines(project: ProjectNode): string[] {
    return project.issues.map((issue) => issue.title);
  }

  function projectHourLines(project: ProjectNode): string[] {
    return project.issues.map((issue) => formatHours(issue.hours));
  }

  async function writeProjectCopy(lines: string[], successMessage: string): Promise<void> {
    copyError = '';
    copyMessage = '';

    if (lines.length === 0) {
      copyError = 'Nothing to copy.';
      return;
    }

    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      copyMessage = successMessage;
    } catch {
      copyError = 'Copy failed.';
    }
  }

  async function copyProjectIssues(project: ProjectNode): Promise<void> {
    await writeProjectCopy(projectIssueLines(project), 'Issues copied.');
  }

  async function copyProjectHours(project: ProjectNode): Promise<void> {
    await writeProjectCopy(projectHourLines(project), 'Hours copied.');
  }

  async function copyMonth(): Promise<void> {
    const text = monthReportText(month, tree, trips);
    await writeProjectCopy(text === null ? [] : [text], 'Month report copied.');
  }
</script>

<div class="page-shell">
  <div class="page-view">

    <h1 class="sr-only">Monthly Report</h1>

    <div class="table-card">
      <!-- Month navigator toolbar -->
      <div class="table-toolbar flex items-center justify-between gap-3">
        <ReportSwitch />
        <MonthNav bind:year bind:month disabled={loading} onchange={loadMonth} />
        <button class="btn-secondary-sm ml-auto gap-1.5" onclick={() => void copyMonth()} title="Copy hours and travel as text">
          <ClipboardList size={14} />
          Copy month
        </button>
      </div>

      {#if copyMessage}
        <div class="mx-4 mt-3 rounded-lg border border-emerald-200 dark:border-emerald-500/30 bg-emerald-50 dark:bg-emerald-500/10 px-3 py-2 text-sm text-emerald-700 dark:text-emerald-300">
          {copyMessage}
        </div>
      {/if}

      {#if copyError}
        <div class="stack-error mx-4 mt-3">{copyError}</div>
      {/if}

      <div class="table-scroll-region">
        {#if rows.length === 0 && !loading}
          <div class="px-4 py-6">
            <div class="empty-state">
              <p class="font-semibold text-slate-700 dark:text-neutral-300">No entries this month</p>
              <p>No time was logged for {label}.</p>
            </div>
          </div>
        {:else}
          <table class="table-base">
            <thead class="table-head">
              <tr>
                <th class="table-heading flex-1">Description</th>
                <th class="table-heading-right w-24">Hours</th>
              </tr>
            </thead>
            <tbody>
              {#each tree as customer (customer.id)}
                <!-- Customer row (Level 0) — collapsed by default -->
                <tr class="table-row bg-slate-50 dark:bg-neutral-800/40">
                  <td class="table-cell">
                    <button
                      class="p-1 rounded text-slate-400 dark:text-neutral-500 hover:text-slate-700 dark:hover:text-neutral-300 transition mr-1"
                      onclick={() => toggleCustomer(customer.id)}
                    >
                      {#if expandedCustomers.includes(customer.id)}
                        <ChevronDown size={14} />
                      {:else}
                        <ChevronRight size={14} />
                      {/if}
                    </button>
                    <span class="font-semibold text-slate-950 dark:text-neutral-100">{customer.name}</span>
                  </td>
                  <td class="table-cell text-right font-semibold text-slate-950 dark:text-neutral-100">
                    {formatHours(customer.hours)}
                  </td>
                </tr>

                {#if expandedCustomers.includes(customer.id)}
                  {#each customer.projects as project (project.id)}
                    <!-- Project row (Level 1) — pl-6 indent -->
                    <tr class="table-row bg-white dark:bg-neutral-900">
                      <td class="table-cell pl-6">
                        <div class="flex items-center gap-2">
                          <button
                            class="p-1 rounded text-slate-400 dark:text-neutral-500 hover:text-slate-700 dark:hover:text-neutral-300 transition"
                            onclick={() => toggleProject(customer.id, project.id)}
                            title={expandedProjects.includes(`${customer.id}-${project.id}`) ? 'Collapse project' : 'Expand project'}
                            aria-label={expandedProjects.includes(`${customer.id}-${project.id}`) ? 'Collapse project' : 'Expand project'}
                          >
                            {#if expandedProjects.includes(`${customer.id}-${project.id}`)}
                              <ChevronDown size={14} />
                            {:else}
                              <ChevronRight size={14} />
                            {/if}
                          </button>

                          <span class="text-slate-800 dark:text-neutral-200">{project.name}</span>

                          <div class="ml-auto flex items-center gap-1">
                            <button
                              class="btn-icon-secondary"
                              onclick={() => void copyProjectIssues(project)}
                              title="Copy issues"
                              aria-label="Copy issues"
                            >
                              <Copy size={14} />
                            </button>
                            <button
                              class="btn-icon-secondary"
                              onclick={() => void copyProjectHours(project)}
                              title="Copy hours"
                              aria-label="Copy hours"
                            >
                              <Hash size={14} />
                            </button>
                          </div>
                        </div>
                      </td>
                      <td class="table-cell text-right text-slate-800 dark:text-neutral-200">
                        {formatHours(project.hours)}
                      </td>
                    </tr>

                    {#if expandedProjects.includes(`${customer.id}-${project.id}`)}
                      {#each project.issues as issue (issue.id)}
                        <!-- Issue row (Level 2) — pl-12 indent, no toggle -->
                        <tr class="table-row-history">
                          <td class="table-cell pl-12 text-slate-600 dark:text-neutral-400">{issue.title}</td>
                          <td class="table-cell text-right text-slate-600 dark:text-neutral-400">
                            {formatHours(issue.hours)}
                          </td>
                        </tr>
                      {/each}
                    {/if}
                  {/each}
                {/if}
              {/each}

              <!-- Grand total row -->
              {#if tree.length > 0}
                <tr class="border-t-2 border-slate-300 dark:border-neutral-700">
                  <td class="table-cell-strong px-3 py-2">Total</td>
                  <td class="table-cell-strong px-3 py-2 text-right">{formatHours(grandTotal)}</td>
                </tr>
              {/if}
            </tbody>
          </table>
        {/if}
      </div>
    </div>

    <section class="table-card">
      <div class="table-toolbar flex items-center justify-between gap-3">
        <span class="table-title">Travel</span>
        <a href={resolve('/travel')} class="text-sm font-medium text-sky-700 hover:text-sky-900 dark:text-sky-400 dark:hover:text-sky-300">Manage travel</a>
      </div>
      <div class="table-scroll-region">
        {#if trips.length === 0}
          {#if !loading}
            <div class="px-4 py-6">
              <div class="empty-state">
                <p class="font-semibold text-slate-700 dark:text-neutral-300">No trips</p>
                <p>No travel logged for {label}.</p>
              </div>
            </div>
          {/if}
        {:else}
          <table class="table-base">
            <thead class="table-head">
              <tr>
                <th class="table-heading w-36">Date</th>
                <th class="table-heading">Route</th>
                <th class="table-heading-right w-28">Km</th>
                <th class="table-heading-right w-32 whitespace-nowrap">Amount (kr)</th>
              </tr>
            </thead>
            <tbody>
              {#each trips as trip (trip.id)}
                <tr class="table-row-history">
                  <td class="table-cell-strong">{trip.date}</td>
                  <td class="table-cell">{trip.from_place} – {trip.to_place}</td>
                  <td class="table-cell text-right">{trip.km === null ? '—' : formatNumber(trip.km)}</td>
                  <td class="table-cell text-right">{trip.amount === null ? '—' : formatNumber(trip.amount)}</td>
                </tr>
              {/each}
              <tr class="border-t-2 border-slate-300 dark:border-neutral-700">
                <td class="table-cell-strong" colspan="2">Total</td>
                <td class="table-cell-strong text-right">{formatNumber(travelTotals.km)}</td>
                <td class="table-cell-strong text-right">{formatNumber(travelTotals.amount)}</td>
              </tr>
            </tbody>
          </table>
        {/if}
      </div>
    </section>

  </div>
</div>
