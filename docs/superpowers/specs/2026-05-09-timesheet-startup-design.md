# Timesheet Startup Design

## Problem

TimeTracker startup feels slow because the Timesheet page performs its heaviest initial query twice during first render. The app also provides little feedback while the first-screen data is loading.

## Goal

Make startup feel faster and do less work by:

- keeping the Timesheet shell visible immediately
- loading the initial timesheet data once
- showing a clear loading state while entries are being fetched

## Approved approach

1. Keep the Timesheet route as the startup destination.
2. Extract the initial Timesheet loading flow into a small frontend helper so the startup sequence is explicit and testable.
3. Prevent the first `$effect(...)` run from reloading entries before the initial `onMount(...)` load has completed.
4. Show a loading state in the table region during the first entries fetch and during later filter-driven reloads.

## Scope

### In scope

- Remove duplicate initial `list_time_entries` work.
- Preserve existing customer/project default selection behavior.
- Preserve existing timesheet filter behavior after startup.
- Add a user-visible loading state for entries.

### Out of scope

- Changing the app's startup route.
- Backend query redesign, indexing, or pagination.
- Startup backup behavior.

## Design

### Frontend startup orchestration

Add a small helper in `src/lib/` that performs the initial Timesheet data load:

1. load customers
2. resolve default customer and projects
3. resolve default project and issues
4. load the first entries payload exactly once

The helper returns all initial state needed by the Timesheet page so the component no longer coordinates the startup sequence ad hoc.

### Duplicate-load guard

Track whether the initial entries load has completed. The date-filter `$effect(...)` should return early until the initial startup load is finished. After that point, filter changes should continue to trigger `loadEntries()` normally.

### Loading UX

Track entry loading separately from the data arrays. While entries are loading:

- keep the page shell visible
- keep the hero visible
- show a loading row/message inside the table area

This improves perceived startup without changing navigation.

## Error handling

Keep existing error behavior unchanged for this pass. The goal is to remove redundant work and improve perceived responsiveness without expanding UX scope.

## Verification

- automated test proves the extracted startup helper loads entries once
- type-check passes
- existing Rust tests still pass
