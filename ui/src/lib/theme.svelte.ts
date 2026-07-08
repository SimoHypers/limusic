// Accent-color themes. Overrides the shadcn --primary / --accent tokens (+ their foregrounds) as
// inline styles on <html>, which win over both the :root and .dark rules, so the choice holds in
// light and dark mode. Persisted to localStorage — a pure UI preference, no backend round-trip.

export type ThemeId = 'rose' | 'blue' | 'lime' | 'purple' | 'teal';

// `fg` is the text/icon color that sits ON the accent (readable contrast): light accents (lime,
// teal) need a dark foreground; dark accents keep the light one.
export const THEMES: { id: ThemeId; label: string; color: string; fg: string }[] = [
	{ id: 'rose', label: 'Rose', color: 'oklch(0.455 0.188 13.697)', fg: 'oklch(0.985 0 0)' },
	{ id: 'blue', label: 'Blue', color: 'oklch(0.49 0.22 264)', fg: 'oklch(0.985 0 0)' },
	{ id: 'lime', label: 'Lime', color: 'oklch(0.77 0.2 131)', fg: 'oklch(0.205 0 0)' },
	{ id: 'purple', label: 'Purple', color: 'oklch(0.56 0.25 302)', fg: 'oklch(0.985 0 0)' },
	{ id: 'teal', label: 'Teal', color: 'oklch(0.85 0.13 181)', fg: 'oklch(0.205 0 0)' }
];

const KEY = 'primary-theme';

/** Reactive current selection, so the picker reflects it. */
export const theme = $state<{ id: ThemeId }>({ id: 'rose' });

export function applyTheme(id: ThemeId): void {
	const t = THEMES.find((x) => x.id === id) ?? THEMES[0];
	const s = document.documentElement.style;
	s.setProperty('--primary', t.color);
	s.setProperty('--primary-foreground', t.fg);
	s.setProperty('--accent', t.color);
	s.setProperty('--accent-foreground', t.fg);
	theme.id = t.id;
	localStorage.setItem(KEY, t.id);
}

/** Apply the stored theme on startup (defaults to rose). */
export function initTheme(): void {
	const stored = localStorage.getItem(KEY) as ThemeId | null;
	applyTheme(stored && THEMES.some((t) => t.id === stored) ? stored : 'rose');
}
