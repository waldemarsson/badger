<script lang="ts">
  import "../app.css";
  import { page } from '$app/stores';

  let { children } = $props();

  const navLinks = [
    { href: '/', label: 'Timesheet' },
    { href: '/reports/weekly', label: 'Weekly' },
    { href: '/reports/monthly', label: 'Monthly' },
    { href: '/settings', label: 'Settings' },
  ];
</script>

<nav class="border-b border-slate-200 bg-white">
  <div class="nav-shell">
    <span class="nav-brand">TimeTracker</span>
    <div class="flex flex-wrap gap-1">
      {#each navLinks as link}
        <a
          href={link.href}
          class="nav-link"
          class:nav-link-active={$page.url.pathname === link.href}
        >{link.label}</a>
      {/each}
    </div>
  </div>
</nav>

{@render children()}
