// Two kinds of theme live here, selected from one picker and persisted to localStorage (a pure UI
// preference, no backend round-trip):
//   - 'accent'  — overrides only --primary/--accent as inline styles on <html>, layered over the
//                 app's default palette. Wins over both :root and .dark.
//   - 'palette' — a full token set (background, card, sidebar, radius, …) for light AND dark, defined
//                 as a `.theme-<id>` class in layout.css. Applied by toggling that class on <html>.

export type ThemeId = 'rose' | 'blue' | 'lime' | 'purple' | 'teal' | 'catppuccin' | 'caffeine' | 'bubblegum';

// `fg` (accent themes only) is the text/icon colour that sits ON the accent: light accents (lime,
// teal) need a dark foreground; dark accents keep the light one. `color` is just the picker swatch.
type Theme =
	| { id: ThemeId; label: string; kind: 'accent'; color: string; fg: string }
	| { id: ThemeId; label: string; kind: 'palette'; color: string };

export const THEMES: Theme[] = [
	{ id: 'rose', label: 'Rose', kind: 'accent', color: 'oklch(0.455 0.188 13.697)', fg: 'oklch(0.985 0 0)' },
	{ id: 'blue', label: 'Blue', kind: 'accent', color: 'oklch(0.49 0.22 264)', fg: 'oklch(0.985 0 0)' },
	{ id: 'lime', label: 'Lime', kind: 'accent', color: 'oklch(0.77 0.2 131)', fg: 'oklch(0.205 0 0)' },
	{ id: 'purple', label: 'Purple', kind: 'accent', color: 'oklch(0.56 0.25 302)', fg: 'oklch(0.985 0 0)' },
	{ id: 'teal', label: 'Teal', kind: 'accent', color: 'oklch(0.85 0.13 181)', fg: 'oklch(0.205 0 0)' },
	{ id: 'catppuccin', label: 'Catppuccin', kind: 'palette', color: 'oklch(0.5547 0.2503 297.0156)' },
	{ id: 'caffeine', label: 'Caffeine', kind: 'palette', color: 'oklch(0.4341 0.0392 41.9938)' },
	{ id: 'bubblegum', label: 'Bubblegum', kind: 'palette', color: 'oklch(0.6209 0.1801 348.1385)' }
];

const KEY = 'primary-theme';
const PALETTE_CLASSES = THEMES.filter((t) => t.kind === 'palette').map((t) => `theme-${t.id}`);
const ACCENT_VARS = ['--primary', '--primary-foreground', '--accent', '--accent-foreground'];

/** Reactive current selection, so the picker reflects it. */
export const theme = $state<{ id: ThemeId }>({ id: 'rose' });

export function applyTheme(id: ThemeId): void {
	const t = THEMES.find((x) => x.id === id) ?? THEMES[0];
	const root = document.documentElement;
	// Reset both mechanisms first, so switching between an accent and a palette (or between palettes)
	// never leaves the previous choice's inline vars or class behind.
	ACCENT_VARS.forEach((v) => root.style.removeProperty(v));
	root.classList.remove(...PALETTE_CLASSES);
	if (t.kind === 'accent') {
		root.style.setProperty('--primary', t.color);
		root.style.setProperty('--primary-foreground', t.fg);
		root.style.setProperty('--accent', t.color);
		root.style.setProperty('--accent-foreground', t.fg);
	} else {
		root.classList.add(`theme-${t.id}`);
	}
	theme.id = t.id;
	localStorage.setItem(KEY, t.id);
}

/** Apply the stored theme on startup (defaults to rose). */
export function initTheme(): void {
	const stored = localStorage.getItem(KEY) as ThemeId | null;
	applyTheme(stored && THEMES.some((t) => t.id === stored) ? stored : 'rose');
}
