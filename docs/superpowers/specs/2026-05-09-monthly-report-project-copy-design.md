# Monthly report project copy design

## Problem

The monthly report is currently easy to read, but it is awkward to reuse in another time-reporting tool because selecting text in the table grabs both columns together.

That makes it hard to copy only the issue names or only the hour values for a single project.

## Goal

Make it easy to copy project-specific issue names or project-specific hour values from the monthly report into another tool such as Excel.

## Non-goals

- Changing the overall monthly report table structure
- Adding copy actions to customer rows or the grand total row
- Changing native text-selection behavior for the whole table

## Recommended approach

Add two explicit icon-based copy actions to each project row in the monthly report:

- copy issues icon
- copy hours icon

Each action should copy newline-delimited plain text for that specific project only, using the issue rows currently shown under that project.

## Why this approach

1. It is explicit and easy to understand.
2. It matches the project-specific workflow better than a global table-level action.
3. It avoids fragile custom selection behavior in a normal web table.

## Design details

### Project-level copy actions

In the monthly report, each project row should get two copy controls shown as icons rather than text buttons:

- one icon for copying issues
- one icon for copying hours

These actions belong only on project rows, not customer rows or the final total row.

The icons should still be clear and accessible:

- provide distinct tooltips or accessible labels for `Copy issues` and `Copy hours`
- keep the controls visually lightweight so they do not overpower the report row

### Copy scope

Each copy action should operate on one project at a time.

- `Copy issues` copies only the issue titles that belong to that project
- `Copy hours` copies only the hour values that belong to that project

The copied values should come from the issue rows for that project in the same order they appear in the report.

### Clipboard format

Clipboard content should be plain text with newline separators so each copied value pastes into a separate row in Excel or a similar tool.

Examples:

`Copy issues`

```text
Issue A
Issue B
Issue C
```

`Copy hours`

```text
4
2
1.5
```

### Visibility and expansion behavior

The copy actions should reflect the project-level issue list currently represented by the report data for that project.

The most predictable behavior is:

- when a project row is visible, its copy actions are available on that row
- copied content uses that project’s issue list in report order

The customer row and total row remain unchanged.

### Error handling and feedback

If clipboard writing fails, the page should show lightweight user-visible feedback consistent with existing page patterns.

If a project has no issue rows, the copy action should avoid producing misleading output.

### Testing

Verify:

- only issues from the selected project are copied
- only hours from the selected project are copied
- copied text is newline-delimited
- copied order matches the visible report order
- customer rows and total rows do not get copy actions

## Affected files

- `src/routes/reports/monthly/+page.svelte`
