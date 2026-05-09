<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { MonthlyReportRow } from '$lib/types';
  import Copy from 'lucide-svelte/icons/copy';
  import ChevronLeft  from 'lucide-svelte/icons/chevron-left';
  import ChevronRight from 'lucide-svelte/icons/chevron-right';
  import ChevronDown  from 'lucide-svelte/icons/chevron-down';
  import Hash from 'lucide-svelte/icons/hash';

  // ── Utilities ──────────────────────────────────────────────────────────

  function formatHours(h: number): string {
    const r = Math.round(h * 10000) / 10000;
    return Number.isInteger(r) ? String(Math.round(r)) : String(r);
  }

  const MONTH_NAMES = [
    'January','February','March','April','May','June',
    'July','August','September','October','November','December',
  ];

  function monthLabel(year: number, month: number): string {
    return `${MONTH_NAMES[month - 1]} ${year}`;
  }

  // ── State ──────────────────────────────────────────────────────────────
  const now      = new Date();
  let year       = $state(now.getFullYear());
  let month      = $state(now.getMonth() + 1); // 1-based
  let rows       = $state<MonthlyReportRow[]>([]);
  let loading    = $state(false);
  let copyMessage = $state('');
  let copyError = $state('');

  // Expansion state: Set of customer_ids that are expanded
  // Set is not deeply reactive with $state — use a plain $state wrapper
  let expandedCustomers = $state(new Set<number>());
  // Expansion state for projects: keyed "customerId-projectId"
  let expandedProjects  = $state(new Set<string>());

  // ── Derived ────────────────────────────────────────────────────────────
  const label = $derived(monthLabel(year, month));

  // Year/month selector options: currentYear ± 3 → flat array of {value:"YYYY-MM", label:"Month YYYY"}
  const currentYear = now.getFullYear();
  const selectorOptions = $derived((() => {
    const opts: { value: string; label: string }[] = [];
    for (let y = currentYear - 3; y <= currentYear + 3; y++) {
      for (let m = 1; m <= 12; m++) {
        opts.push({
          value: `${y}-${String(m).padStart(2, '0')}`,
          label: `${MONTH_NAMES[m - 1]} ${y}`,
        });
      }
    }
    return opts;
  })());

  const selectorValue = $derived(`${year}-${String(month).padStart(2, '0')}`);

  // Derive Customer→Project→Issue tree from flat rows
  type IssueNode   = { id: number; title: string; hours: number };
  type ProjectNode = { id: number; name: string; hours: number; issues: IssueNode[] };
  type CustomerNode = { id: number; name: string; hours: number; projects: ProjectNode[] };

  const tree = $derived((() => {
    const customerMap = new Map<number, CustomerNode>();
    for (const row of rows) {
      if (!customerMap.has(row.customer_id)) {
        customerMap.set(row.customer_id, {
          id: row.customer_id,
          name: row.customer_name,
          hours: 0,
          projects: [],
        });
      }
      const customer = customerMap.get(row.customer_id)!;

      let project = customer.projects.find(p => p.id === row.project_id);
      if (!project) {
        project = { id: row.project_id, name: row.project_name, hours: 0, issues: [] };
        customer.projects.push(project);
      }

      project.issues.push({ id: row.issue_id, title: row.issue_title, hours: row.hours });
      project.hours  += row.hours;
      customer.hours += row.hours;
    }
    return Array.from(customerMap.values());
  })());

  const grandTotal = $derived(tree.reduce((a, c) => a + c.hours, 0));

  // ── Data loading ───────────────────────────────────────────────────────
  async function loadMonth() {
    loading = true;
    try {
      rows = await invoke<MonthlyReportRow[]>('monthly_report', { year, month });
      copyMessage = '';
      copyError = '';
      // Expand all customers by default on each month load
      expandedCustomers = new Set(rows.map(r => r.customer_id));
      expandedProjects  = new Set();
    } finally {
      loading = false;
    }
  }

  onMount(() => loadMonth());

  function prevMonth() {
    if (month === 1) { year -= 1; month = 12; }
    else              { month -= 1; }
    loadMonth();
  }

  function nextMonth() {
    if (month === 12) { year += 1; month = 1; }
    else               { month += 1; }
    loadMonth();
  }

  function onSelectorChange(e: Event) {
    const val = (e.target as HTMLSelectElement).value; // "YYYY-MM"
    const [y, m] = val.split('-').map(Number);
    year  = y;
    month = m;
    loadMonth();
  }

  // ── Expansion toggles ──────────────────────────────────────────────────
  function toggleCustomer(id: number) {
    const next = new Set(expandedCustomers);
    if (next.has(id)) next.delete(id); else next.add(id);
    expandedCustomers = next;
  }

  function toggleProject(customerId: number, projectId: number) {
    const key = `${customerId}-${projectId}`;
    const next = new Set(expandedProjects);
    if (next.has(key)) next.delete(key); else next.add(key);
    expandedProjects = next;
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
      copyError = 'Nothing to copy for this project.';
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
</script>

<div class="page-shell">
  <div class="page-view">

    <section class="page-hero">
      <p class="page-hero-kicker">TimeTracker</p>
      <h1 class="page-hero-title">Monthly Report</h1>
      <p class="page-hero-copy">Hierarchical time totals by month.</p>
    </section>

    <div class="table-card">
      <!-- Month navigator toolbar -->
      <div class="table-toolbar flex items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <button class="btn-icon-secondary" onclick={prevMonth} disabled={loading} title="Previous month">
            <ChevronLeft size={16} />
          </button>
          <span class="min-w-[8rem] text-center text-sm font-semibold text-slate-700">
            {label}
          </span>
          <button class="btn-icon-secondary" onclick={nextMonth} disabled={loading} title="Next month">
            <ChevronRight size={16} />
          </button>
        </div>
        <!-- Year/month jump selector -->
        <select class="form-select w-auto" value={selectorValue} onchange={onSelectorChange}>
          {#each selectorOptions as opt (opt.value)}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>

      {#if copyMessage}
        <div class="mx-4 mt-3 rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
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
              <p class="font-semibold text-slate-700">No entries this month</p>
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
                <tr class="table-row bg-slate-50">
                  <td class="table-cell">
                    <button
                      class="p-1 rounded text-slate-400 hover:text-slate-700 transition mr-1"
                      onclick={() => toggleCustomer(customer.id)}
                    >
                      {#if expandedCustomers.has(customer.id)}
                        <ChevronDown size={14} />
                      {:else}
                        <ChevronRight size={14} />
                      {/if}
                    </button>
                    <span class="font-semibold text-slate-950">{customer.name}</span>
                  </td>
                  <td class="table-cell text-right font-semibold text-slate-950">
                    {formatHours(customer.hours)}
                  </td>
                </tr>

                {#if expandedCustomers.has(customer.id)}
                  {#each customer.projects as project (project.id)}
                    <!-- Project row (Level 1) — pl-6 indent -->
                    <tr class="table-row bg-white">
                      <td class="table-cell pl-6">
                        <div class="flex items-center gap-2">
                          <button
                            class="p-1 rounded text-slate-400 hover:text-slate-700 transition"
                            onclick={() => toggleProject(customer.id, project.id)}
                            title={expandedProjects.has(`${customer.id}-${project.id}`) ? 'Collapse project' : 'Expand project'}
                            aria-label={expandedProjects.has(`${customer.id}-${project.id}`) ? 'Collapse project' : 'Expand project'}
                          >
                            {#if expandedProjects.has(`${customer.id}-${project.id}`)}
                              <ChevronDown size={14} />
                            {:else}
                              <ChevronRight size={14} />
                            {/if}
                          </button>

                          <span class="text-slate-800">{project.name}</span>

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
                      <td class="table-cell text-right text-slate-800">
                        {formatHours(project.hours)}
                      </td>
                    </tr>

                    {#if expandedProjects.has(`${customer.id}-${project.id}`)}
                      {#each project.issues as issue (issue.id)}
                        <!-- Issue row (Level 2) — pl-12 indent, no toggle -->
                        <tr class="table-row-history">
                          <td class="table-cell pl-12 text-slate-600">{issue.title}</td>
                          <td class="table-cell text-right text-slate-600">
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
                <tr class="border-t-2 border-slate-300">
                  <td class="table-cell-strong px-3 py-2">Total</td>
                  <td class="table-cell-strong px-3 py-2 text-right">{formatHours(grandTotal)}</td>
                </tr>
              {/if}
            </tbody>
          </table>
        {/if}
      </div>
    </div>

  </div>
</div>
