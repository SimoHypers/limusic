// Canonical spelling for the app's keyboard shortcuts, shared by the global-hotkeys recorder and
// the editable in-app bindings so the two cannot drift apart.
//
// A combo is built from `KeyboardEvent.code`, not `key`: a binding made on a US layout keeps
// pointing at the same physical key on QWERTZ or AZERTY. The tokens are the modifiers
// `Ctrl`/`Alt`/`Shift`/`Super` (Super is ⌘ on macOS) joined to a key token by `+`, e.g.
// `Super+Shift+M`, `F5`, `;`.
//
// Everything here is pure: no runes, no DOM beyond the `KeyboardEvent` (and `navigator.platform`)
// handed in, so it is exercised by `keycombo.check.ts` without a browser.

export const IS_MAC = typeof navigator !== 'undefined' && navigator.platform.startsWith('Mac');

/** The modifier the app's own shortcuts hang off on this machine. */
export const PRIMARY = IS_MAC ? 'Super' : 'Ctrl';

/** Bare keys that arrive as a `code` and keep their name. */
const NAMED_KEYS: Record<string, true> = {
	ArrowUp: true,
	ArrowDown: true,
	ArrowLeft: true,
	ArrowRight: true,
	PageUp: true,
	PageDown: true,
	Home: true,
	End: true,
	Insert: true,
	Delete: true,
	Space: true,
	Enter: true,
	Tab: true,
	Backspace: true,
	Escape: true
};

/** Punctuation `code` → the character printed on the key. Matches the backend's `parse_key_code`,
 *  so a combo means the same thing to the recorder, the local matcher and a global hotkey. */
const PUNCTUATION: Record<string, string> = {
	Minus: '-',
	Equal: '=',
	BracketLeft: '[',
	BracketRight: ']',
	Backslash: '\\',
	Semicolon: ';',
	Quote: "'",
	Comma: ',',
	Period: '.',
	Slash: '/'
};

/**
 * Physical code → canonical key token. `null` for anything the app does not bind, which is also
 * how the recorder knows to keep waiting.
 */
export function keyTokenFromCode(code: string): string | null {
	if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
	if (/^Key[A-Z]$/.test(code)) return code.slice(3);
	if (/^Digit[0-9]$/.test(code)) return code.slice(5);
	if (/^Numpad[0-9]$/.test(code)) return code;
	if (NAMED_KEYS[code]) return code;
	return PUNCTUATION[code] ?? null;
}

const MODIFIER_CODES: Record<string, true> = {
	ControlLeft: true,
	ControlRight: true,
	ShiftLeft: true,
	ShiftRight: true,
	AltLeft: true,
	AltRight: true,
	MetaLeft: true,
	MetaRight: true
};

/**
 * KeyboardEvent → canonical combo, or `null` for a modifier on its own or an unbindable key.
 *
 * `requireModifier` is the global-hotkeys rule: a bare key grabbed system-wide would be taken from
 * every other app, so only F-keys are allowed without Ctrl/Alt/Super. The app's own shortcuts pass
 * `false` — they are local, so a bare `Space` or `;` is fine.
 */
export function comboFromEvent(e: KeyboardEvent, requireModifier = false): string | null {
	if (MODIFIER_CODES[e.code] || ['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return null;
	const key = keyTokenFromCode(e.code);
	if (!key) return null;
	if (requireModifier && !e.ctrlKey && !e.altKey && !e.metaKey && !/^F\d/.test(key)) return null;
	const parts: string[] = [];
	if (e.ctrlKey) parts.push('Ctrl');
	if (e.altKey) parts.push('Alt');
	if (e.shiftKey) parts.push('Shift');
	if (e.metaKey) parts.push('Super');
	parts.push(key);
	return parts.join('+');
}

/** Split a combo into its modifier tokens and the key token. */
export function parseCombo(combo: string): { mods: string[]; key: string } {
	const parts = combo.split('+').map((s) => s.trim()).filter(Boolean);
	const key = parts.pop() ?? '';
	return { mods: parts, key };
}

/**
 * Keys where Shift is layout noise rather than part of the binding: `>` is Shift+`.` on a US
 * layout but its own key elsewhere, and the same goes for `<` and `+`. Collapsing the Shift here is
 * why one binding answers to both `⌘>` and `⌘.`, the way the shortcuts always have.
 */
const SHIFT_INSENSITIVE: Record<string, true> = { '.': true, ',': true, '=': true };

/** Case-folded, modifier-order-independent form used for comparing and de-duplicating combos.
 *  The order is not fixed because a default is written `Super+Shift+M` and a recorded event
 *  arrives `Shift+Super+M`; both have to compare equal. */
export function normalizeCombo(combo: string): string {
	const { mods, key } = parseCombo(combo);
	const kept = SHIFT_INSENSITIVE[key] ? mods.filter((m) => m !== 'Shift') : mods;
	return [...kept.map((m) => m.toLowerCase()).sort(), key.toLowerCase()].join('+');
}

/**
 * Whether the app's own dispatcher will ever fire this combo. The recorder and the dispatcher both
 * read it, so a binding the recorder saves is one the dispatcher handles — otherwise the row would
 * show a shortcut that silently does nothing.
 *
 * Three shapes are unusable locally:
 *
 * - **An empty combo** (no key token): there is nothing to press, and the dispatcher never
 *   produces one, so malformed storage must not adopt it.
 * - **Shift and a bare key** (`⇧K`, `⇧Space`): with no other modifier that is a character or a
 *   caret movement, not a command, so the dispatcher only accepts it for F-keys — and then only
 *   because nothing types with them.
 * - **Ctrl or Super with Alt** (`⌃⌥K`): that belongs to the global hotkeys, and on Windows it is
 *   how AltGr arrives to type a character.
 */
export function isLocalBindable(combo: string): boolean {
	const { mods, key } = parseCombo(combo);
	if (!key) return false;
	const hasPrimary = mods.includes('Ctrl') || mods.includes('Super');
	if (hasPrimary && mods.includes('Alt')) return false;
	return !(mods.includes('Shift') && !hasPrimary && !mods.includes('Alt') && !/^F\d/.test(key));
}

const MAC_MOD: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' };
const PLAIN_MOD: Record<string, string> = { Ctrl: 'Ctrl', Alt: 'Alt', Shift: 'Shift', Super: 'Super' };
// Primary modifier first on macOS so it reads ⌘⇧M, the way the shortcuts always did; Apple's order
// (⌃⌥⇧⌘) elsewhere.
const MAC_ORDER = ['Super', 'Ctrl', 'Alt', 'Shift'];
const PLAIN_ORDER = ['Ctrl', 'Alt', 'Shift', 'Super'];

/** Key token → how it reads in a badge. */
const KEY_LABEL: Record<string, string> = {
	Space: 'SPACE',
	Semicolon: ';',
	Comma: '<',
	Period: '>',
	Equal: '+',
	Minus: '-',
	Slash: '/',
	BracketLeft: '[',
	BracketRight: ']',
	Backslash: '\\',
	Quote: "'",
	ArrowUp: '↑',
	ArrowDown: '↓',
	ArrowLeft: '←',
	ArrowRight: '→',
	PageUp: 'PgUp',
	PageDown: 'PgDn',
	Escape: 'Esc'
};

/**
 * The combo as one string to show the user: `⌘⇧M` on macOS, `Ctrl+Shift+M` elsewhere. The key is
 * mapped to what the layout prints (`Equal` → `+`, `Period` → `>`), so a row shows the key you
 * would actually reach for.
 */
export function formatCombo(combo: string): string {
	const { mods, key } = parseCombo(combo);
	const order = IS_MAC ? MAC_ORDER : PLAIN_ORDER;
	const sorted = order.filter((m) => mods.includes(m));
	const label = KEY_LABEL[key] ?? key;
	if (IS_MAC) return sorted.map((m) => MAC_MOD[m]).join('') + label;
	return [...sorted.map((m) => PLAIN_MOD[m]), label].join('+');
}
