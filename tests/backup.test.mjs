import { test } from 'node:test';
import assert from 'node:assert/strict';
import { shortKey, lastBackupLabel } from '../src/lib/backup.ts';

test('shortKey keeps short keys and elides long ones', () => {
  assert.equal(shortKey('age1short'), 'age1short');
  const key = 'age1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs3290gq';
  assert.equal(shortKey(key), 'age1qyqszq…3290gq');
});

test('lastBackupLabel names today and yesterday, else the date', () => {
  const now = new Date(2026, 10, 12, 15, 0, 0);
  assert.equal(lastBackupLabel('2026-11-12T10:42:05', now), 'Today 10:42');
  assert.equal(lastBackupLabel('2026-11-11T23:10:00', now), 'Yesterday 23:10');
  assert.equal(lastBackupLabel('2026-11-01T08:00:00', now), '1 Nov 2026 08:00');
  assert.equal(lastBackupLabel('garbage', now), 'garbage');
});

test('lastBackupLabel handles month boundaries', () => {
  const now = new Date(2026, 11, 1, 9, 0, 0);
  assert.equal(lastBackupLabel('2026-11-30T22:00:00', now), 'Yesterday 22:00');
});

test('parseBackupPreview counts every previewed table, scripts and tasks included', async () => {
  const { parseBackupPreview } = await import('../src/lib/backup.ts');
  const json = JSON.stringify({ customers: [{}, {}], projects: [{}], issues: [], time_entries: [{}, {}, {}], notes: [{}], scripts: [{}, {}], tasks: [{}, {}, {}, {}] });
  assert.deepEqual(parseBackupPreview(json), { customers: 2, projects: 1, issues: 0, time_entries: 3, notes: 1, scripts: 2, tasks: 4 });
  // An empty tasks array replaces the current tasks with none: 0, not "kept".
  assert.equal(parseBackupPreview('{"tasks":[]}').tasks, 0);
  // Older backups have no scripts (or notes) array: 0, not an error. No tasks field (or null)
  // means a backup from before Tasks: null, and the current tasks are kept.
  assert.deepEqual(parseBackupPreview('{"customers":[]}'), { customers: 0, projects: 0, issues: 0, time_entries: 0, notes: 0, scripts: 0, tasks: null });
  assert.equal(parseBackupPreview('{"tasks":null}').tasks, null);
  assert.deepEqual(parseBackupPreview('null'), { customers: 0, projects: 0, issues: 0, time_entries: 0, notes: 0, scripts: 0, tasks: null });
  assert.equal(parseBackupPreview('{not json'), null);
});

test('restoreNotice explains a pre-Tasks backup and its skipped todos and reminders', async () => {
  const { restoreNotice } = await import('../src/lib/backup.ts');
  assert.equal(restoreNotice(null), '');
  assert.equal(restoreNotice({ kept_tasks: false, skipped_todos: 0, skipped_reminders: 0 }), '');
  assert.equal(
    restoreNotice({ kept_tasks: true, skipped_todos: 3, skipped_reminders: 1 }),
    'This backup is from before Tasks, so your current tasks were kept. Its 3 todos and 1 reminder were not restored; recreate them as tasks if you need them.'
  );
  assert.equal(restoreNotice({ kept_tasks: true, skipped_todos: 0, skipped_reminders: 0 }), 'This backup is from before Tasks, so your current tasks were kept.');
});
