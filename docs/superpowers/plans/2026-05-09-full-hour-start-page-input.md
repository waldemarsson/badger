# Full-Hour Start-Page Input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the start-page create and edit hour inputs accept positive whole hours only, without breaking display of existing fractional entries.

**Architecture:** Keep the change local to `src/routes/+page.svelte`. Tighten the existing page-level validation so decimals are rejected before any Tauri command is invoked, and update both numeric inputs to whole-hour stepping. Leave Rust storage and display formatting unchanged so older fractional rows still render safely.

**Tech Stack:** Svelte 5, TypeScript, Tauri invoke, ESLint, svelte-check

---

## File structure

- `src/routes/+page.svelte` — start-page create/edit hour input attributes, validation, and error messaging

### Task 1: Restrict start-page hour entry to positive whole numbers

**Files:**
- Modify: `src/routes/+page.svelte`
- Test: `src/routes/+page.svelte`

- [ ] **Step 1: Write the failing validation change**

Update `validateEntry` in `src/routes/+page.svelte` so it rejects decimal hour values for both create and edit flows:

```ts
function validateEntry(
  date: string,
  customerId: string,
  projectId: string,
  issueIdentifier: string,
  hours: string
): Record<string, string> {
  const errs: Record<string, string> = {};
  if (!date) errs.date = 'Required';
  if (!customerId) errs.customer = 'Required';
  if (!projectId) errs.project = 'Required';
  if (!issueIdentifier.trim()) errs.issue = 'Required';

  const h = parseFloat(hours);
  if (!hours) {
    errs.hours = 'Required';
  } else if (isNaN(h) || h <= 0) {
    errs.hours = 'Must be > 0';
  } else if (!Number.isInteger(h)) {
    errs.hours = 'Must be a whole hour';
  }

  return errs;
}
```

This keeps existing error behavior for missing and non-positive values, but adds the new whole-hour rule for both the new row and the inline edit row because both already use `validateEntry(...)`.

- [ ] **Step 2: Run the checker to verify the validation edit type-checks**

Run: `npm run check`

Expected: PASS with no TypeScript or Svelte errors after tightening `validateEntry`.

- [ ] **Step 3: Update both hour inputs to whole-hour settings**

In the new-entry row, replace:

```svelte
<input type="number" step="0.25" min="0.25" max="24" bind:value={newHours} placeholder="0" class="form-input w-full text-right" />
```

with:

```svelte
<input type="number" step="1" min="1" max="24" bind:value={newHours} placeholder="0" class="form-input w-full text-right" />
```

In the inline edit row, replace:

```svelte
<input type="number" step="0.25" min="0.25" max="24" bind:value={editHours} placeholder="0" class="form-input w-full text-right" />
```

with:

```svelte
<input type="number" step="1" min="1" max="24" bind:value={editHours} placeholder="0" class="form-input w-full text-right" />
```

Do not change `formatHours(...)`, `saveEntry()`, `updateEntry()`, or the Rust commands. Existing fractional rows must continue to display using the current formatting logic:

```ts
function formatHours(h: number): string {
  const r = Math.round(h * 10000) / 10000;
  return Number.isInteger(r) ? String(Math.round(r)) : String(r);
}
```

- [ ] **Step 4: Run verification for the touched page**

Run: `npm run check && npx eslint src/routes/+page.svelte`

Expected: PASS. `npm run check` should remain clean, and the touched Svelte file should pass targeted ESLint even if repo-wide lint still has unrelated baseline failures.

- [ ] **Step 5: Manually verify the user-facing behavior**

Run: `npm run dev`

Expected:
- the Timesheet page still loads
- the new-entry Hours field increments by whole hours only
- the edit Hours field increments by whole hours only
- entering `0.5` or `1.5` in either flow shows `Must be a whole hour`
- entering `1`, `2`, or `8` still saves successfully
- any pre-existing fractional rows still render without breaking the page

- [ ] **Step 6: Commit**

```bash
git add src/routes/+page.svelte
git commit -m "Restrict start-page hours to whole numbers"
```
