// Two kinds of theme live here, selected from one picker and persisted to localStorage (a pure UI
// preference, no backend round-trip):
//   - 'accent'  — overrides only --primary/--accent as inline styles on <html>, layered over the
//                 app's default palette. Wins over both :root and .dark.
//   - 'palette' — a full token set (background, card, sidebar, radius, …) for light AND dark, defined
//                 as a `.theme-<id>` class in layout.css. Applied by toggling that class on <html>.
//
// On top of whichever preset is selected sits the *custom* layer (accent colour, background tint,
// roundness, fonts). It's inline styles too, applied after the preset, so it wins over both kinds
// and survives switching presets. Anything the user hasn't touched stays null and the preset shows
// through — the customization is a set of overrides, not a rival theme to maintain.

import { convertFileSrc } from '@tauri-apps/api/core';
import { hexToHsv, isLight, nearestHue } from './color';
import { artworkAccent, warmAccent } from './artcolor';
import { allowFontFile } from './api';

export type ThemeId = 'rose' | 'blue' | 'lime' | 'purple' | 'teal' | 'catppuccin' | 'caffeine' | 'neon' | 'breeze';

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
	{ id: 'neon', label: 'Neon', kind: 'palette', color: 'oklch(0.6726 0.2904 341.4084)' },
	{ id: 'breeze', label: 'Breeze', kind: 'palette', color: 'oklch(0.7227 0.1920 149.5793)' }
];

/** Font stacks bundled with the app (imported in layout.css). "System" needs no download. */
export const FONTS: { label: string; value: string }[] = [
	{ label: 'Oxanium', value: "'Oxanium Variable', sans-serif" },
	{ label: 'IBM Plex Sans', value: "'IBM Plex Sans Variable', sans-serif" },
	{ label: 'Montserrat', value: "'Montserrat Variable', sans-serif" },
	{ label: 'Outfit', value: "'Outfit Variable', sans-serif" },
	{ label: 'DM Sans', value: "'DM Sans Variable', sans-serif" },
	{ label: 'System', value: 'ui-sans-serif, system-ui, sans-serif' }
];

/** The custom layer. `null` = untouched, so the selected preset decides. */
export type Custom = {
	accent: string | null; // hex
	hue: number | null; // 0–360, tints the default palette's neutrals
	radius: number | null; // rem
	fontSans: string | null; // a CSS font-family value
	fontHeading: string | null;
	// Font files the user loaded from disk, by absolute path. Not an override — a small library that
	// both font rows can then choose from, which is why `resetCustom` leaves it alone.
	fontFiles: string[];
};

const KEY = 'primary-theme';
const CUSTOM_KEY = 'custom-theme';
const APPEARANCE_KEY = 'appearance';
const PALETTE_CLASSES = THEMES.filter((t) => t.kind === 'palette').map((t) => `theme-${t.id}`);
const ACCENT_VARS = ['--primary', '--primary-foreground', '--accent', '--accent-foreground'];
/** Set on <html> while the artwork tint is live; the surface rules in layout.css hang off it. */
const TINT_CLASS = 'art-tint';
const CUSTOM_VARS = ['--hue', '--radius', '--font-sans', '--font-heading'];
// Same two neutrals the preset accent themes pick between.
const ON_DARK = 'oklch(0.985 0 0)';
const ON_LIGHT = 'oklch(0.205 0 0)';

/** Reactive current selection, so the picker reflects it. */
export const theme = $state<{ id: ThemeId }>({ id: 'rose' });
export const custom = $state<Custom>({
	accent: null,
	hue: null,
	radius: null,
	fontSans: null,
	fontHeading: null,
	fontFiles: []
});

/**
 * Looks the UI reads directly rather than through a CSS token. Same store and same reasoning as
 * the theme above (a pure UI preference, no backend round-trip), which also means a component can
 * read it during its first render instead of flashing the default while a command round-trips.
 */
export const appearance = $state({
	/** Blur the playing track's artwork behind the now-playing view. */
	artworkBackground: true,
	/**
	 * The now-playing view carries queue and lyrics itself, as tabs, and the player bar's two
	 * buttons switch between them while it's open. Off, those buttons only ever open the floating
	 * side panels, which then sit over the now-playing view like they sit over a page (#62).
	 */
	tabbedPlayer: true,
	/** Starting playback opens the now-playing view. Off, it plays and leaves you where you are (#64). */
	openPlayerOnPlay: true,
	/** Take the accent colour from the playing track's cover, crossfading on each change. */
	artworkAccent: false
});

export function setAppearance(patch: Partial<typeof appearance>): void {
	Object.assign(appearance, patch);
	localStorage.setItem(APPEARANCE_KEY, JSON.stringify(appearance));
}

/**
 * What the tokens resolve to *after* the preset and the overrides are applied. The controls read
 * this so their starting position is wherever the current theme actually sits, instead of a
 * hardcoded default that goes stale the moment a preset moves it.
 */
export const effective = $state({
	hue: 326,
	radius: 0.45,
	accent: '#000000',
	fontSans: '',
	fontHeading: ''
});

/**
 * oklch/oklab/rgb/anything CSS -> hex. Painted and read back rather than taken from `fillStyle`,
 * which WebKitGTK hands straight back in whatever space it was given: reading the string only ever
 * worked for the hex accents, so every palette theme's picker opened on black. It matters more now
 * that `--primary` is a registered property (layout.css) and therefore computes to `oklab()` even
 * when it was written as a hex. '#000000' if the colour won't parse, same as before.
 */
let scratch: CanvasRenderingContext2D | null | undefined;
function toHex(color: string): string {
	// One scratch canvas, reused. A fresh 2D context per call is a native allocation the JS heap
	// measurement cannot see, and this runs on most track changes (via `applyArtworkAccent`).
	if (scratch === undefined) {
		const c = document.createElement('canvas');
		c.width = c.height = 1;
		scratch = c.getContext('2d', { willReadFrequently: true });
	}
	const ctx = scratch;
	if (!ctx) return '#000000';
	ctx.fillStyle = '#000000'; // stays, if the engine can't parse what comes next
	ctx.fillStyle = color;
	ctx.fillRect(0, 0, 1, 1);
	const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
	return '#' + [r, g, b].map((c) => c.toString(16).padStart(2, '0')).join('');
}

/**
 * Re-read the tokens. Called after every apply, and by the settings modal on open: toggling
 * light/dark doesn't route through here, and a palette's --primary differs between the two.
 */
export function readBack(): void {
	const cs = getComputedStyle(document.documentElement);
	const g = (n: string) => cs.getPropertyValue(n).trim();
	effective.hue = parseFloat(g('--hue')) || 0;
	effective.radius = parseFloat(g('--radius')) || 0;
	effective.accent = toHex(g('--primary'));
	effective.fontSans = g('--font-sans');
	effective.fontHeading = g('--font-heading');
}

/** Write the accent quartet as inline vars on <html>, foreground picked for legibility on it. */
function setAccentVars(color: string): void {
	const fg = isLight(color) ? ON_LIGHT : ON_DARK;
	const root = document.documentElement;
	root.style.setProperty('--primary', color);
	root.style.setProperty('--primary-foreground', fg);
	root.style.setProperty('--accent', color);
	root.style.setProperty('--accent-foreground', fg);
}

function apply(): void {
	const t = THEMES.find((x) => x.id === theme.id) ?? THEMES[0];
	const root = document.documentElement;
	// Reset every mechanism first, so switching between an accent and a palette (or clearing a
	// custom override) never leaves the previous choice's inline vars or class behind.
	[...ACCENT_VARS, ...CUSTOM_VARS, '--art-h'].forEach((v) => root.style.removeProperty(v));
	root.classList.remove(...PALETTE_CLASSES, TINT_CLASS);

	if (t.kind === 'accent') {
		root.style.setProperty('--primary', t.color);
		root.style.setProperty('--primary-foreground', t.fg);
		root.style.setProperty('--accent', t.color);
		root.style.setProperty('--accent-foreground', t.fg);
	} else {
		root.classList.add(`theme-${t.id}`);
	}

	if (custom.accent) setAccentVars(custom.accent);
	// Last, so the artwork wins while it's on and the user's own theme is back the moment it isn't.
	if (art) setArtVars(art);
	if (custom.hue !== null) root.style.setProperty('--hue', String(custom.hue));
	if (custom.radius !== null) root.style.setProperty('--radius', `${custom.radius}rem`);
	if (custom.fontSans) root.style.setProperty('--font-sans', custom.fontSans);
	if (custom.fontHeading) root.style.setProperty('--font-heading', custom.fontHeading);

	readBack();
}

export function applyTheme(id: ThemeId): void {
	theme.id = THEMES.some((t) => t.id === id) ? id : THEMES[0].id;
	apply();
	localStorage.setItem(KEY, theme.id);
}

const persist = () => localStorage.setItem(CUSTOM_KEY, JSON.stringify(custom));

export function setCustom(patch: Partial<Custom>): void {
	Object.assign(custom, patch);
	apply();
	persist();
}

/** Drops the overrides. Loaded font *files* stay: they're assets, not a setting. */
export function resetCustom(): void {
	setCustom({ accent: null, hue: null, radius: null, fontSans: null, fontHeading: null });
}

/** True when nothing is overridden, so the UI can disable the reset. */
export function isDefaultCustom(): boolean {
	return !custom.accent && custom.hue === null && custom.radius === null && !custom.fontSans && !custom.fontHeading;
}

// --- Font files loaded from disk -------------------------------------------------------------
// Each file becomes an @font-face keyed on its filename, so it shows up in both font dropdowns
// like a bundled family. Only the path is stored: re-reading the file each launch keeps a 4 MB
// variable font out of localStorage, at the cost of the entry going dead if the file moves.

const FONT_STYLE_ID = 'custom-font-files';

/** Family name for a loaded file: its base name, minus extension and anything CSS-unsafe. */
export function fileFamily(path: string): string {
	const base = path.split(/[\\/]/).pop() ?? path;
	// ponytail: two files with the same name collide on one family — last one registered wins.
	return base.replace(/\.[^.]+$/, '').replace(/[^\w \-]/g, '').trim() || 'Custom font';
}

/** Loaded files as font-dropdown entries, so they sit alongside the bundled ones. */
export function fileFonts(): { label: string; value: string }[] {
	return custom.fontFiles.map((p) => ({
		label: fileFamily(p),
		value: `'${fileFamily(p)}', sans-serif`
	}));
}

/** Drop a path from the library, and clear any font row still pointing at its family. */
function forget(path: string): void {
	custom.fontFiles = custom.fontFiles.filter((p) => p !== path);
	const gone = `'${fileFamily(path)}', sans-serif`;
	if (custom.fontSans === gone) custom.fontSans = null;
	if (custom.fontHeading === gone) custom.fontHeading = null;
}

/**
 * (Re)build the `@font-face` rules, and forget any file that has since been deleted or moved —
 * the grant is what tells us, since it checks the file exists. Without the pruning, a font that is
 * no longer on disk keeps its dropdown entry and its row keeps *claiming* to use it while the app
 * silently renders the fallback. Runs at startup and whenever the settings modal opens.
 */
export async function registerFontFiles(): Promise<void> {
	const rules: string[] = [];
	const missing: string[] = [];
	for (const path of custom.fontFiles) {
		try {
			// Grant the URL before the rule exists: a font that 403s once is never retried.
			await allowFontFile(path);
		} catch {
			missing.push(path);
			continue;
		}
		rules.push(
			`@font-face { font-family: '${fileFamily(path)}'; src: url('${convertFileSrc(path)}'); }`
		);
	}
	if (missing.length) {
		missing.forEach(forget);
		persist();
		apply(); // a row that pointed at a missing font falls back to the preset's, visibly
	}
	let el = document.getElementById(FONT_STYLE_ID);
	if (!el) {
		el = document.createElement('style');
		el.id = FONT_STYLE_ID;
		document.head.append(el);
	}
	el.textContent = rules.join('\n');
}

/** Load a font file the user picked. Throws (for the caller to report) if it can't be granted. */
export async function addFontFile(path: string): Promise<string> {
	await allowFontFile(path);
	if (!custom.fontFiles.includes(path)) custom.fontFiles.push(path);
	persist();
	await registerFontFiles();
	return fileFamily(path);
}

export function removeFontFile(path: string): void {
	forget(path);
	persist();
	apply();
	registerFontFiles();
}

/** First family in a font stack, unquoted — what the UI shows and matches on. */
export function familyName(stack: string): string {
	return (stack.split(',')[0] ?? '').replace(/["']/g, '').trim();
}

/**
 * Is this font family installed? Renders a string in it and compares the width against a fallback.
 * ponytail: a custom font that happens to measure exactly like monospace reads as missing. It's a
 * hint next to the input, not a gate — the font is applied either way.
 */
export function fontAvailable(name: string): boolean {
	const ctx = document.createElement('canvas').getContext('2d');
	if (!ctx || !name.trim()) return true;
	const probe = 'mmmmmmmmmmlli';
	ctx.font = '72px monospace';
	const base = ctx.measureText(probe).width;
	ctx.font = `72px "${name}", monospace`;
	return ctx.measureText(probe).width !== base;
}

// --- Artwork colours --------------------------------------------------------------------------
// The playing cover's colour, layered on top of the preset and the custom accent, so switching it
// off puts the user's own theme straight back. Not persisted: it's derived from whatever is
// playing, and the next track overwrites it.
//
// Two things come out of one colour: the accent quartet (inline vars, as everywhere else) and
// --art-h, the hue every surface in the `.art-tint` rules is derived from (layout.css).
//
// The crossfade between tracks is CSS, not JS: `--primary` and `--accent` are registered with
// @property in layout.css, so setting them once starts an interpolation the engine owns. This used
// to be a requestAnimationFrame loop, which meant ~36 style invalidations of the whole document,
// driven from the main thread, landing exactly on the frames the track change was already paying
// for. All that is left here is picking the target and keeping the hue continuous.
//
// --art-h is set the same way but is NOT transitioned: every surface is derived from it, so
// animating it restyled the whole document once a frame and WebKitGTK kept ~26 MB of that per
// track change, permanently. The numbers are in the `html.art-tint` comment in layout.css.
// `nearestHue` stays because the value written here is unwrapped either way, and a future
// crossfade would need it again.

let art: { h: number; hex: string } | null = null;
let wanted = '';

// The window can be closed to the tray with playback carrying on, and WebKitGTK does not tell the
// page (`document.visibilityState` stays "visible"), so Rust does: `ui-visible`, from tray.rs.
// Restyling the document for a window nobody can see is not just wasted work: the web process
// holds on to every one of those restyles until the window is mapped again, so a night in the tray
// grows unbounded and then hands it all back the moment the user opens the app.
let uiVisible = true;
let restyleWanted = false;

/** Skip the artwork restyle while the window is hidden; run the last one when it comes back. */
function restyle(): void {
	if (!uiVisible) {
		restyleWanted = true;
		return;
	}
	apply();
}

export function setUiVisible(visible: boolean): void {
	uiVisible = visible;
	if (visible && restyleWanted) {
		restyleWanted = false;
		apply();
	}
}

/** Push the current artwork colour into the accent vars and the tint hue. */
function setArtVars(c: { h: number; hex: string }): void {
	setAccentVars(c.hex);
	document.documentElement.style.setProperty('--art-h', c.h.toFixed(1));
	document.documentElement.classList.add(TINT_CLASS);
}

/**
 * Point the theme at a cover URL. `null`/undefined (setting off, nothing playing) drops the layer
 * immediately; an unreadable or colourless image leaves the current colours alone rather than
 * flashing to grey.
 */
export function applyArtworkAccent(url: string | undefined | null): void {
	wanted = url ?? '';
	if (!url) {
		if (art) {
			art = null;
			restyle();
		}
		return;
	}
	artworkAccent(url).then((hex) => {
		if (!hex || wanted !== url) return; // colourless cover, or a faster track change already won
		if (art?.hex === hex) return; // same colour (a repeat, or the queue moved under us)
		const hsv = hexToHsv(hex);
		if (!hsv) return;
		// Continuous, never rewrapped: the CSS transition on --art-h is a plain number lerp, so the
		// short way round the wheel has to be baked into the value it lands on. The first track has
		// no previous target, so it starts from whatever --art-h currently resolves to.
		const from =
			art?.h ??
			(parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--art-h')) || 0);
		art = { h: nearestHue(from, hsv.h), hex };
		restyle(); // through the normal path, so `effective` and the pickers agree
	});
}

/** Decode a cover the user is about to hear, so its colour is ready the instant the track flips. */
export function prewarmArtworkAccent(url: string | undefined | null): void {
	if (url) warmAccent(url);
}

/** Apply the stored theme + customization on startup (defaults to rose, no overrides). */
export function initTheme(): void {
	const stored = localStorage.getItem(KEY) as ThemeId | null;
	theme.id = stored && THEMES.some((t) => t.id === stored) ? stored : 'rose';
	try {
		const saved = JSON.parse(localStorage.getItem(CUSTOM_KEY) ?? '{}');
		// Only keys we know about, only the shape we expect: a hand-edited or older localStorage
		// entry must not be able to write arbitrary properties into the inline style.
		for (const k of ['accent', 'fontSans', 'fontHeading'] as const) {
			if (typeof saved?.[k] === 'string') custom[k] = saved[k];
		}
		for (const k of ['hue', 'radius'] as const) {
			if (typeof saved?.[k] === 'number') custom[k] = saved[k];
		}
		if (Array.isArray(saved?.fontFiles)) {
			custom.fontFiles = saved.fontFiles.filter((p: unknown) => typeof p === 'string');
		}
	} catch {
		// unparseable — start clean
	}
	try {
		const saved = JSON.parse(localStorage.getItem(APPEARANCE_KEY) ?? '{}');
		for (const k of ['artworkBackground', 'tabbedPlayer', 'openPlayerOnPlay', 'artworkAccent'] as const) {
			if (typeof saved?.[k] === 'boolean') appearance[k] = saved[k];
		}
	} catch {
		// unparseable — keep the defaults
	}
	apply();
	// Async (each file needs its URL granted first), so the app paints in the fallback font for a
	// frame or two before a loaded font swaps in.
	if (custom.fontFiles.length) registerFontFiles();
}
