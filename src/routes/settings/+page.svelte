<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { Customer, Project } from '$lib/types';
  import Star   from 'lucide-svelte/icons/star';
  import Trash2 from 'lucide-svelte/icons/trash-2';
  import Check  from 'lucide-svelte/icons/check';
  import X      from 'lucide-svelte/icons/x';
  import Download   from 'lucide-svelte/icons/download';
  import FolderOpen from 'lucide-svelte/icons/folder-open';
  import { save, open } from '@tauri-apps/plugin-dialog';

  type Tab = 'customers' | 'preferences' | 'backup' | 'dev';
  let activeTab = $state<Tab>('customers');

  // ── Customers & state Projects 
  let customers          = $state<Customer[]>([]);
  let projects           = $state<Project[]>([]);
  let selectedCustomerId = $state<number | null>(null);
  let newCustomerName    = $state('');
  let newProjectName     = $state('');
  let savingCustomer     = $state(false);
  let savingProject      = $state(false);
  let newCustomerError   = $state('');
  let newProjectError    = $state('');
  let customerDeleteErrors = $state({} as Record<number, string>);
  let projectDeleteErrors  = $state({} as Record<number, string>);

  const selectedCustomer = $derived(
    customers.find(c => c.id === selectedCustomerId) ?? null
  );

  onMount(() => {
    void Promise.allSettled([
      invoke<Customer[]>('list_customers').then((loadedCustomers) => {
        customers = loadedCustomers;
      }),
      loadStartupBackupSettings(),
    ]);
  });

  $effect(() => {
    if (selectedCustomerId !== null) {
      invoke<Project[]>('list_projects', { customerId: selectedCustomerId })
        .then(p => { projects = p; });
    } else {
      projects = [];
    }
  });

  // ── actions Customer ──────────
  async function saveCustomer() {
    newCustomerError = '';
    if (!newCustomerName.trim()) { newCustomerError = 'Customer name is required.'; return; }
    savingCustomer = true;
    try {
      const created = await invoke<Customer>('create_customer', { name: newCustomerName.trim() });
      customers = [...customers, created];
      newCustomerName = '';
    } catch (e) { newCustomerError = String(e); }
    finally { savingCustomer = false; }
  }

  function discardCustomer() { newCustomerName = ''; newCustomerError = ''; }

  async function deleteCustomer(id: number) {
    delete customerDeleteErrors[id];
    try {
      await invoke<void>('delete_customer', { id });
      customers = customers.filter(c => c.id !== id);
      if (selectedCustomerId === id) selectedCustomerId = null;
    } catch {
      const projs = await invoke<Project[]>('list_projects', { customerId: id });
      const n = projs.length;
      customerDeleteErrors[id] = `Has ${n} project${n !== 1 ? 's' : ''} — delete them first`;
    }
  }

  async function toggleDefaultCustomer(customer: Customer) {
    customers = customers.map(c => ({
      ...c, use_as_default: c.id === customer.id ? !customer.use_as_default : false,
    }));
    try {
      if (!customer.use_as_default) {
        await invoke<void>('set_default_customer', { id: customer.id });
      } else {
        customers = await invoke<Customer[]>('list_customers');
      }
    } catch { customers = await invoke<Customer[]>('list_customers'); }
  }

  function selectCustomer(id: number) {
    selectedCustomerId = id;
    projectDeleteErrors = {};
  }

  // ── Project actions ────────────────────────────────────────────────────
  async function saveProject() {
    newProjectError = '';
    if (selectedCustomerId === null) return;
    if (!newProjectName.trim()) { newProjectError = 'Project name is required.'; return; }
    savingProject = true;
    try {
      const created = await invoke<Project>('create_project', {
        customerId: selectedCustomerId, name: newProjectName.trim(),
      });
      projects = [...projects, created];
      newProjectName = '';
    } catch (e) { newProjectError = String(e); }
    finally { savingProject = false; }
  }

  function discardProject() { newProjectName = ''; newProjectError = ''; }

  async function deleteProject(id: number) {
    delete projectDeleteErrors[id];
    try {
      await invoke<void>('delete_project', { id });
      projects = projects.filter(p => p.id !== id);
    } catch {
      const n = await invoke<number>('count_project_time_entries', { projectId: id });
      projectDeleteErrors[id] = `Has ${n} time entr${n !== 1 ? 'ies' : 'y'} — delete them first`;
    }
  }

  async function toggleDefaultProject(project: Project) {
    projects = projects.map(p => ({
      ...p, use_as_default: p.id === project.id ? !project.use_as_default : false,
    }));
    try {
      if (!project.use_as_default) {
        await invoke<void>('set_default_project', { projectId: project.id });
      } else if (selectedCustomerId !== null) {
        projects = await invoke<Project[]>('list_projects', { customerId: selectedCustomerId });
      }
    } catch {
      if (selectedCustomerId !== null)
        projects = await invoke<Project[]>('list_projects', { customerId: selectedCustomerId });
    }
  }

  // ── Backup state ──────────────────────────────────────────────────────────────
  let backupState = $state<'idle' | 'saving' | 'success' | 'error'>('idle');
  let backupError = $state('');
  type StoredStartupBackup = { date: string; size_bytes: number };
  let startupBackupRetention = $state('');
  let startupBackupRetentionState = $state<'loading' | 'idle' | 'saving' | 'success' | 'error'>('loading');
  let startupBackupRetentionError = $state('');
  let startupBackupRetentionMessage = $state('');
  let storedStartupBackups = $state<StoredStartupBackup[]>([]);
  let storedStartupBackupsState = $state<'loading' | 'ready' | 'error'>('loading');
  let storedStartupBackupsError = $state('');

  // ── Dev seed state (dev-only) ────────────────────────────────────────────
  let seedState = $state<'idle' | 'seeding' | 'success' | 'error'>('idle');
  let seedError = $state('');

  // ── Restore state ─────────────────────────────────────────────────────────────
  type RestorePreview = { customers: number; projects: number; issues: number; time_entries: number };
  type RestoreMode    = 'replace' | 'merge';

  let restoreFileName   = $state<string>('');
  let restoreJsonText   = $state<string>('');
  let restorePreview    = $state<RestorePreview | null>(null);
  let restoreMode       = $state<RestoreMode>('merge');
  let restoreParseError = $state('');
  let restoreState      = $state<'idle' | 'preview' | 'restoring' | 'success' | 'error'>('idle');
  let restoreError      = $state('');

  function parseBackupPreview(jsonText: string): RestorePreview | null {
    try {
      const data = JSON.parse(jsonText);
      return {
        customers:    Array.isArray(data.customers)    ? data.customers.length    : 0,
        projects:     Array.isArray(data.projects)     ? data.projects.length     : 0,
        issues:       Array.isArray(data.issues)       ? data.issues.length       : 0,
        time_entries: Array.isArray(data.time_entries) ? data.time_entries.length : 0,
      };
    } catch { return null; }
  }

  function formatBackupDate(date: string) {
    const parsed = new Date(`${date}T00:00:00`);
    if (Number.isNaN(parsed.getTime())) return date;
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(parsed);
  }

  function formatBackupSize(sizeBytes: number) {
    if (sizeBytes < 1024) return `${sizeBytes} B`;

    const units = ['KB', 'MB', 'GB', 'TB'];
    let value = sizeBytes / 1024;
    let unitIndex = 0;

    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex += 1;
    }

    return `${value.toFixed(value >= 10 || unitIndex === 0 ? 1 : 2)} ${units[unitIndex]}`;
  }

  function validateStartupBackupRetention(value: string) {
    const normalized = value.trim();

    if (!normalized) return 'Retention is required.';
    if (!/^\d+$/.test(normalized)) return 'Retention must be a non-negative whole number.';

    const parsed = Number(normalized);
    if (!Number.isSafeInteger(parsed)) return 'Retention is too large.';

    return null;
  }

  async function loadStartupBackupSettings() {
    startupBackupRetentionError = '';
    startupBackupRetentionMessage = '';
    startupBackupRetentionState = 'loading';
    storedStartupBackupsState = 'loading';
    storedStartupBackupsError = '';

    const [retentionResult, backupsResult] = await Promise.allSettled([
      invoke<number>('get_startup_backup_retention'),
      invoke<StoredStartupBackup[]>('list_startup_backups'),
    ]);

    if (retentionResult.status === 'fulfilled') {
      startupBackupRetention = String(retentionResult.value);
      startupBackupRetentionState = 'idle';
    } else {
      startupBackupRetentionState = 'error';
      startupBackupRetentionError = `Could not load startup backup retention. ${String(retentionResult.reason)}`;
    }

    if (backupsResult.status === 'fulfilled') {
      storedStartupBackups = backupsResult.value;
      storedStartupBackupsState = 'ready';
    } else {
      storedStartupBackups = [];
      storedStartupBackupsState = 'error';
      storedStartupBackupsError = `Could not load stored startup backups. ${String(backupsResult.reason)}`;
    }
  }

  function handleStartupBackupRetentionInput() {
    startupBackupRetentionError = '';
    startupBackupRetentionMessage = '';
    if (startupBackupRetentionState !== 'loading') {
      startupBackupRetentionState = 'idle';
    }
  }

  async function saveStartupBackupRetention() {
    startupBackupRetentionError = '';
    startupBackupRetentionMessage = '';

    const validationError = validateStartupBackupRetention(startupBackupRetention);
    if (validationError) {
      startupBackupRetentionState = 'error';
      startupBackupRetentionError = validationError;
      return;
    }

    startupBackupRetentionState = 'saving';

    try {
      const savedRetention = await invoke<number>('set_startup_backup_retention', {
        value: Number(startupBackupRetention.trim()),
      });
      startupBackupRetention = String(savedRetention);
      startupBackupRetentionState = 'success';
      startupBackupRetentionMessage = 'Startup backup retention saved.';
    } catch (e) {
      startupBackupRetentionState = 'error';
      startupBackupRetentionError = String(e);
    }
  }

  async function exportBackup() {
    backupState = 'saving';
    backupError = '';
    try {
      const today = new Date().toISOString().slice(0, 10);
      const path = await save({
        defaultPath: `TimeTracker-backup-${today}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!path) { backupState = 'idle'; return; }
      await invoke<void>('backup_data_to_file', { path });
      backupState = 'success';
    } catch (e) {
      backupError = String(e);
      backupState = 'error';
    }
  }

  async function chooseRestoreFile() {
    restoreParseError = '';
    restorePreview = null;
    const path = await open({
      filters: [{ name: 'JSON', extensions: ['json'] }],
    });
    if (!path) return;
    try {
      const json = await invoke<string>('read_backup_file', { path });
      const preview = parseBackupPreview(json);
      if (!preview) {
        restoreParseError = 'Could not read backup file. Make sure it is a valid TimeTracker backup.';
        return;
      }
      restoreFileName  = path.split('/').pop() ?? path;
      restoreJsonText  = json;
      restorePreview   = preview;
      restoreState     = 'preview';
      restoreMode      = 'merge';
    } catch (e) {
      restoreParseError = String(e);
    }
  }

  function clearRestoreFile() {
    restoreFileName   = '';
    restoreJsonText   = '';
    restorePreview    = null;
    restoreParseError = '';
    restoreState      = 'idle';
    restoreError      = '';
  }

  async function executeRestore() {
    restoreState = 'restoring';
    restoreError = '';
    try {
      await invoke<void>('restore_data', { mode: restoreMode, json: restoreJsonText });
      restoreState = 'success';
    } catch (e) {
      restoreError = String(e);
      restoreState = 'error';
    }
  }

  async function seedDatabase() {
    seedState = 'seeding';
    seedError = '';
    try {
      await invoke<string>('seed_database');
      seedState = 'success';
      // Refresh customer list to reflect seeded data (matches existing inline pattern)
      customers = await invoke<Customer[]>('list_customers');
    } catch (e) {
      seedError = String(e);
      seedState = 'error';
    }
  }
</script>

<div class="page-shell">
  <div class="page-view">

    <section class="page-hero">
      <p class="page-hero-kicker">TimeTracker</p>
      <h1 class="page-hero-title">Settings</h1>
    </section>

    <!-- Tab bar -->
    <div class="mb-4 flex gap-1 border-b border-slate-200">
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'customers'
          ? 'border-b-2 border-slate-900 text-slate-900'
          : 'text-slate-500 hover:text-slate-800'}"
        onclick={() => activeTab = 'customers'}
      >Customers &amp; Projects</button>
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'preferences'
          ? 'border-b-2 border-slate-900 text-slate-900'
          : 'text-slate-500 hover:text-slate-800'}"
        onclick={() => activeTab = 'preferences'}
      >Preferences</button>
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'backup'
          ? 'border-b-2 border-slate-900 text-slate-900'
          : 'text-slate-500 hover:text-slate-800'}"
        onclick={() => activeTab = 'backup'}
      >Backup &amp; Restore</button>
      {#if import.meta.env.DEV}
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'dev'
          ? 'border-b-2 border-amber-500 text-amber-600'
          : 'text-amber-500/70 hover:text-amber-600'}"
        onclick={() => activeTab = 'dev'}
      >Developer Tools</button>
      {/if}
    </div>

    {#if activeTab === 'customers'}
      <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">

        <!-- Customer Panel -->
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
                <tr class="table-row-new">
                  <td class="table-cell">
                    <input class="form-input" placeholder="Customer name…"
                      bind:value={newCustomerName}
                      onkeydown={(e) => e.key === 'Enter' && saveCustomer()} />
                    {#if newCustomerError}<span class="form-error">{newCustomerError}</span>{/if}
                  </td>
                  <td class="table-cell w-10"></td>
                  <td class="table-cell w-16">
                    <div class="flex gap-1">
                      <button class="btn-icon-confirm" onclick={saveCustomer}
                        disabled={savingCustomer} title="Save"><Check size={14} /></button>
                      <button class="btn-icon-danger" onclick={discardCustomer}
                        title="Discard"><X size={14} /></button>
                    </div>
                  </td>
                </tr>

                {#each customers as customer (customer.id)}
                  <tr class="table-row cursor-pointer {selectedCustomerId === customer.id
                      ? 'border-l-2 border-slate-900 bg-slate-50' : ''}"
                    onclick={() => selectCustomer(customer.id)}>
                    <td class="table-cell-strong">{customer.name}</td>
                    <td class="table-cell w-10">
                      <button class="btn-icon-star {customer.use_as_default ? 'active' : ''}"
                        onclick={(e) => { e.stopPropagation(); toggleDefaultCustomer(customer); }}
                        title={customer.use_as_default ? 'Default customer' : 'Set as default'}>
                        <Star size={14} fill={customer.use_as_default ? 'currentColor' : 'none'} />
                      </button>
                    </td>
                    <td class="table-cell w-16">
                      <button class="btn-icon-danger"
                        onclick={(e) => { e.stopPropagation(); deleteCustomer(customer.id); }}
                        title="Delete"><Trash2 size={14} /></button>
                    </td>
                  </tr>
                  {#if customerDeleteErrors[customer.id]}
                    <tr><td colspan="3" class="px-3 pb-2 pt-0">
                      <div class="stack-error">{customerDeleteErrors[customer.id]}</div>
                    </td></tr>
                  {/if}
                {/each}

                {#if customers.length === 0}
                  <tr><td colspan="3" class="px-3 py-4">
                    <div class="empty-state">
                      <p class="font-semibold text-slate-700">No customers yet</p>
                      <p>Add your first customer above.</p>
                    </div>
                  </td></tr>
                {/if}
              </tbody>
            </table>
          </div>
        </div>

        <!-- Project Panel -->
        <div class="table-card">
          <div class="table-toolbar">
            <span class="table-title">
              {selectedCustomer ? `${selectedCustomer.name} — Projects` : 'Projects'}
            </span>
          </div>

          {#if selectedCustomerId === null}
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
                  <tr class="table-row-new">
                    <td class="table-cell">
                      <input class="form-input" placeholder="Project name…"
                        bind:value={newProjectName}
                        onkeydown={(e) => e.key === 'Enter' && saveProject()} />
                      {#if newProjectError}<span class="form-error">{newProjectError}</span>{/if}
                    </td>
                    <td class="table-cell w-10"></td>
                    <td class="table-cell w-16">
                      <div class="flex gap-1">
                        <button class="btn-icon-confirm" onclick={saveProject}
                          disabled={savingProject} title="Save"><Check size={14} /></button>
                        <button class="btn-icon-danger" onclick={discardProject}
                          title="Discard"><X size={14} /></button>
                      </div>
                    </td>
                  </tr>

                  {#each projects as project (project.id)}
                    <tr class="table-row">
                      <td class="table-cell-strong">{project.name}</td>
                      <td class="table-cell w-10">
                        <button class="btn-icon-star {project.use_as_default ? 'active' : ''}"
                          onclick={() => toggleDefaultProject(project)}
                          title={project.use_as_default ? 'Default project' : 'Set as default'}>
                          <Star size={14} fill={project.use_as_default ? 'currentColor' : 'none'} />
                        </button>
                      </td>
                      <td class="table-cell w-16">
                        <button class="btn-icon-danger" onclick={() => deleteProject(project.id)}
                          title="Delete"><Trash2 size={14} /></button>
                      </td>
                    </tr>
                    {#if projectDeleteErrors[project.id]}
                      <tr><td colspan="3" class="px-3 pb-2 pt-0">
                        <div class="stack-error">{projectDeleteErrors[project.id]}</div>
                      </td></tr>
                    {/if}
                  {/each}

                  {#if projects.length === 0}
                    <tr><td colspan="3" class="px-3 py-4">
                      <div class="empty-state">
                        <p class="font-semibold text-slate-700">No projects yet</p>
                        <p>Add your first project above.</p>
                      </div>
                    </td></tr>
                  {/if}
                </tbody>
              </table>
            </div>
          {/if}
        </div>

      </div>

    {:else if activeTab === 'preferences'}
      <section class="panel-card">
        <p class="text-slate-500">Preferences will be available in a future update.</p>
      </section>

    {:else if activeTab === 'backup'}
      <div class="space-y-4">

        <section class="panel-card space-y-4">
          <div>
            <h2 class="panel-title">Startup Backups</h2>
            <p class="panel-copy">Automatic backups are created on startup. Set how many daily backups to keep and review the stored backup files below.</p>
          </div>

          <div class="grid gap-4 lg:grid-cols-[minmax(0,18rem)_1fr]">
            <div class="space-y-3">
              <div>
                <label class="form-label mb-2" for="startup-backup-retention">Retention (days)</label>
                <input
                  id="startup-backup-retention"
                  class="form-input"
                  type="text"
                  inputmode="numeric"
                  autocomplete="off"
                  bind:value={startupBackupRetention}
                  oninput={handleStartupBackupRetentionInput}
                  onkeydown={async (e) => {
                    if (e.key === 'Enter') {
                      await saveStartupBackupRetention();
                    }
                  }}
                  disabled={startupBackupRetentionState === 'loading' || startupBackupRetentionState === 'saving'}
                />
                <p class="mt-2 text-xs text-slate-500">Enter a whole number greater than or equal to 0.</p>
              </div>

              <button
                class="btn-primary"
                onclick={saveStartupBackupRetention}
                disabled={startupBackupRetentionState === 'loading' || startupBackupRetentionState === 'saving'}
              >
                {startupBackupRetentionState === 'saving' ? 'Saving…' : 'Save Retention'}
              </button>

              {#if startupBackupRetentionState === 'success'}
                <div class="alert-panel alert-success">
                  <p class="alert-success-title">Retention updated</p>
                  <p class="alert-success-copy">{startupBackupRetentionMessage}</p>
                </div>
              {/if}

              {#if startupBackupRetentionError}
                <div class="alert-panel alert-error">
                  <p class="alert-error-title">Retention unavailable</p>
                  <p class="alert-error-copy">{startupBackupRetentionError}</p>
                </div>
              {/if}
            </div>

            <div class="panel-muted space-y-3">
              <div>
                <p class="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500">Stored Backups</p>
                <p class="mt-1 text-sm text-slate-500">Read-only list of the startup backups stored beside your database.</p>
              </div>

              {#if storedStartupBackupsState === 'loading'}
                <p class="text-sm text-slate-500">Loading startup backups…</p>
              {:else if storedStartupBackupsState === 'error'}
                <div class="stack-error">{storedStartupBackupsError}</div>
              {:else if storedStartupBackups.length === 0}
                <div class="empty-state">
                  <p class="font-semibold text-slate-700">No startup backups yet</p>
                  <p>Automatic startup backups will appear here after they are created.</p>
                </div>
              {:else}
                <div class="overflow-hidden rounded-2xl border border-slate-200 bg-white">
                  <table class="table-base">
                    <thead class="table-head">
                      <tr>
                        <th class="table-heading">Date</th>
                        <th class="table-heading text-right">Size</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each storedStartupBackups as backup (backup.date)}
                        <tr class="table-row">
                          <td class="table-cell">
                            <div class="font-semibold text-slate-900">{formatBackupDate(backup.date)}</div>
                            <div class="text-xs text-slate-500">{backup.date}</div>
                          </td>
                          <td class="table-cell text-right font-medium text-slate-700">
                            {formatBackupSize(backup.size_bytes)}
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              {/if}
            </div>
          </div>
        </section>

        <!-- Backup Section -->
        <section class="panel-card space-y-4">
          <div>
            <h2 class="panel-title">Backup</h2>
            <p class="panel-copy">Export all your data as a JSON file. Store it somewhere safe — you can use it to restore your data at any time.</p>
          </div>

          <button class="btn-secondary" onclick={exportBackup} disabled={backupState === 'saving'}>
            <Download size={16} class="mr-2" />
            Export Backup
          </button>

          {#if backupState === 'success'}
            <div class="alert-panel alert-success">
              <p class="alert-success-title">Backup saved</p>
              <p class="alert-success-copy">Your data has been exported successfully.</p>
            </div>
          {/if}

          {#if backupState === 'error'}
            <div class="alert-panel alert-error">
              <p class="alert-error-title">Export failed</p>
              <p class="alert-error-copy">{backupError}</p>
            </div>
          {/if}
        </section>

        <!-- Restore Section -->
        <section class="panel-card space-y-4">
          <div>
            <h2 class="panel-title">Restore</h2>
            <p class="panel-copy">Import a backup file to restore your data. Preview the contents before choosing how to apply it.</p>
          </div>

          {#if restoreState === 'success'}
            <div class="alert-panel alert-success">
              <p class="alert-success-title">Data restored</p>
              <p class="alert-success-copy">Your backup has been applied. All data is now live.</p>
            </div>
            <button
              class="text-sm text-slate-500 underline hover:text-slate-900"
              onclick={clearRestoreFile}
            >Restore another file</button>

          {:else}
            {#if restoreState === 'idle'}
              <button class="btn-secondary" onclick={chooseRestoreFile}>
                <FolderOpen size={16} class="mr-2" />
                Choose Backup File
              </button>
              {#if restoreParseError}
                <div class="stack-error">{restoreParseError}</div>
              {/if}

            {:else}
              <!-- preview / restoring / error states -->
              <div class="flex items-center gap-2">
                <span class="text-sm text-slate-600">File: </span>
                <span class="text-sm font-semibold text-slate-950">{restoreFileName}</span>
                <button class="btn-icon-secondary" onclick={clearRestoreFile} title="Clear">
                  <X size={14} />
                </button>
              </div>

              {#if restorePreview}
                <div class="panel-muted">
                  <p class="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500 mb-3">Backup Contents</p>
                  <div class="grid grid-cols-4 gap-2 py-6 text-center">
                    <div class="restore-stat">
                      <span class="restore-stat-number">{restorePreview.customers}</span>
                      <span class="restore-stat-label">Customers</span>
                    </div>
                    <div class="restore-stat">
                      <span class="restore-stat-number">{restorePreview.projects}</span>
                      <span class="restore-stat-label">Projects</span>
                    </div>
                    <div class="restore-stat">
                      <span class="restore-stat-number">{restorePreview.issues}</span>
                      <span class="restore-stat-label">Issues</span>
                    </div>
                    <div class="restore-stat">
                      <span class="restore-stat-number">{restorePreview.time_entries}</span>
                      <span class="restore-stat-label">Time Entries</span>
                    </div>
                  </div>
                </div>
              {/if}

              <div>
                <p class="form-label mb-2">Restore Mode</p>
                <div class="flex gap-4">
                  <label class="list-card {restoreMode === 'merge' ? 'list-card-selected' : ''} flex-1 cursor-pointer">
                    <input type="radio" name="restoreMode" value="merge" class="sr-only"
                      checked={restoreMode === 'merge'}
                      onchange={() => restoreMode = 'merge'} />
                    <p class="font-semibold text-sm text-slate-950">Merge</p>
                    <p class="text-xs text-slate-500 mt-1">Adds missing records. Existing data is kept.</p>
                  </label>
                  <label class="list-card {restoreMode === 'replace' ? 'list-card-selected' : ''} flex-1 cursor-pointer">
                    <input type="radio" name="restoreMode" value="replace" class="sr-only"
                      checked={restoreMode === 'replace'}
                      onchange={() => restoreMode = 'replace'} />
                    <p class="font-semibold text-sm text-slate-950">Replace</p>
                    <p class="text-xs text-slate-500 mt-1">Wipes all data and restores from backup.</p>
                  </label>
                </div>
              </div>

              {#if restoreMode === 'replace'}
                <div class="alert-panel alert-warning">
                  <p class="alert-warning-copy">⚠ This will permanently delete all customers, projects, issues, and time entries before restoring from the backup file.</p>
                </div>
              {/if}

              <button
                class={restoreMode === 'replace' ? 'btn-danger' : 'btn-primary'}
                onclick={executeRestore}
                disabled={restoreState === 'restoring'}
              >
                {restoreState === 'restoring' ? 'Restoring…' : 'Restore Data'}
              </button>

              {#if restoreState === 'error'}
                <div class="alert-panel alert-error">
                  <p class="alert-error-title">Restore failed</p>
                  <p class="alert-error-copy">{restoreError}</p>
                </div>
              {/if}
            {/if}
          {/if}
        </section>

      </div>
    {:else if activeTab === 'dev'}
      <section class="panel-card space-y-4 border-dashed border-amber-400/50">
        <div>
          <h2 class="panel-title text-amber-600">🛠 Developer Tools</h2>
          <p class="panel-copy">These tools are only available in development mode and are excluded from production builds.</p>
        </div>

        <div>
          <h3 class="text-sm font-semibold text-slate-700 mb-1">Seed Demo Data</h3>
          <p class="text-sm text-slate-500 mb-3">Inserts demo customers, projects, issues, and time entries if the database is empty.</p>
          <div class="flex items-center gap-3">
            <button
              class="btn-primary bg-amber-600 hover:bg-amber-500"
              onclick={seedDatabase}
              disabled={seedState === 'seeding'}
            >
              {seedState === 'seeding' ? 'Seeding…' : 'Seed Demo Data'}
            </button>
            {#if seedState === 'success'}
              <span class="text-sm text-green-600">✓ Database seeded successfully</span>
            {/if}
            {#if seedState === 'error'}
              <span class="text-sm text-red-500">✗ {seedError}</span>
            {/if}
          </div>
        </div>
      </section>
    {/if}

  </div>

</div>
