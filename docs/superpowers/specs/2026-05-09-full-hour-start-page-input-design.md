# Full-hour start-page input design

## Problem

The start page currently allows hour entry in quarter-hour increments by using `step="0.25"` on both the new-entry and edit hour inputs.

That no longer matches the desired workflow for the start page.

## Goal

Make both hour inputs on the start page accept positive whole hours only.

## Non-goals

- Changing backend storage away from `f64`
- Rewriting reports or display formatting for existing stored entries
- Changing hour-entry behavior outside the start page

## Recommended approach

Constrain the start-page create and edit hour inputs in the Svelte page and tighten the existing page-level validation to reject decimals before calling the backend.

- Change both hour inputs to use whole-hour input settings
- Require positive integers in the page validation for create and edit flows
- Leave Rust command signatures and persistence unchanged so existing fractional records remain readable

## Why this approach

1. It matches the requested behavior with the smallest safe change.
2. It avoids breaking existing data that may already contain fractional hours.
3. It keeps the change local to the page where the behavior is being changed.

## Design details

### Start-page hour inputs

Update both hour inputs in `src/routes/+page.svelte`:

- new-entry hour input
- inline edit hour input

Both inputs should:

- use `step="1"`
- use `min="1"`
- keep the existing max unless current page logic requires a different limit

### Validation behavior

The page already validates hour values before invoking backend commands. Tighten that validation so:

- empty values are still rejected
- values less than or equal to zero are rejected
- decimal values such as `0.5` or `1.5` are rejected
- positive whole numbers such as `1`, `2`, and `8` are accepted

Apply the same rule to both create and edit flows so the page behaves consistently.

### Existing data behavior

This change should not alter how existing entries are loaded or displayed.

If the database already contains fractional hours, those rows should continue to render normally on the page. The restriction only applies when entering or editing values through the start-page inputs after this change.

### Testing

Verify:

- create flow accepts whole hours and rejects decimals
- edit flow accepts whole hours and rejects decimals
- existing fractional rows can still load and display without breaking the page

## Affected files

- `src/routes/+page.svelte`

