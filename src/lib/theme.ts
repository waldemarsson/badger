// Pure helpers for the System / Light / Dark appearance setting.

export const THEME_MODES = ['system', 'light', 'dark'] as const;
export type ThemeMode = (typeof THEME_MODES)[number];
export type ResolvedTheme = 'light' | 'dark';

export function isThemeMode(value: unknown): value is ThemeMode {
	return typeof value === 'string' && (THEME_MODES as readonly string[]).includes(value);
}

/** Unknown or missing values fall back to 'system'. */
export function toThemeMode(value: unknown): ThemeMode {
	return isThemeMode(value) ? value : 'system';
}

/** The theme to paint: 'system' follows the OS preference. */
export function resolveTheme(mode: ThemeMode, systemPrefersDark: boolean): ResolvedTheme {
	if (mode === 'system') return systemPrefersDark ? 'dark' : 'light';
	return mode;
}
