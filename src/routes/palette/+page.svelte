<script lang="ts">
  // The floating command palette window (global shortcut, see palette_window.rs). The root
  // layout renders only this page here: no nav, toasts or app wiring.
  import { invoke } from '@tauri-apps/api/core';
  import { resolve } from '$app/paths';
  import { onMount } from 'svelte';
  import { toThemeMode } from '#lib/theme.js';
  import { applyTheme } from '#lib/theme-state.svelte.js';
  import { navLinks, type Href } from '#lib/nav.js';
  import CommandPalette from '#lib/components/CommandPalette.svelte';

  onMount(() => {
    // The theme may have changed in the main window since this one loaded.
    const refreshTheme = () =>
      invoke<string>('get_theme')
        .then((mode) => applyTheme(toThemeMode(mode)))
        .catch(() => {});
    window.addEventListener('focus', refreshTheme);
    return () => window.removeEventListener('focus', refreshTheme);
  });

  function navigate(href: Href) {
    invoke('palette_to_main', { action: 'goto', href: resolve(href) }).catch(() => {});
  }
</script>

<h1 class="sr-only">Command palette</h1>
<CommandPalette pages={navLinks} {navigate} floating />
