<script lang="ts">
  import "../app.css";
  import { resolve } from '$app/paths';
  import { page } from '$app/stores';
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { toThemeMode } from '$lib/theme';
  import { applyTheme, theme } from '$lib/theme-state.svelte';

  let { children } = $props();

  const navLinks = [
    { href: '/', label: 'Timesheet' },
    { href: '/todos', label: 'Todos' },
    { href: '/reports/weekly', label: 'Weekly' },
    { href: '/reports/monthly', label: 'Monthly' },
    { href: '/settings', label: 'Settings' },
  ] as const;

  onMount(() => {
    // Start from the mirrored mode (already painted by app.html), then defer to the DB.
    applyTheme(theme.mode);
    invoke<string>('get_theme')
      .then((mode) => applyTheme(toThemeMode(mode)))
      .catch(() => {});
  });
</script>

<nav class="border-b border-slate-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
  <div class="nav-shell">
    <span class="nav-brand">TimeTracker</span>
    <div class="flex flex-wrap gap-1">
      {#each navLinks as link (link.href)}
        <a
          href={resolve(link.href)}
          class="nav-link"
          class:nav-link-active={$page.url.pathname === link.href}
        >{link.label}</a>
      {/each}
    </div>
  </div>
</nav>

{@render children()}
