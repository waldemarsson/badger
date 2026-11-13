// Pure helpers for the automatic backup settings.

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
