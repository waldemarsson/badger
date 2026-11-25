// Pure helpers for the backup settings (automatic backups and restore).

/** What a backup holds, counted for the restore preview. */
export interface RestorePreview {
  customers: number;
  projects: number;
  issues: number;
  time_entries: number;
  notes: number;
  scripts: number;
  /** null: a backup from before Tasks (no `tasks` field), so restoring keeps the current tasks. */
  tasks: number | null;
}

/** Counts a backup's rows; null when the text is not JSON. Missing arrays count 0 (older backups). */
export function parseBackupPreview(jsonText: string): RestorePreview | null {
  let data: unknown;
  try {
    data = JSON.parse(jsonText);
  } catch {
    return null;
  }
  const field = (key: string) => (data !== null && typeof data === 'object' ? (data as Record<string, unknown>)[key] : undefined);
  const count = (key: string) => {
    const value = field(key);
    return Array.isArray(value) ? value.length : 0;
  };
  const tasks = field('tasks');
  return {
    customers: count('customers'),
    projects: count('projects'),
    issues: count('issues'),
    time_entries: count('time_entries'),
    notes: count('notes'),
    scripts: count('scripts'),
    tasks: Array.isArray(tasks) ? tasks.length : null,
  };
}

/** What `restore_data` reports (src/lib/types.ts `RestoreReport`). */
export interface RestoreOutcome {
  kept_tasks: boolean;
  skipped_todos: number;
  skipped_reminders: number;
}

/** The extra line after a restore: a backup from before Tasks kept the current tasks. '' otherwise. */
export function restoreNotice(report: RestoreOutcome | null): string {
  if (!report?.kept_tasks) return '';
  const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;
  const skipped = [
    report.skipped_todos ? plural(report.skipped_todos, 'todo') : '',
    report.skipped_reminders ? plural(report.skipped_reminders, 'reminder') : '',
  ].filter(Boolean);
  const tail = skipped.length ? ` Its ${skipped.join(' and ')} were not restored; recreate them as tasks if you need them.` : '';
  return `This backup is from before Tasks, so your current tasks were kept.${tail}`;
}

/** `age1abcdef…uvwxyz`: enough of a public key to recognise it. */
export function shortKey(key: string): string {
  return key.length <= 20 ? key : `${key.slice(0, 10)}…${key.slice(-6)}`;
}

/** "Today 10:42", "Yesterday 23:10" or "12 Nov 2026 10:42" for a local `YYYY-MM-DDTHH:MM:SS`. */
export function lastBackupLabel(stamp: string, now: Date): string {
  const at = new Date(stamp);
  if (Number.isNaN(at.getTime())) return stamp;
  const time = stamp.slice(11, 16);
  const day = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (day(at) === day(now)) return `Today ${time}`;
  if (day(at) === day(yesterday)) return `Yesterday ${time}`;
  const date = new Intl.DateTimeFormat('en-GB', { dateStyle: 'medium' }).format(at);
  return `${date} ${time}`;
}

/** Shell command that decrypts an encrypted backup without Badger. */
export const DECRYPT_COMMAND =
  'age -d -i key.txt badger-YYYY-MM-DD.json.gz.age | gunzip > backup.json';
