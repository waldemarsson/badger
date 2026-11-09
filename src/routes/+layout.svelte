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
  import { reminders, requestHighlight, watchReminders } from '#lib/reminders-state.svelte.js';
  import { scratchpad, toggleScratchpad } from '#lib/scratchpad-state.svelte.js';
  import type { PaletteSettings } from '#lib/types.js';
  import CommandPalette from '#lib/components/CommandPalette.svelte';
  import Toast from '#lib/components/Toast.svelte';
  import ReminderToasts from '#lib/components/ReminderToasts.svelte';
  import ScratchpadDrawer from '#lib/components/ScratchpadDrawer.svelte';
  import SettingsIcon from '@lucide/svelte/icons/settings';
  import StickyNote from '@lucide/svelte/icons/sticky-note';

  let { children } = $props();

  // Every page, for ⌘K. The nav bar groups them below.
  const navLinks = [
    { href: '/', label: 'Today' },
    { href: '/todos', label: 'Todos' },
    { href: '/notes', label: 'Notes' },
    { href: '/reminders', label: 'Reminders' },
    { href: '/timesheet', label: 'Timesheet' },
    { href: '/travel', label: 'Travel' },
    { href: '/reports/weekly', label: 'Weekly report' },
    { href: '/reports/monthly', label: 'Monthly report' },
    { href: '/settings', label: 'Settings' },
  ] as const;
  type Href = (typeof navLinks)[number]['href'];

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
      { href: '/todos', label: 'Todos' },
      { href: '/notes', label: 'Notes' },
      { href: '/reminders', label: 'Reminders' },
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

    // Quit/close guard: save what pages have pending, then let Rust exit.
    let unlisten: (() => void) | null = null;
    let disposed = false;
    listen(FLUSH_REQUEST_EVENT, async () => {
      await runFlushHandlers();
      await invoke('flush_done').catch(() => {});
    })
      .then((stop) => (disposed ? stop() : (unlisten = stop)))
      .catch(() => {});
    const stopSemantic = watchSemanticStatus();
    // Notification click (Rust already focused the window): open the row on /reminders.
    const stopReminders = watchReminders((id) => {
      requestHighlight(id);
      if (page.url.pathname !== '/reminders') void goto(resolve('/reminders'));
    });
    window.addEventListener('keydown', onScratchpadKeydown, true);
    return () => {
      disposed = true;
      unlisten?.();
      stopSemantic();
      stopReminders();
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

  const overdue = $derived(reminders.list.filter((r) => r.state === 'fired').length);

  function navigate(href: Href) {
    void goto(resolve(href));
  }
</script>

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
          >{link.label}{#if link.href === '/reminders' && overdue > 0}<span class="nav-count" title="{overdue} overdue">{overdue}</span>{/if}</a>
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
        onclick={toggleScratchpad}
      ><StickyNote size={16} /></button>
      <a
        href={resolve('/settings')}
        class="nav-link inline-flex h-10 items-center px-3"
        class:nav-link-active={page.url.pathname === '/settings'}
        aria-current={page.url.pathname === '/settings' ? 'page' : undefined}
        aria-label="Settings"
        title="Settings"
      ><SettingsIcon size={16} /></a>
    </div>
  </div>
</nav>

{@render children()}

<ScratchpadDrawer />
<CommandPalette pages={navLinks} {navigate} />
<ReminderToasts />
<Toast />
