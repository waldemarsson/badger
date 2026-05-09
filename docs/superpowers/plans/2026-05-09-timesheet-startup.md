# Timesheet Startup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Timesheet page feel faster on startup by removing the duplicate initial entries load and showing clear progressive loading feedback.

**Architecture:** Move the initial Timesheet startup sequence into a small frontend helper so it can be tested independently from the Svelte component. Update the Timesheet page to use that helper, gate the first filter effect until startup is complete, and render a loading state while entries are being fetched.

**Tech Stack:** Svelte 5, TypeScript, Node built-in test runner, existing Tauri invoke API, svelte-check

---

### Task 1: Add a failing startup-loader test

**Files:**
- Create: `src/lib/timesheet-startup.ts`
- Create: `tests/timesheet-startup.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
import test from 'node:test';
import assert from 'node:assert/strict';
import { loadInitialTimesheetData } from '../src/lib/timesheet-startup.js';

test('loadInitialTimesheetData loads initial entries once and resolves defaults', async () => {
  const calls = [];
  const result = await loadInitialTimesheetData({
    listCustomers: async () => [{ id: 1, name: 'Acme', use_as_default: true }],
    listProjects: async (customerId) => {
      calls.push(['listProjects', customerId]);
      return [{ id: 10, customer_id: customerId, name: 'Web', use_as_default: true }];
    },
    listIssues: async (projectId) => {
      calls.push(['listIssues', projectId]);
      return [{ id: 100, project_id: projectId, external_id: null, title: 'Issue' }];
    },
    listTimeEntries: async (startDate, endDate) => {
      calls.push(['listTimeEntries', startDate, endDate]);
      return [{ id: 200, date: startDate, hours: 1, notes: null, issue_id: 100, issue_title: 'Issue', project_id: 10, project_name: 'Web', customer_id: 1, customer_name: 'Acme' }];
    },
    getDateRange: () => ({ start: '2026-05-01', end: '2026-05-31' })
  });

  assert.equal(result.newCustomerId, '1');
  assert.equal(result.newProjectId, '10');
  assert.equal(result.entries.length, 1);
  assert.deepEqual(
    calls.filter(([name]) => name === 'listTimeEntries'),
    [['listTimeEntries', '2026-05-01', '2026-05-31']]
  );
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
mkdir -p .tmp-tests && \
npx tsc src/lib/timesheet-startup.ts --module esnext --target es2022 --moduleResolution bundler --outDir .tmp-tests && \
node --test tests/timesheet-startup.test.mjs
```

Expected: FAIL because `src/lib/timesheet-startup.ts` or `loadInitialTimesheetData` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

```ts
export async function loadInitialTimesheetData(...) {
  // load customers
  // resolve default customer/project
  // load issues for default project
  // load entries once
}
```

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
mkdir -p .tmp-tests && \
npx tsc src/lib/timesheet-startup.ts --module esnext --target es2022 --moduleResolution bundler --outDir .tmp-tests && \
node --test tests/timesheet-startup.test.mjs
```

Expected: PASS

### Task 2: Wire the Timesheet page to the startup helper

**Files:**
- Modify: `src/routes/+page.svelte`
- Modify: `src/lib/timesheet-startup.ts`

- [ ] **Step 1: Write the next failing test expectation**

Use the same test file to verify the helper also returns empty `entries` when `getDateRange()` has no start/end values.

```js
test('loadInitialTimesheetData skips entries load when date range is incomplete', async () => {
  let entryCalls = 0;
  const result = await loadInitialTimesheetData({
    listCustomers: async () => [],
    listProjects: async () => [],
    listIssues: async () => [],
    listTimeEntries: async () => {
      entryCalls += 1;
      return [];
    },
    getDateRange: () => ({ start: '', end: '' })
  });

  assert.equal(entryCalls, 0);
  assert.deepEqual(result.entries, []);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
mkdir -p .tmp-tests && \
npx tsc src/lib/timesheet-startup.ts --module esnext --target es2022 --moduleResolution bundler --outDir .tmp-tests && \
node --test tests/timesheet-startup.test.mjs
```

Expected: FAIL because the helper still calls `listTimeEntries` for empty ranges.

- [ ] **Step 3: Write minimal implementation**

Update `src/lib/timesheet-startup.ts` to return `entries: []` when `start` or `end` is missing. Update `src/routes/+page.svelte` to:

```ts
let entriesLoading = $state(true);
let startupReady = $state(false);

onMount(async () => {
  entriesLoading = true;
  const initial = await loadInitialTimesheetData(...);
  customers = initial.customers;
  projects = initial.projects;
  issues = initial.issues;
  entries = initial.entries;
  newCustomerId = initial.newCustomerId;
  newProjectId = initial.newProjectId;
  newDate = today();
  startupReady = true;
  entriesLoading = false;
});

$effect(() => {
  void dateFilter;
  void customStart;
  void customEnd;
  if (!startupReady) return;
  entriesLoading = true;
  void loadEntries().finally(() => {
    entriesLoading = false;
  });
});
```

- [ ] **Step 4: Run tests and type-check**

Run:

```bash
mkdir -p .tmp-tests && \
npx tsc src/lib/timesheet-startup.ts --module esnext --target es2022 --moduleResolution bundler --outDir .tmp-tests && \
node --test tests/timesheet-startup.test.mjs && \
npm run check -- --fail-on-warnings
```

Expected: PASS

### Task 3: Show progressive loading in the table region

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Add the rendering behavior**

Render a loading row when `entriesLoading` is true:

```svelte
{#if entriesLoading}
  <tr>
    <td colspan="7">
      <div class="empty-state">
        <p class="font-semibold">Loading entries…</p>
        <p>Please wait while TimeTracker loads your timesheet.</p>
      </div>
    </td>
  </tr>
{:else if entries.length === 0}
```

- [ ] **Step 2: Run checks**

Run:

```bash
npm run check -- --fail-on-warnings
```

Expected: PASS

### Task 4: Final verification

**Files:**
- Modify: `src/routes/+page.svelte`
- Modify: `src/lib/timesheet-startup.ts`
- Create: `tests/timesheet-startup.test.mjs`

- [ ] **Step 1: Run targeted verification**

```bash
mkdir -p .tmp-tests && \
npx tsc src/lib/timesheet-startup.ts --module esnext --target es2022 --moduleResolution bundler --outDir .tmp-tests && \
node --test tests/timesheet-startup.test.mjs && \
npm run check -- --fail-on-warnings && \
cargo test --workspace --quiet time_entries::tests::test_list_time_entries
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/timesheet-startup.ts src/routes/+page.svelte tests/timesheet-startup.test.mjs docs/superpowers/specs/2026-05-09-timesheet-startup-design.md docs/superpowers/plans/2026-05-09-timesheet-startup.md
git commit -m "Improve timesheet startup loading"
```
