<script lang="ts">
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { invoke } from '@tauri-apps/api/core';
  import CustomerProjects from '#lib/components/CustomerProjects.svelte';
  import X      from '@lucide/svelte/icons/x';
  import Download   from '@lucide/svelte/icons/download';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import Monitor from '@lucide/svelte/icons/monitor';
  import Sun     from '@lucide/svelte/icons/sun';
  import Moon    from '@lucide/svelte/icons/moon';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import type { ThemeMode } from '#lib/theme.js';
  import { applyTheme, theme } from '#lib/theme-state.svelte.js';
  import { DEFAULT_SHORTCUT, formatShortcut, recorderProblem, shortcutFromEvent } from '#lib/palette.js';
  import { scratchpadRestored } from '#lib/scratchpad-state.svelte.js';
  import { runFlushHandlers } from '#lib/flush-guard.js';
  import { palette } from '#lib/palette-state.svelte.js';
  import { formatHours, parseDailyTarget } from '#lib/today.js';
  import { formatBytes } from '#lib/notes.js';
  import type { SearchIndexStats, SemanticStatus } from '#lib/types.js';
  import { page } from '$app/state';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import { downloadPercent, semanticStatusLabel } from '#lib/semantic.js';
  import { refreshSemanticStatus, semantic } from '#lib/semantic-state.svelte.js';

  type Tab = 'customers' | 'preferences' | 'backup' | 'dev';
  let activeTab = $state<Tab>('customers');

  onMount(() => {
    // The notes search hint links to ?tab=preferences.
    if (page.url.searchParams.get('tab') === 'preferences') {
      activeTab = 'preferences';
      void loadIndexStats();
      void refreshSemanticStatus();
    }
    void loadStartupBackupSettings();
    void loadDailyTarget();
    return () => (palette.recording = false);
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

  // ── Search index ──────────────────────────────────────────────────────────────
  let indexStats = $state<SearchIndexStats | null>(null);
  let indexState = $state<'idle' | 'rebuilding' | 'rebuilt'>('idle');
  let indexError = $state('');

  async function loadIndexStats() {
    if (indexState === 'rebuilding') return;
    indexState = 'idle';
    indexError = '';
    try {
      indexStats = await invoke<SearchIndexStats>('search_index_stats');
    } catch (e) {
      indexError = String(e);
    }
  }

  async function rebuildIndex() {
    indexState = 'rebuilding';
    indexError = '';
    try {
      indexStats = await invoke<SearchIndexStats>('rebuild_search_index');
      indexState = 'rebuilt';
    } catch (e) {
      indexError = String(e);
      indexState = 'idle';
    }
  }

  // ── Semantic search ───────────────────────────────────────────────────────────
  // Status comes from the shared watch (Rust status events); actions reply with a fresh one.
  let semanticBusy = $state<'' | 'enable' | 'cancel' | 'remove' | 'rebuild'>('');
  let semanticError = $state('');
  let confirmingRemove = $state(false);
  const semanticStatus = $derived(semantic.status);

  async function semanticAction(kind: 'enable' | 'cancel' | 'remove' | 'rebuild', command: string) {
    semanticBusy = kind;
    semanticError = '';
    try {
      const next = await invoke<SemanticStatus | null>(command);
      if (next) semantic.status = next;
      else await refreshSemanticStatus();
    } catch (e) {
      semanticError = String(e);
    } finally {
      semanticBusy = '';
      confirmingRemove = false;
    }
  }

  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  // ── Hours target ──────────────────────────────────────────────────────────────
  let dailyTarget = $state('');
  let savedDailyTarget = $state<number | null>(null);
  let dailyTargetState = $state<'loading' | 'idle' | 'saving' | 'saved'>('loading');
  let dailyTargetError = $state('');

  async function loadDailyTarget() {
    try {
      const target = await invoke<number>('get_daily_target');
      savedDailyTarget = target;
      dailyTarget = formatHours(target);
    } catch (e) {
      dailyTargetError = String(e);
    } finally {
      dailyTargetState = 'idle';
    }
  }

  async function saveDailyTarget() {
    const parsed = parseDailyTarget(dailyTarget);
    if ('error' in parsed) {
      dailyTargetError = parsed.error;
      return;
    }
    dailyTargetError = '';
    dailyTargetState = 'saving';
    try {
      const target = await invoke<number>('set_daily_target', { target: parsed.target });
      savedDailyTarget = target;
      dailyTarget = formatHours(target);
      dailyTargetState = 'saved';
    } catch (e) {
      dailyTargetError = String(e);
      dailyTargetState = 'idle';
    }
  }

  function onDailyTargetKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      void saveDailyTarget();
    } else if (e.key === 'Escape' && savedDailyTarget !== null) {
      dailyTarget = formatHours(savedDailyTarget);
      dailyTargetError = '';
    }
  }

  // ── Command palette shortcut ──────────────────────────────────────────────────
  let shortcutError = $state('');

  // WebKit does not focus buttons on click, so focus explicitly to receive keydown.
  function startRecording(e: MouseEvent & { currentTarget: HTMLButtonElement }) {
    shortcutError = '';
    palette.recording = true;
    e.currentTarget.focus();
  }

  function stopRecording() {
    palette.recording = false;
  }

  async function saveShortcut(shortcut: string) {
    try {
      palette.shortcut = await invoke<string>('set_palette_shortcut', { shortcut });
      shortcutError = '';
    } catch (e) {
      shortcutError = String(e);
    }
  }

  async function onRecorderKeydown(e: KeyboardEvent) {
    if (!palette.recording) return;
    if (e.key === 'Tab') return stopRecording();
    e.preventDefault();
    e.stopPropagation();
    const noModifiers = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
    if (e.key === 'Escape' && noModifiers) {
      shortcutError = '';
      return stopRecording();
    }
    const shortcut = shortcutFromEvent(e);
    if (!shortcut) return; // modifier still being held
    const problem = recorderProblem(shortcut);
    if (problem) {
      shortcutError = problem;
      return;
    }
    stopRecording();
    await saveShortcut(shortcut);
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
  type RestorePreview = { customers: number; projects: number; issues: number; time_entries: number; notes: number };

  let restoreFileName   = $state<string>('');
  let restoreJsonText   = $state<string>('');
  let restorePreview    = $state<RestorePreview | null>(null);
  let restoreParseError = $state('');
  let restoreState      = $state<'idle' | 'preview' | 'restoring' | 'success' | 'error'>('idle');
  let restoreError      = $state('');
  let currentNoteCount  = $state(0); // notes in the database, for the "backup has no notes" warning

  function parseBackupPreview(jsonText: string): RestorePreview | null {
    try {
      const data = JSON.parse(jsonText);
      return {
        customers:    Array.isArray(data.customers)    ? data.customers.length    : 0,
        projects:     Array.isArray(data.projects)     ? data.projects.length     : 0,
        issues:       Array.isArray(data.issues)       ? data.issues.length       : 0,
        time_entries: Array.isArray(data.time_entries) ? data.time_entries.length : 0,
        notes:        Array.isArray(data.notes)        ? data.notes.length        : 0,
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
        defaultPath: `badger-backup-${today}.json`,
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
        restoreParseError = 'Could not read backup file. Make sure it is a valid Badger backup.';
        return;
      }
      currentNoteCount = preview.notes === 0
        ? await invoke<unknown[]>('list_notes').then((n) => n.length).catch(() => 0)
        : 0;
      restoreFileName  = path.split('/').pop() ?? path;
      restoreJsonText  = json;
      restorePreview   = preview;
      restoreState     = 'preview';
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
      // Pending autosaves (the scratchpad's) land first, so none can overwrite restored data.
      await runFlushHandlers();
      await invoke<void>('restore_data', { json: restoreJsonText });
      restoreState = 'success';
      scratchpadRestored(); // the drawer reloads the restored pad
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

    <h1 class="sr-only">Settings</h1>

    <!-- Tab bar -->
    <div class="mb-4 flex gap-1 border-b border-slate-200 dark:border-neutral-800">
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'customers'
          ? 'border-b-2 border-slate-900 text-slate-900 dark:border-neutral-100 dark:text-neutral-100'
          : 'text-slate-500 dark:text-neutral-400 hover:text-slate-800 dark:hover:text-neutral-200'}"
        onclick={() => activeTab = 'customers'}
      >Customers &amp; Projects</button>
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'preferences'
          ? 'border-b-2 border-slate-900 text-slate-900 dark:border-neutral-100 dark:text-neutral-100'
          : 'text-slate-500 dark:text-neutral-400 hover:text-slate-800 dark:hover:text-neutral-200'}"
        onclick={() => { activeTab = 'preferences'; void loadIndexStats(); void refreshSemanticStatus(); }}
      >Preferences</button>
      <button
        class="border-0 bg-transparent px-4 py-2 text-sm font-medium transition {activeTab === 'backup'
          ? 'border-b-2 border-slate-900 text-slate-900 dark:border-neutral-100 dark:text-neutral-100'
          : 'text-slate-500 dark:text-neutral-400 hover:text-slate-800 dark:hover:text-neutral-200'}"
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

      <section class="panel-card mt-4 space-y-4">
        <div>
          <h2 class="panel-title">Hours target</h2>
          <p class="panel-copy">Daily hours for Today's progress bars. The week target is five workdays. Set 0 to turn progress off.</p>
        </div>

        <div class="flex flex-wrap items-end gap-2">
          <div>
            <label class="form-label mb-2" for="daily-target">Daily target</label>
            <div class="relative w-32">
              <input
                id="daily-target"
                class="form-input pr-8"
                type="text"
                inputmode="decimal"
                autocomplete="off"
                bind:value={dailyTarget}
                oninput={() => { dailyTargetError = ''; if (dailyTargetState === 'saved') dailyTargetState = 'idle'; }}
                onkeydown={onDailyTargetKeydown}
                disabled={dailyTargetState === 'loading' || dailyTargetState === 'saving'}
                aria-invalid={dailyTargetError ? 'true' : undefined}
                aria-describedby={dailyTargetError ? 'daily-target-error' : undefined}
              />
              <span class="pointer-events-none absolute inset-y-0 right-3 flex items-center text-sm text-slate-400 dark:text-neutral-500">h</span>
            </div>
          </div>
          <button
            type="button"
            class="btn-secondary"
            onclick={saveDailyTarget}
            disabled={dailyTargetState === 'loading' || dailyTargetState === 'saving'}
          >{dailyTargetState === 'saving' ? 'Saving…' : 'Save'}</button>
          {#if dailyTargetState === 'saved'}
            <span class="pb-2 text-xs text-emerald-600 dark:text-emerald-400">
              {savedDailyTarget === 0 ? 'Saved — progress off' : `Saved — ${formatHours((savedDailyTarget ?? 0) * 5)} h per week`}
            </span>
          {/if}
        </div>

        {#if dailyTargetError}
          <p id="daily-target-error" class="form-error">{dailyTargetError}</p>
        {/if}
      </section>

      <section class="panel-card mt-4 space-y-4">
        <div>
          <h2 class="panel-title">Command palette</h2>
          <p class="panel-copy">Opens quick add and navigation from anywhere in the app. Needs ⌘, ⌃ or ⌥.</p>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="inline-flex min-w-[9rem] items-center justify-center rounded-lg px-4 py-2 font-mono text-sm font-medium ring-1 transition {palette.recording
              ? 'bg-sky-50 text-sky-700 ring-2 ring-sky-400 dark:bg-sky-500/10 dark:text-sky-300 dark:ring-sky-400'
              : 'bg-white text-slate-800 ring-slate-200 hover:ring-slate-300 dark:bg-neutral-800 dark:text-neutral-100 dark:ring-neutral-700 dark:hover:ring-neutral-600'}"
            aria-label="Command palette shortcut — click, then press the new combination"
            onclick={startRecording}
            onkeydown={onRecorderKeydown}
            onblur={stopRecording}
          >
            {palette.recording ? 'Press shortcut…' : formatShortcut(palette.shortcut)}
          </button>
          {#if palette.recording}
            <span class="text-xs text-slate-500 dark:text-neutral-400">Esc to cancel</span>
          {:else if palette.shortcut !== DEFAULT_SHORTCUT}
            <button type="button" class="btn-secondary-sm" onclick={() => saveShortcut(DEFAULT_SHORTCUT)}>
              Reset to {formatShortcut(DEFAULT_SHORTCUT)}
            </button>
          {/if}
        </div>

        {#if shortcutError}
          <p class="form-error">{shortcutError}</p>
        {/if}
      </section>

      <section class="panel-card mt-4 space-y-4">
        <div>
          <h2 class="panel-title">Search index</h2>
          <p class="panel-copy">Notes search uses an index that updates as you write. Rebuild it if search results look wrong.</p>
        </div>

        <div class="flex flex-wrap items-center gap-3">
          <p class="text-sm text-slate-600 dark:text-neutral-300">
            {#if indexStats}
              {indexStats.notes} {indexStats.notes === 1 ? 'note' : 'notes'} · {formatBytes(indexStats.bytes)}
            {:else if !indexError}
              Loading…
            {/if}
          </p>
          <button
            type="button"
            class="btn-secondary"
            onclick={rebuildIndex}
            disabled={indexState === 'rebuilding'}
          >{indexState === 'rebuilding' ? 'Rebuilding…' : 'Rebuild'}</button>
          {#if indexState === 'rebuilt'}
            <span class="text-xs text-emerald-600 dark:text-emerald-400">Rebuilt</span>
          {/if}
        </div>

        {#if indexError}
          <p class="form-error" role="alert">{indexError}</p>
        {/if}
      </section>

      <section class="panel-card mt-4 space-y-4" aria-labelledby="semantic-title">
        <div class="flex items-start justify-between gap-3">
          <div>
            <h2 id="semantic-title" class="panel-title">Semantic search</h2>
            <p class="panel-copy">Finds notes by meaning as well as by words, across Swedish and English. The model runs on this Mac; it downloads once (about {formatBytes(semanticStatus?.download_bytes ?? 0)}) from Hugging Face.</p>
          </div>
          {#if semanticStatus}
            <span
              class="semantic-pill"
              class:semantic-pill-ready={semanticStatus.state === 'ready'}
              class:semantic-pill-busy={semanticStatus.state === 'downloading' || semanticStatus.state === 'loading' || semanticStatus.state === 'indexing'}
              class:semantic-pill-error={semanticStatus.state === 'error'}
              role="status"
            >{semanticStatusLabel(semanticStatus)}</span>
          {/if}
        </div>

        {#if !semanticStatus}
          <p class="text-sm text-slate-500 dark:text-neutral-400">Loading…</p>
        {:else}
          {#if semanticStatus.state === 'downloading' || semanticStatus.state === 'indexing'}
            {@const percent = semanticStatus.state === 'downloading'
              ? downloadPercent(semanticStatus)
              : semanticStatus.indexing_total ? Math.floor((semanticStatus.indexing_done / semanticStatus.indexing_total) * 100) : 0}
            <div class="space-y-1.5" in:fade={{ duration: 150 }}>
              <div class="semantic-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent} aria-label={semanticStatusLabel(semanticStatus)}>
                <div class="semantic-progress-bar" style:width="{percent}%"></div>
              </div>
              <p class="text-xs text-slate-500 dark:text-neutral-400">
                {#if semanticStatus.state === 'downloading'}
                  {formatBytes(semanticStatus.downloaded_bytes)} of {formatBytes(semanticStatus.download_bytes)}
                {:else}
                  Indexing notes: {semanticStatus.indexing_done} of {semanticStatus.indexing_total}. Search already uses what is indexed.
                {/if}
              </p>
            </div>
          {/if}

          {#if semanticStatus.state === 'error'}
            <div class="alert-panel alert-error" role="alert">
              <p class="alert-error-title">Semantic search is unavailable</p>
              <p class="alert-error-copy">{semanticStatus.error}</p>
            </div>
          {/if}

          {@const installed = semanticStatus.state !== 'off' && semanticStatus.state !== 'downloading' && semanticStatus.model_bytes > 0}
          <dl class="semantic-facts">
            <div><dt>Model</dt><dd>{semanticStatus.model_name}{installed ? ` · ${formatBytes(semanticStatus.model_bytes)}` : ''}</dd></div>
            {#if installed}
              <div><dt>Index</dt><dd>{semanticStatus.indexed_notes} of {semanticStatus.notes} {semanticStatus.notes === 1 ? 'note' : 'notes'} · {formatBytes(semanticStatus.index_bytes)}</dd></div>
            {/if}
          </dl>

          <div class="flex flex-wrap items-center gap-2">
            {#if semanticStatus.state === 'off'}
              <button type="button" class="btn-primary gap-1.5" onclick={() => semanticAction('enable', 'enable_semantic_search')} disabled={semanticBusy !== ''}>
                <Download size={16} /> {semanticBusy === 'enable' ? 'Starting…' : 'Download and turn on'}
              </button>
            {:else if semanticStatus.state === 'downloading'}
              <button type="button" class="btn-secondary" onclick={() => semanticAction('cancel', 'cancel_semantic_download')} disabled={semanticBusy !== ''}>Cancel download</button>
            {:else if semanticStatus.state === 'error'}
              <button type="button" class="btn-primary gap-1.5" onclick={() => semanticAction('enable', 'enable_semantic_search')} disabled={semanticBusy !== ''}>
                <Download size={16} /> {semanticBusy === 'enable' ? 'Starting…' : semanticStatus.model_bytes > 0 ? 'Retry' : 'Download again'}
              </button>
            {:else if semanticStatus.state === 'ready'}
              <button type="button" class="btn-secondary" onclick={() => semanticAction('rebuild', 'rebuild_semantic_index')} disabled={semanticBusy !== ''}>
                {semanticBusy === 'rebuild' ? 'Rebuilding…' : 'Rebuild'}
              </button>
            {/if}
            {#if semanticStatus.state !== 'off' && semanticStatus.state !== 'downloading'}
              {#if confirmingRemove}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div class="flex items-center gap-1.5" onkeydown={(e) => { if (e.key === 'Escape') confirmingRemove = false; }} in:fade={{ duration: 150 }}>
                  <span class="text-xs font-medium text-red-700 dark:text-red-300">Delete the model and all vectors?</span>
                  <button type="button" class="btn-danger-sm" use:focusOnMount onclick={() => semanticAction('remove', 'remove_semantic_model')} disabled={semanticBusy !== ''}>
                    {semanticBusy === 'remove' ? 'Removing…' : 'Remove'}
                  </button>
                  <button type="button" class="btn-secondary-sm" onclick={() => (confirmingRemove = false)}>Cancel</button>
                </div>
              {:else}
                <button type="button" class="btn-secondary" onclick={() => (confirmingRemove = true)} disabled={semanticBusy !== ''}>Remove model</button>
              {/if}
            {/if}
            {#if semanticStatus.state === 'ready' && semanticBusy === ''}
              <span class="inline-flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400"><Sparkles size={12} /> Notes search and ⌘K include related notes</span>
            {/if}
          </div>
        {/if}

        {#if semanticError}
          <p class="form-error" role="alert">{semanticError}</p>
        {/if}
      </section>

    {:else if activeTab === 'backup'}
      <div class="space-y-4">

        <section class="panel-card space-y-4">
          <div>
            <h2 class="panel-title">Startup Backups</h2>
            <p class="panel-copy">Automatic backups are created on startup and again each new day while Badger stays open. Set how many daily backups to keep and review the stored backup files below.</p>
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
                <p class="mt-2 text-xs text-slate-500 dark:text-neutral-400">Enter a whole number greater than or equal to 0.</p>
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
                <p class="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500 dark:text-neutral-400">Stored Backups</p>
                <p class="mt-1 text-sm text-slate-500 dark:text-neutral-400">Read-only list of the startup backups stored beside your database.</p>
              </div>

              {#if storedStartupBackupsState === 'loading'}
                <p class="text-sm text-slate-500 dark:text-neutral-400">Loading startup backups…</p>
              {:else if storedStartupBackupsState === 'error'}
                <div class="stack-error">{storedStartupBackupsError}</div>
              {:else if storedStartupBackups.length === 0}
                <div class="empty-state">
                  <p class="font-semibold text-slate-700 dark:text-neutral-300">No startup backups yet</p>
                  <p>Automatic startup backups will appear here after they are created.</p>
                </div>
              {:else}
                <div class="overflow-hidden rounded-2xl border border-slate-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
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
                            <div class="font-semibold text-slate-900 dark:text-neutral-100">{formatBackupDate(backup.date)}</div>
                            <div class="text-xs text-slate-500 dark:text-neutral-400">{backup.date}</div>
                          </td>
                          <td class="table-cell text-right font-medium text-slate-700 dark:text-neutral-300">
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
            <p class="panel-copy">Import a backup file to restore your data. Preview the contents before replacing your current data.</p>
          </div>

          {#if restoreState === 'success'}
            <div class="alert-panel alert-success">
              <p class="alert-success-title">Data restored</p>
              <p class="alert-success-copy">Your backup has been applied. All data is now live.</p>
            </div>
            <button
              class="text-sm text-slate-500 dark:text-neutral-400 underline hover:text-slate-900 dark:hover:text-neutral-100"
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
                <span class="text-sm text-slate-600 dark:text-neutral-400">File: </span>
                <span class="text-sm font-semibold text-slate-950 dark:text-neutral-100">{restoreFileName}</span>
                <button class="btn-icon-secondary" onclick={clearRestoreFile} title="Clear">
                  <X size={14} />
                </button>
              </div>

              {#if restorePreview}
                <div class="panel-muted">
                  <p class="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500 dark:text-neutral-400 mb-3">Backup Contents</p>
                  <div class="grid grid-cols-5 gap-2 py-6 text-center">
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
                    <div class="restore-stat">
                      <span class="restore-stat-number">{restorePreview.notes}</span>
                      <span class="restore-stat-label">Notes</span>
                    </div>
                  </div>
                </div>
              {/if}

              <div class="alert-panel alert-warning">
                <p class="alert-warning-copy">⚠ This will permanently delete all current data — customers, projects, issues, time entries, todos, travel, and notes — and replace it with the backup.</p>
              </div>

              {#if restorePreview && restorePreview.notes === 0 && currentNoteCount > 0}
                <div class="alert-panel alert-warning" role="alert">
                  <p class="alert-warning-title">This backup has no notes; restoring deletes your {currentNoteCount} {currentNoteCount === 1 ? 'note' : 'notes'}.</p>
                </div>
              {/if}

              <button
                class="btn-danger"
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
          <h3 class="text-sm font-semibold text-slate-700 dark:text-neutral-300 mb-1">Seed Demo Data</h3>
          <p class="text-sm text-slate-500 dark:text-neutral-400 mb-3">Inserts demo customers, projects, issues, and time entries if the database is empty.</p>
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
