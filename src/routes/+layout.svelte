<script lang="ts">
  import "../app.css";
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { toThemeMode } from '#lib/theme.js';
  import { applyTheme, theme } from '#lib/theme-state.svelte.js';
  import { normalizeShortcut } from '#lib/palette.js';
  import { palette } from '#lib/palette-state.svelte.js';
  import type { PaletteSettings } from '#lib/types.js';
  import CommandPalette from '#lib/components/CommandPalette.svelte';
  import Toast from '#lib/components/Toast.svelte';

  let { children } = $props();

  const navLinks = [
    { href: '/', label: 'Today' },
    { href: '/timesheet', label: 'Timesheet' },
    { href: '/todos', label: 'Todos' },
    { href: '/reports/weekly', label: 'Weekly' },
    { href: '/reports/monthly', label: 'Monthly' },
    { href: '/travel', label: 'Travel' },
    { href: '/settings', label: 'Settings' },
  ] as const;

  onMount(() => {
    // Start from the mirrored mode (already painted by app.html), then defer to the DB.
    applyTheme(theme.mode);
    invoke<string>('get_theme')
      .then((mode) => applyTheme(toThemeMode(mode)))
      .catch(() => {});
    invoke<PaletteSettings>('get_palette_settings')
      .then((s) => (palette.shortcut = normalizeShortcut(s.shortcut)))
      .catch(() => {});
  });

  function navigate(href: (typeof navLinks)[number]['href']) {
    void goto(resolve(href));
  }
</script>

<nav class="border-b border-slate-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
  <div class="nav-shell">
    <span class="nav-brand">TimeTracker</span>
    <div class="flex flex-wrap gap-1">
      {#each navLinks as link (link.href)}
        <a
          href={resolve(link.href)}
          class="nav-link"
          class:nav-link-active={page.url.pathname === link.href}
        >{link.label}</a>
      {/each}
    </div>
  </div>
</nav>

{@render children()}

<CommandPalette pages={navLinks} {navigate} />
<Toast />
