<script lang="ts">
  import "../app.css";
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { toThemeMode } from '#lib/theme.js';
  import { applyTheme, theme } from '#lib/theme-state.svelte.js';
  import { normalizeShortcut, togglesScratchpad } from '#lib/palette.js';
  import { palette } from '#lib/palette-state.svelte.js';
  import { FLUSH_REQUEST_EVENT, runFlushHandlers } from '#lib/flush-guard.js';
  import { watchSemanticStatus } from '#lib/semantic-state.svelte.js';
  import { watchAppleSync } from '#lib/apple-sync-state.svelte.js';
  import { clock, requestOpen, tasks, watchTasks } from '#lib/tasks-state.svelte.js';
  import { overdueCount } from '#lib/tasks.js';
  import { openScratchpad, scratchpad, toggleScratchpad } from '#lib/scratchpad-state.svelte.js';
  import { checkForUpdate, updater } from '#lib/updater-state.svelte.js';
  import { changes, requestLogTime, showToast, TIME_ENTRIES_EVENT } from '#lib/palette-state.svelte.js';
  import { requestHandoverFocus } from '#lib/handover-state.svelte.js';
  import { navLinks, type Href } from '#lib/nav.js';
  import type { PaletteSettings } from '#lib/types.js';

  /** Rust's `tray::ACTION_EVENT`: the menu bar and the global shortcut ask for these. */
  const APP_ACTION_EVENT = 'app-action';
  import CommandPalette from '#lib/components/CommandPalette.svelte';
  import Toast from '#lib/components/Toast.svelte';
  import ReminderToasts from '#lib/components/ReminderToasts.svelte';
  import TaskUndoToast from '#lib/components/TaskUndoToast.svelte';
  import ScratchpadDrawer from '#lib/components/ScratchpadDrawer.svelte';
  import SettingsIcon from '@lucide/svelte/icons/settings';
  import StickyNote from '@lucide/svelte/icons/sticky-note';

  let { children } = $props();

  // The floating palette window (global shortcut) loads /palette: no chrome, no app wiring.
  const floating = page.url.pathname === resolve('/palette');

  // One "Reports" tab covers both reports and reopens the last one viewed this session.
  let lastReport = $state<'/reports/weekly' | '/reports/monthly'>('/reports/weekly');
  $effect(() => {
    const path = page.url.pathname;
    if (path === '/reports/weekly' || path === '/reports/monthly') lastReport = path;
  });

  // Tab groups by job: the day, planning and capture, time and money. Settings is the gear.
  type Tab = { href: Href; label: string };
  const tabGroups = $derived<Tab[][]>([
    [{ href: '/', label: 'Today' }],
    [
      { href: '/tasks', label: 'Tasks' },
      { href: '/notes', label: 'Notes' },
      { href: '/playground', label: 'Playground' },
    ],
    [
      { href: '/timesheet', label: 'Timesheet' },
      { href: '/travel', label: 'Travel' },
      { href: lastReport, label: 'Reports' },
    ],
  ]);
  const navGroups = $derived(tabGroups.map((group) =>
    group.map((tab) => ({
      ...tab,
      active: tab.label === 'Reports' ? page.url.pathname.startsWith('/reports/') : page.url.pathname === tab.href,
    })),
  ));

  onMount(() => {
    // Start from the mirrored mode (already painted by app.html), then defer to the DB.
    applyTheme(theme.mode);
    invoke<string>('get_theme')
      .then((mode) => applyTheme(toThemeMode(mode)))
      .catch(() => {});
    invoke<PaletteSettings>('get_palette_settings')
      .then((s) => (palette.shortcut = normalizeShortcut(s.shortcut)))
      .catch(() => {});
    if (floating) return watchSemanticStatus(); // the palette's note search uses it

    // Quit/close guard: save what pages have pending, then let Rust exit.
    let unlisten: (() => void) | null = null;
    let disposed = false;
    listen(FLUSH_REQUEST_EVENT, async () => {
      await runFlushHandlers();
      await invoke('flush_done').catch(() => {});
    })
      .then((stop) => (disposed ? stop() : (unlisten = stop)))
      .catch(() => {});
    // Menu bar and the floating palette (Rust has already shown and focused the window).
    let stopAction: (() => void) | null = null;
    listen<{ action: string; href?: string }>(APP_ACTION_EVENT, ({ payload }) => {
      // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query string
      if (payload.action === 'new-task') void goto(`${resolve('/tasks')}?new=1`);
      else if (payload.action === 'log-time') requestLogTime();
      // eslint-disable-next-line svelte/no-navigation-without-resolve -- the palette sends resolved paths
      else if (payload.action === 'goto' && payload.href?.startsWith('/')) void goto(payload.href);
      else if (payload.action === 'scratchpad') openScratchpad();
      else if (payload.action === 'handover') {
        requestHandoverFocus();
        if (page.url.pathname !== resolve('/')) void goto(resolve('/'));
      }
    })
      .then((stop) => (disposed ? stop() : (stopAction = stop)))
      .catch(() => {});
    // Time logged in the floating palette: Today and Timesheet reload.
    let stopEntries: (() => void) | null = null;
    listen(TIME_ENTRIES_EVENT, () => (changes.timeEntries += 1))
      .then((stop) => (disposed ? stop() : (stopEntries = stop)))
      .catch(() => {});
    const stopSemantic = watchSemanticStatus();
    const stopAppleSync = watchAppleSync(); // "Keep off phone" controls show while sync is on
    // The shared tasks list (sync, scheduler and other windows' writes arrive as events).
    // Notification click (Rust already focused the window): open the task on /tasks.
    const stopTasks = watchTasks((id) => {
      requestOpen(id);
      if (page.url.pathname !== '/tasks') void goto(resolve('/tasks'));
    });
    // Quiet update check: a toast and a dot on the gear, never a dialog. Errors (offline,
    // no release yet) are silent here; Settings shows them on a manual check.
    if (!import.meta.env.DEV) {
      checkForUpdate()
        .then((u) => { if (u && !disposed) showToast(`Badger ${u.version} is available — see Settings`); })
        .catch(() => {});
    }
    window.addEventListener('keydown', onScratchpadKeydown, true);
    return () => {
      disposed = true;
      unlisten?.();
      stopAction?.();
      stopEntries?.();
      stopSemantic();
      stopAppleSync();
      stopTasks();
      window.removeEventListener('keydown', onScratchpadKeydown, true);
    };
  });

  /** ⌘J on every page. Capture phase, so it works from inside the drawer and editors too. */
  function onScratchpadKeydown(e: KeyboardEvent) {
    const ctx = { paletteOpen: palette.open, recording: palette.recording, paletteShortcut: palette.shortcut };
    if (!togglesScratchpad(e, ctx)) return;
    e.preventDefault();
    e.stopPropagation();
    if (!e.repeat) toggleScratchpad();
  }

  // Past due or a sent reminder (D10), like the dock badge; the clock ticks it over as times pass.
  const overdue = $derived(overdueCount(tasks.list, new Date(clock.ms)));

  function navigate(href: Href) {
    void goto(resolve(href));
  }
</script>

{#if floating}
{@render children()}
{:else}
<nav class="border-b border-slate-100 dark:border-neutral-800/60 bg-white dark:bg-neutral-900">
  <div class="nav-shell">
    <span class="nav-brand">Badger</span>
    <div class="flex flex-wrap items-center gap-1">
      {#each navGroups as group, i (i)}
        {#if i > 0}<span class="nav-divider" aria-hidden="true"></span>{/if}
        {#each group as link (link.label)}
          <a
            href={resolve(link.href)}
            class="nav-link"
            class:nav-link-active={link.active}
            aria-current={link.active ? 'page' : undefined}
          >{link.label}{#if link.href === '/tasks' && overdue > 0}<span class="nav-count" title="{overdue} overdue">{overdue}</span>{/if}</a>
        {/each}
      {/each}
      <span class="nav-divider" aria-hidden="true"></span>
      <button
        type="button"
        class="nav-link inline-flex h-10 items-center px-3"
        class:nav-link-active={scratchpad.open}
        aria-pressed={scratchpad.open}
        aria-label="Scratchpad"
        title="Scratchpad (⌘J)"
        data-scratchpad-toggle
        onclick={toggleScratchpad}
      ><StickyNote size={16} /></button>
      <a
        href={resolve('/settings')}
        class="nav-link relative inline-flex h-10 items-center px-3"
        class:nav-link-active={page.url.pathname === '/settings'}
        aria-current={page.url.pathname === '/settings' ? 'page' : undefined}
        aria-label={updater.available ? `Settings — Badger ${updater.available.version} available` : 'Settings'}
        title={updater.available ? `Badger ${updater.available.version} is available` : 'Settings'}
      ><SettingsIcon size={16} />{#if updater.available}<span class="nav-update-dot" aria-hidden="true"></span>{/if}</a>
    </div>
  </div>
</nav>

{@render children()}

<ScratchpadDrawer />
<CommandPalette pages={navLinks} {navigate} />
<ReminderToasts />
<TaskUndoToast />
<Toast />
{/if}
