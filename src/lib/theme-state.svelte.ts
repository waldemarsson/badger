// Applies the appearance setting to the document and native window.
// The DB (get_theme/set_theme) is the source of truth; localStorage only mirrors it
// so the pre-paint script in app.html can avoid a light flash on startup.
import { getCurrentWindow } from '@tauri-apps/api/window';
import { resolveTheme, toThemeMode, type ThemeMode } from './theme';

const STORAGE_KEY = 'theme';

export const theme = $state<{ mode: ThemeMode }>({ mode: readMirror() });

let stopListening: (() => void) | null = null;

function readMirror(): ThemeMode {
	try {
		return toThemeMode(localStorage.getItem(STORAGE_KEY));
	} catch {
		return 'system';
	}
}

export function applyTheme(mode: ThemeMode): void {
	theme.mode = mode;
	const media = window.matchMedia('(prefers-color-scheme: dark)');
	const paint = () => {
		const resolved = resolveTheme(mode, media.matches);
		document.documentElement.classList.toggle('dark', resolved === 'dark');
		document.documentElement.style.colorScheme = resolved;
	};
	paint();

	stopListening?.();
	stopListening = null;
	if (mode === 'system') {
		media.addEventListener('change', paint);
		stopListening = () => media.removeEventListener('change', paint);
	}

	try {
		localStorage.setItem(STORAGE_KEY, mode);
	} catch {
		// Mirror is best-effort; the DB still holds the setting.
	}

	// Match the macOS title bar; null follows the OS. Not available outside Tauri.
	try {
		getCurrentWindow()
			.setTheme(mode === 'system' ? null : mode)
			.catch(() => {});
	} catch {
		// Ignore: no native window (e.g. a plain browser).
	}
}
