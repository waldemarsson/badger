// Pure helpers for displaying named records (customers, projects).

/** Returns a copy sorted by name, case- and accent-insensitive in the user's locale. */
export function byName<T extends { name: string }>(list: T[]): T[] {
	return [...list].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
}

/** `plural(1, 'project')` → "1 project"; `plural(2, 'project')` → "2 projects". */
export function plural(n: number, word: string): string {
	return `${n} ${word}${n === 1 ? '' : 's'}`;
}
