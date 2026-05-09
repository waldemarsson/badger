<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { Customer, Project } from '$lib/types';
  import Star   from 'lucide-svelte/icons/star';
  import Trash2 from 'lucide-svelte/icons/trash-2';
  import Check  from 'lucide-svelte/icons/check';
  import X      from 'lucide-svelte/icons/x';

  // ── State ──────────────────────────────────────────────────────────────
  let customers          = $state<Customer[]>([]);
  let projects           = $state<Project[]>([]);
  let selectedCustomerId = $state<number | null>(null);

  // New-row inputs
  let newCustomerName    = $state('');
  let newProjectName     = $state('');

  // Saving spinners (disable confirm button while awaiting invoke)
  let savingCustomer     = $state(false);
  let savingProject      = $state(false);

  // Inline form errors (new row validation / backend error)
  let newCustomerError   = $state('');
  let newProjectError    = $state('');

  // Delete guard errors — keyed by row id
  // Plain $state object (not Map) so property mutations are reactive
  let customerDeleteErrors = $state({} as Record<number, string>);
  let projectDeleteErrors  = $state({} as Record<number, string>);

  // ── Derived ────────────────────────────────────────────────────────────
  const selectedCustomer = $derived(
    customers.find(c => c.id === selectedCustomerId) ?? null
  );

  // ── Lifecycle ──────────────────────────────────────────────────────────
  onMount(async () => {
    customers = await invoke<Customer[]>('list_customers');
  });

  $effect(() => {
    // Reload projects whenever selected customer changes
    if (selectedCustomerId !== null) {
      invoke<Project[]>('list_projects', { customerId: selectedCustomerId })
        .then(p => { projects = p; });
    } else {
      projects = [];
    }
  });

  // ── Customer actions ───────────────────────────────────────────────────
  async function saveCustomer() {
    newCustomerError = '';
    if (!newCustomerName.trim()) {
      newCustomerError = 'Customer name is required.';
      return;
    }
    savingCustomer = true;
    try {
      const created = await invoke<Customer>('create_customer', { name: newCustomerName.trim() });
      customers = [...customers, created];
      newCustomerName = '';
    } catch (e) {
      newCustomerError = String(e);
    } finally {
      savingCustomer = false;
    }
  }

  function discardCustomer() {
    newCustomerName  = '';
    newCustomerError = '';
  }

  async function deleteCustomer(id: number) {
    // Clear any previous guard for this row
    delete customerDeleteErrors[id];
    try {
      await invoke<void>('delete_customer', { id });
      customers = customers.filter(c => c.id !== id);
      if (selectedCustomerId === id) selectedCustomerId = null;
    } catch {
      // FK violation — count dependent projects for the error message
      const projs = await invoke<Project[]>('list_projects', { customerId: id });
      const n = projs.length;
      customerDeleteErrors[id] =
        `Has ${n} project${n !== 1 ? 's' : ''} — delete them first`;
    }
  }

  async function toggleDefaultCustomer(customer: Customer) {
    // Optimistic UI: update local state immediately, then sync with backend
    customers = customers.map(c => ({
      ...c,
      use_as_default: c.id === customer.id ? !customer.use_as_default : false,
    }));
    try {
      if (!customer.use_as_default) {
        // Was inactive → now activating
        await invoke<void>('set_default_customer', { id: customer.id });
      } else {
        // Was active → user toggled off — no backend command for "unset default"
        // Re-fetch to restore authoritative state
        customers = await invoke<Customer[]>('list_customers');
      }
    } catch {
      // Revert on error
      customers = await invoke<Customer[]>('list_customers');
    }
  }

  function selectCustomer(id: number) {
    selectedCustomerId = id;
    // Clear project panel errors when switching customer
    projectDeleteErrors = {};
  }

  // ── Project actions ────────────────────────────────────────────────────
  async function saveProject() {
    newProjectError = '';
    if (selectedCustomerId === null) return;
    if (!newProjectName.trim()) {
      newProjectError = 'Project name is required.';
      return;
    }
    savingProject = true;
    try {
      const created = await invoke<Project>('create_project', {
        customerId: selectedCustomerId,
        name: newProjectName.trim(),
      });
      projects = [...projects, created];
      newProjectName = '';
    } catch (e) {
      newProjectError = String(e);
    } finally {
      savingProject = false;
    }
  }

  function discardProject() {
    newProjectName  = '';
    newProjectError = '';
  }

  async function deleteProject(id: number) {
    delete projectDeleteErrors[id];
    try {
      await invoke<void>('delete_project', { id });
      projects = projects.filter(p => p.id !== id);
    } catch {
      // FK violation — count linked time entries
      const n = await invoke<number>('count_project_time_entries', { projectId: id });
      projectDeleteErrors[id] =
        `Has ${n} time entr${n !== 1 ? 'ies' : 'y'} — delete them first`;
    }
  }

  async function toggleDefaultProject(project: Project) {
    projects = projects.map(p => ({
      ...p,
      use_as_default: p.id === project.id ? !project.use_as_default : false,
    }));
    try {
      if (!project.use_as_default) {
        await invoke<void>('set_default_project', { projectId: project.id });
      } else {
        // Re-fetch to restore authoritative state
        if (selectedCustomerId !== null) {
          projects = await invoke<Project[]>('list_projects', { customerId: selectedCustomerId });
        }
      }
    } catch {
      if (selectedCustomerId !== null) {
        projects = await invoke<Project[]>('list_projects', { customerId: selectedCustomerId });
      }
    }
  }
</script>

<!-- ═══════════════════════════ TEMPLATE ═══════════════════════════════ -->

<div class="page-shell">
  <div class="page-view">

    <!-- Hero -->
    <section class="page-hero">
      <p class="page-hero-kicker">TimeTracker</p>
      <h1 class="page-hero-title">Customers &amp; Projects</h1>
      <p class="page-hero-copy">Manage your customers and projects.</p>
    </section>

    <!-- Two-panel grid -->
    <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">

      <!-- ── Customer Panel ──────────────────────────────────────────── -->
      <div class="table-card">
        <div class="table-toolbar">
          <span class="table-title">Customers</span>
        </div>
        <div class="table-scroll-region">
          <table class="table-base">
            <thead class="table-head">
              <tr>
                <th class="table-heading">Name</th>
                <th class="table-heading w-10">Default</th>
                <th class="table-heading w-16">Actions</th>
              </tr>
            </thead>
            <tbody>

              <!-- New customer row (always visible) -->
              <tr class="table-row-new">
                <td class="table-cell">
                  <input
                    class="form-input"
                    placeholder="Customer name…"
                    bind:value={newCustomerName}
                    onkeydown={(e) => e.key === 'Enter' && saveCustomer()}
                  />
                  {#if newCustomerError}
                    <span class="form-error">{newCustomerError}</span>
                  {/if}
                </td>
                <td class="table-cell w-10"></td>
                <td class="table-cell w-16">
                  <div class="flex gap-1">
                    <button
                      class="btn-icon-confirm"
                      onclick={saveCustomer}
                      disabled={savingCustomer}
                      title="Save"
                    ><Check size={14} /></button>
                    <button
                      class="btn-icon-danger"
                      onclick={discardCustomer}
                      title="Discard"
                    ><X size={14} /></button>
                  </div>
                </td>
              </tr>

              <!-- Customer rows -->
              {#each customers as customer (customer.id)}
                <tr
                  class="table-row cursor-pointer {selectedCustomerId === customer.id
                    ? 'border-l-2 border-slate-900 bg-slate-50'
                    : ''}"
                  onclick={() => selectCustomer(customer.id)}
                >
                  <td class="table-cell-strong">{customer.name}</td>
                  <td class="table-cell w-10">
                    <button
                      class="btn-icon-star {customer.use_as_default ? 'active' : ''}"
                      onclick={(e) => { e.stopPropagation(); toggleDefaultCustomer(customer); }}
                      title={customer.use_as_default ? 'Default customer' : 'Set as default'}
                    >
                      {#if customer.use_as_default}
                        <Star size={14} fill="currentColor" />
                      {:else}
                        <Star size={14} />
                      {/if}
                    </button>
                  </td>
                  <td class="table-cell w-16">
                    <button
                      class="btn-icon-danger"
                      onclick={(e) => { e.stopPropagation(); deleteCustomer(customer.id); }}
                      title="Delete"
                    ><Trash2 size={14} /></button>
                  </td>
                </tr>

                <!-- Delete guard row (only when error exists for this id) -->
                {#if customerDeleteErrors[customer.id]}
                  <tr>
                    <td colspan="3" class="px-3 pb-2 pt-0">
                      <div class="stack-error">{customerDeleteErrors[customer.id]}</div>
                    </td>
                  </tr>
                {/if}
              {/each}

              <!-- Empty state -->
              {#if customers.length === 0}
                <tr>
                  <td colspan="3" class="px-3 py-4">
                    <div class="empty-state">
                      <p class="font-semibold text-slate-700">No customers yet</p>
                      <p>Add your first customer above.</p>
                    </div>
                  </td>
                </tr>
              {/if}

            </tbody>
          </table>
        </div>
      </div>

      <!-- ── Project Panel ───────────────────────────────────────────── -->
      <div class="table-card">
        <div class="table-toolbar">
          <span class="table-title">
            {selectedCustomer ? `${selectedCustomer.name} — Projects` : 'Projects'}
          </span>
        </div>

        {#if selectedCustomerId === null}
          <!-- No customer selected — placeholder -->
          <div class="px-4 py-6">
            <div class="empty-state">Select a customer to view its projects.</div>
          </div>

        {:else}
          <div class="table-scroll-region">
            <table class="table-base">
              <thead class="table-head">
                <tr>
                  <th class="table-heading">Name</th>
                  <th class="table-heading w-10">Default</th>
                  <th class="table-heading w-16">Actions</th>
                </tr>
              </thead>
              <tbody>

                <!-- New project row (always visible, disabled when no customer) -->
                <tr class="table-row-new">
                  <td class="table-cell">
                    <input
                      class="form-input"
                      placeholder="Project name…"
                      bind:value={newProjectName}
                      onkeydown={(e) => e.key === 'Enter' && saveProject()}
                    />
                    {#if newProjectError}
                      <span class="form-error">{newProjectError}</span>
                    {/if}
                  </td>
                  <td class="table-cell w-10"></td>
                  <td class="table-cell w-16">
                    <div class="flex gap-1">
                      <button
                        class="btn-icon-confirm"
                        onclick={saveProject}
                        disabled={savingProject}
                        title="Save"
                      ><Check size={14} /></button>
                      <button
                        class="btn-icon-danger"
                        onclick={discardProject}
                        title="Discard"
                      ><X size={14} /></button>
                    </div>
                  </td>
                </tr>

                <!-- Project rows -->
                {#each projects as project (project.id)}
                  <tr class="table-row">
                    <td class="table-cell-strong">{project.name}</td>
                    <td class="table-cell w-10">
                      <button
                        class="btn-icon-star {project.use_as_default ? 'active' : ''}"
                        onclick={() => toggleDefaultProject(project)}
                        title={project.use_as_default ? 'Default project' : 'Set as default'}
                      >
                        {#if project.use_as_default}
                          <Star size={14} fill="currentColor" />
                        {:else}
                          <Star size={14} />
                        {/if}
                      </button>
                    </td>
                    <td class="table-cell w-16">
                      <button
                        class="btn-icon-danger"
                        onclick={() => deleteProject(project.id)}
                        title="Delete"
                      ><Trash2 size={14} /></button>
                    </td>
                  </tr>

                  <!-- Delete guard row -->
                  {#if projectDeleteErrors[project.id]}
                    <tr>
                      <td colspan="3" class="px-3 pb-2 pt-0">
                        <div class="stack-error">{projectDeleteErrors[project.id]}</div>
                      </td>
                    </tr>
                  {/if}
                {/each}

                <!-- Empty state -->
                {#if projects.length === 0}
                  <tr>
                    <td colspan="3" class="px-3 py-4">
                      <div class="empty-state">
                        <p class="font-semibold text-slate-700">No projects yet</p>
                        <p>Add your first project above.</p>
                      </div>
                    </td>
                  </tr>
                {/if}

              </tbody>
            </table>
          </div>
        {/if}
      </div>

    </div><!-- /grid -->
  </div><!-- /page-view -->
</div><!-- /page-shell -->

