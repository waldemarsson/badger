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

test('parseBackupPreview counts every previewed table, scripts included', async () => {
  const { parseBackupPreview } = await import('../src/lib/backup.ts');
  const json = JSON.stringify({ customers: [{}, {}], projects: [{}], issues: [], time_entries: [{}, {}, {}], notes: [{}], scripts: [{}, {}] });
  assert.deepEqual(parseBackupPreview(json), { customers: 2, projects: 1, issues: 0, time_entries: 3, notes: 1, scripts: 2 });
  // Older backups have no scripts (or notes) array: 0, not an error.
  assert.deepEqual(parseBackupPreview('{"customers":[]}'), { customers: 0, projects: 0, issues: 0, time_entries: 0, notes: 0, scripts: 0 });
  assert.deepEqual(parseBackupPreview('null'), { customers: 0, projects: 0, issues: 0, time_entries: 0, notes: 0, scripts: 0 });
  assert.equal(parseBackupPreview('{not json'), null);
});
