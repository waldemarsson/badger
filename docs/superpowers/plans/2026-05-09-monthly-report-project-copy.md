# Monthly Report Project Copy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add project-level icon actions to the monthly report so users can copy newline-delimited issue titles or hour values for one project at a time.

**Architecture:** Keep the feature local to `src/routes/reports/monthly/+page.svelte`. Extend the existing derived project tree with small copy helpers and lightweight page feedback, then add two accessible icon buttons to each project row that write plain-text newline-separated values to the clipboard. Leave the report structure, customer rows, and total row unchanged.

**Tech Stack:** Svelte 5, TypeScript, Tauri/Web clipboard API, lucide-svelte, svelte-check, ESLint

---

## File structure

- `src/routes/reports/monthly/+page.svelte` — monthly report tree rendering, project-row copy actions, clipboard formatting, and copy feedback

### Task 1: Add per-project copy icons for issues and hours

**Files:**
- Modify: `src/routes/reports/monthly/+page.svelte`
- Test: `src/routes/reports/monthly/+page.svelte`

- [ ] **Step 1: Add the failing state and helper skeletons**

In `src/routes/reports/monthly/+page.svelte`, add the icon imports and the minimal state/helpers that the new UI will rely on:

```ts
import Copy from 'lucide-svelte/icons/copy';
import Hash from 'lucide-svelte/icons/hash';

let copyMessage = $state('');
let copyError = $state('');

function projectIssueLines(project: ProjectNode): string[] {
  return project.issues.map((issue) => issue.title);
}

function projectHourLines(project: ProjectNode): string[] {
  return project.issues.map((issue) => formatHours(issue.hours));
}

async function copyProjectIssues(project: ProjectNode): Promise<void> {
  copyError = '';
  copyMessage = '';
  throw new Error('implement in step 3');
}

async function copyProjectHours(project: ProjectNode): Promise<void> {
  copyError = '';
  copyMessage = '';
  throw new Error('implement in step 3');
}
```

These helpers keep the copy scope local to one project and preserve the current issue order from the derived `tree`.

- [ ] **Step 2: Run the checker to verify the new references fail for the expected reason**

Run: `npm run check`

Expected: FAIL because the new imports and helper skeletons are present but the template does not use them yet, or because the thrown placeholder functions are not yet wired to working UI behavior.

- [ ] **Step 3: Implement clipboard formatting, feedback, and project-row icon actions**

Replace the placeholder functions with working clipboard helpers:

```ts
async function writeProjectCopy(lines: string[], successMessage: string): Promise<void> {
  copyError = '';
  copyMessage = '';

  if (lines.length === 0) {
    copyError = 'Nothing to copy for this project.';
    return;
  }

  try {
    await navigator.clipboard.writeText(lines.join('\n'));
    copyMessage = successMessage;
  } catch {
    copyError = 'Copy failed.';
  }
}

async function copyProjectIssues(project: ProjectNode): Promise<void> {
  await writeProjectCopy(projectIssueLines(project), 'Issues copied.');
}

async function copyProjectHours(project: ProjectNode): Promise<void> {
  await writeProjectCopy(projectHourLines(project), 'Hours copied.');
}
```

Add lightweight feedback above the table body, reusing existing page patterns:

```svelte
{#if copyMessage}
  <div class="mx-4 mt-3 rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
    {copyMessage}
  </div>
{/if}

{#if copyError}
  <div class="stack-error mx-4 mt-3">{copyError}</div>
{/if}
```

Update the project row so it includes two icon buttons beside the project name:

```svelte
<td class="table-cell pl-6">
  <div class="flex items-center gap-2">
    <button
      class="p-1 rounded text-slate-400 hover:text-slate-700 transition"
      onclick={() => toggleProject(customer.id, project.id)}
      title={expandedProjects.has(`${customer.id}-${project.id}`) ? 'Collapse project' : 'Expand project'}
      aria-label={expandedProjects.has(`${customer.id}-${project.id}`) ? 'Collapse project' : 'Expand project'}
    >
      {#if expandedProjects.has(`${customer.id}-${project.id}`)}
        <ChevronDown size={14} />
      {:else}
        <ChevronRight size={14} />
      {/if}
    </button>

    <span class="text-slate-800">{project.name}</span>

    <div class="ml-auto flex items-center gap-1">
      <button
        class="btn-icon-secondary"
        onclick={() => copyProjectIssues(project)}
        title="Copy issues"
        aria-label="Copy issues"
      >
        <Copy size={14} />
      </button>
      <button
        class="btn-icon-secondary"
        onclick={() => copyProjectHours(project)}
        title="Copy hours"
        aria-label="Copy hours"
      >
        <Hash size={14} />
      </button>
    </div>
  </div>
</td>
```

Keep the customer rows and grand total row unchanged, and do not add copy actions to issue rows.

- [ ] **Step 4: Run page verification**

Run: `npm run check && npx eslint src/routes/reports/monthly/+page.svelte`

Expected: PASS for `npm run check`. If targeted ESLint still reports baseline issues in this file unrelated to the copy feature, record them explicitly; the new copy-action code itself should not introduce new lint errors.

- [ ] **Step 5: Manually verify clipboard behavior**

Run: `npm run dev`

Expected:
- the monthly report still loads
- each project row shows two icon controls with distinct tooltips/accessibility labels
- clicking the issue-copy icon copies newline-delimited issue titles for only that project
- clicking the hours-copy icon copies newline-delimited hour values for only that project
- pasted content lands as separate rows in Excel or another grid tool because values are joined with `\n`
- customer rows and the total row do not show copy controls

- [ ] **Step 6: Commit**

```bash
git add src/routes/reports/monthly/+page.svelte
git commit -m "Add project copy actions to monthly report"
```
