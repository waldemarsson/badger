<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import CustomerProjects from '$lib/components/CustomerProjects.svelte';
  import X      from 'lucide-svelte/icons/x';
  import Download   from 'lucide-svelte/icons/download';
  import FolderOpen from 'lucide-svelte/icons/folder-open';
  import Monitor from 'lucide-svelte/icons/monitor';
  import Sun     from 'lucide-svelte/icons/sun';
  import Moon    from 'lucide-svelte/icons/moon';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import type { ThemeMode } from '$lib/theme';
  import { applyTheme, theme } from '$lib/theme-state.svelte';

  type Tab = 'customers' | 'preferences' | 'backup' | 'dev';
  let activeTab = $state<Tab>('customers');

  onMount(() => {
    void loadStartupBackupSettings();
  });

  // ── Appearance ────────────────────────────────────────────────────────────────
  const themeOptions = [
    { mode: 'system', label: 'System', icon: Monitor },
    { mode: 'light',  label: 'Light',  icon: Sun },
    { mode: 'dark',   label: 'Dark',   icon: Moon },
  ] as const;
  let themeError = $state('');

  // Applies instantly, then persists; reverts if saving fails.
  async function chooseTheme(mode: ThemeMode) {
    const previous = theme.mode;
    if (mode === previous) return;
    themeError = '';
    applyTheme(mode);
    try {
      await invoke<string>('set_theme', { theme: mode });
    } catch (e) {
      applyTheme(previous);
      themeError = String(e);
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
    <div class="mb-4 flex gap-1 border-b border-slate-200 dark:border-slate-800">
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'customers'
          ? 'border-b-2 border-slate-900 text-slate-900 dark:border-slate-100 dark:text-slate-100'
          : 'text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200'}"
        onclick={() => activeTab = 'customers'}
      >Customers &amp; Projects</button>
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'preferences'
          ? 'border-b-2 border-slate-900 text-slate-900 dark:border-slate-100 dark:text-slate-100'
          : 'text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200'}"
        onclick={() => activeTab = 'preferences'}
      >Preferences</button>
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'backup'
          ? 'border-b-2 border-slate-900 text-slate-900 dark:border-slate-100 dark:text-slate-100'
          : 'text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200'}"
        onclick={() => activeTab = 'backup'}
      >Backup &amp; Restore</button>
      {#if import.meta.env.DEV}
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'dev'
          ? 'border-b-2 border-amber-500 text-amber-600 dark:text-amber-400'
          : 'text-amber-500/70 dark:text-amber-400/70 hover:text-amber-600 dark:hover:text-amber-400'}"
        onclick={() => activeTab = 'dev'}
      >Developer Tools</button>
      {/if}
    </div>

    {#if activeTab === 'customers'}
      <CustomerProjects />

    {:else if activeTab === 'preferences'}
      <section class="panel-card space-y-4">
        <div>
          <h2 class="panel-title">Appearance</h2>
          <p class="panel-copy">Choose a theme. System follows your macOS appearance.</p>
        </div>

        <div class="segmented" role="radiogroup" aria-label="Theme">
          {#each themeOptions as option (option.mode)}
            <label class="segmented-option" class:segmented-option-active={theme.mode === option.mode}>
              <input
                class="sr-only"
                type="radio"
                name="theme"
                value={option.mode}
                checked={theme.mode === option.mode}
                onchange={() => chooseTheme(option.mode)}
              />
              <option.icon size={16} />
              {option.label}
            </label>
          {/each}
        </div>

        {#if themeError}
          <p class="form-error">{themeError}</p>
        {/if}
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
                <p class="mt-2 text-xs text-slate-500 dark:text-slate-400">Enter a whole number greater than or equal to 0.</p>
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
                <p class="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500 dark:text-slate-400">Stored Backups</p>
                <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">Read-only list of the startup backups stored beside your database.</p>
              </div>

              {#if storedStartupBackupsState === 'loading'}
                <p class="text-sm text-slate-500 dark:text-slate-400">Loading startup backups…</p>
              {:else if storedStartupBackupsState === 'error'}
                <div class="stack-error">{storedStartupBackupsError}</div>
              {:else if storedStartupBackups.length === 0}
                <div class="empty-state">
                  <p class="font-semibold text-slate-700 dark:text-slate-300">No startup backups yet</p>
                  <p>Automatic startup backups will appear here after they are created.</p>
                </div>
              {:else}
                <div class="overflow-hidden rounded-2xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900">
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
                            <div class="font-semibold text-slate-900 dark:text-slate-100">{formatBackupDate(backup.date)}</div>
                            <div class="text-xs text-slate-500 dark:text-slate-400">{backup.date}</div>
                          </td>
                          <td class="table-cell text-right font-medium text-slate-700 dark:text-slate-300">
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
              class="text-sm text-slate-500 dark:text-slate-400 underline hover:text-slate-900 dark:hover:text-slate-100"
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
                <span class="text-sm text-slate-600 dark:text-slate-400">File: </span>
                <span class="text-sm font-semibold text-slate-950 dark:text-slate-100">{restoreFileName}</span>
                <button class="btn-icon-secondary" onclick={clearRestoreFile} title="Clear">
                  <X size={14} />
                </button>
              </div>

              {#if restorePreview}
                <div class="panel-muted">
                  <p class="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500 dark:text-slate-400 mb-3">Backup Contents</p>
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
                    <p class="font-semibold text-sm text-slate-950 dark:text-slate-100">Merge</p>
                    <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">Adds missing records. Existing data is kept.</p>
                  </label>
                  <label class="list-card {restoreMode === 'replace' ? 'list-card-selected' : ''} flex-1 cursor-pointer">
                    <input type="radio" name="restoreMode" value="replace" class="sr-only"
                      checked={restoreMode === 'replace'}
                      onchange={() => restoreMode = 'replace'} />
                    <p class="font-semibold text-sm text-slate-950 dark:text-slate-100">Replace</p>
                    <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">Wipes all data and restores from backup.</p>
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
          <h2 class="panel-title text-amber-600 dark:text-amber-400">🛠 Developer Tools</h2>
          <p class="panel-copy">These tools are only available in development mode and are excluded from production builds.</p>
        </div>

        <div>
          <h3 class="text-sm font-semibold text-slate-700 dark:text-slate-300 mb-1">Seed Demo Data</h3>
          <p class="text-sm text-slate-500 dark:text-slate-400 mb-3">Inserts demo customers, projects, issues, and time entries if the database is empty.</p>
          <div class="flex items-center gap-3">
            <button
              class="btn-primary bg-amber-600 hover:bg-amber-500"
              onclick={seedDatabase}
              disabled={seedState === 'seeding'}
            >
              {seedState === 'seeding' ? 'Seeding…' : 'Seed Demo Data'}
            </button>
            {#if seedState === 'success'}
              <span class="text-sm text-green-600 dark:text-green-400">✓ Database seeded successfully</span>
            {/if}
            {#if seedState === 'error'}
              <span class="text-sm text-red-500 dark:text-red-400">✗ {seedError}</span>
            {/if}
          </div>
        </div>
      </section>
    {/if}

  </div>

</div>
