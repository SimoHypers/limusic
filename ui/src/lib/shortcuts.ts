// App-wide keyboard shortcuts. One window listener: the unmodified keys (space, ;) bail out on a
// typing target first, everything else is gated on Ctrl/Cmd, so a key typed into a field costs a
// couple of cheap checks and falls straight through. Zoom keeps its own listener (zoom.ts) because
// it also owns the ctrl+wheel gesture.
import { browser } from '$app/environment';
import * as api from './api';
import { cycleRepeat, np, nudgeVolume, playback, refreshView, toggleMute, ui } from './player.svelte';

export const IS_MAC = browser && navigator.platform.startsWith('Mac');

/** How this machine writes the modifier these shortcuts hang off, for anything that shows a key
 *  hint. Mac takes the bare glyph; everywhere else the `+` is part of the spelling. */
export const MOD = IS_MAC ? '⌘' : 'Ctrl+';

/** macOS keeps ⌘H for the system "hide the window", so the shortcuts list answers to ⌘/ there. */
export const HELP_KEY = IS_MAC ? '/' : 'H';

/** The whole combo that opens the shortcuts list, spelled for this machine. */
export const HELP_COMBO = `${MOD}${HELP_KEY}`;

/** What a key event means, whatever layout is active. A Latin letter is used as typed (so Dvorak and
 *  AZERTY keep working); a character from another script (й, р, ю, б) falls back to the physical key
 *  from e.code. Everything else (named keys, digits, symbols) is returned as e.key. */
export const keyOf = (e: KeyboardEvent): string => {
	const k = e.key;
	if (k.length === 1 && /[a-z]/i.test(k)) return k.toLowerCase();
	const nonAscii = k.length === 1 && k.charCodeAt(0) > 127;
	if (!nonAscii) return k;
	if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3).toLowerCase();
	if (e.code === 'Period') return '.';
	if (e.code === 'Comma') return ',';
	if (e.code === 'Slash') return '/';
	return k;
};

/** `HELP_KEY` as the event reports it. A letter arrives in either case; `/` only ever as itself. */
const isHelpKey = (e: KeyboardEvent) => keyOf(e) === HELP_KEY.toLowerCase();

/** macOS keeps ⌘M for the system "minimize the window", so mute asks for ⇧ on top there. */
export const MUTE_COMBO = IS_MAC ? `${MOD}⇧M` : `${MOD}M`;

/** Mute's key, shift and all. Elsewhere ⇧ is ignored, the way it always was for these letters. */
const isMuteKey = (e: KeyboardEvent) => keyOf(e) === 'm' && (!IS_MAC || e.shiftKey);

/** Percent per press, matching a step of the volume slider's arrow keys. */
const VOLUME_STEP = 5;

/** Somewhere a bare space or `;` is a character, not a command. */
export const typing = (t: EventTarget | null) =>
	t instanceof HTMLElement &&
	(t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName));

/** Where focus a click left behind stays put: see `onFocus` below. */
const KEEPS_FOCUS =
	'[role=dialog],[role=alertdialog],[role=menu],[role=listbox],[data-selected],[data-track-selection]';

/** `mini` = the mini-player window: same transport keys, minus the ones that toggle a piece of
 *  chrome that window doesn't render (palette, shortcut list, now-playing view). */
export function initShortcuts(mini = false) {
	const onKey = (e: KeyboardEvent) => {
		// Focused controls (including track selection) have already handled this key.
		if (e.defaultPrevented) return;
		// F5 reloads the page here for the same reason it does in a browser, and like a browser it
		// works from inside a text field too. Ctrl+R is not a second way in: that key cycles repeat.
		// The mini widget has no page to reload, so it keeps the key for the OS.
		if (!mini && e.key === 'F5') {
			refreshView();
			e.preventDefault();
			return;
		}
		if (!e.ctrlKey && !e.metaKey) {
			// Space also activates a focused button and scrolls the page, so it is swallowed either
			// way once we know it isn't being typed.
			if (e.key !== ' ' && e.key !== ';') return;
			if (typing(e.target) || e.altKey || e.shiftKey) return;
			api.togglePause();
			e.preventDefault();
			return;
		}
		// Ctrl+Alt belongs to the global hotkeys (Ctrl+Alt+M would otherwise mute here too and the
		// two toggles cancel out), and on Windows it is also how AltGr arrives, typing a character.
		if (e.altKey) return;
		if (mini && (['k', 'e'].includes(keyOf(e)) || isHelpKey(e))) return;
		// Out of the switch because the key is per-platform: on macOS ⌘H has to fall through
		// untouched, so the window still hides.
		if (isHelpKey(e)) {
			ui.shortcutsOpen = !ui.shortcutsOpen;
			e.preventDefault();
			return;
		}
		// Out of the switch for the same reason, and it has to read the whole event: on macOS a
		// bare ⌘M falls through so AppKit still minimizes, and only ⌘⇧M mutes.
		if (isMuteKey(e)) {
			toggleMute();
			e.preventDefault();
			return;
		}
		switch (keyOf(e)) {
			// Real quit, unlike the window's X which hides to tray.
			case 'q':
				// Holding the keys auto-repeats keydown; one quit request is enough.
				if (!e.repeat) api.quitApp();
				break;
			// Toggles, so the key that opened the palette also dismisses it.
			case 'k':
				ui.paletteOpen = !ui.paletteOpen;
				break;
			case 'e':
				// With nothing playing there is no view to open (the layout renders it behind
				// `playback.now`), and flipping the flag anyway would ambush the next play.
				if (!playback.now) return;
				np.open = !np.open;
				break;
			case 'f':
				api.nextTrack();
				break;
			case 'd':
				api.prevTrack();
				break;
			case 's':
				api.toggleShuffle();
				break;
			case 'r':
				cycleRepeat();
				break;
			// Shift+. and Shift+, on a US layout. The unshifted keys are accepted too, so the
			// shortcut still works on layouts that put > and < somewhere else.
			case '>':
			case '.':
				nudgeVolume(VOLUME_STEP);
				break;
			case '<':
			case ',':
				nudgeVolume(-VOLUME_STEP);
				break;
			default:
				return;
		}
		e.preventDefault();
	};
	// A click leaves focus on what it hit (a track row, a tab, the artwork), and every key after it
	// went there: Space replayed the clicked row instead of pausing (#551), Enter clicked the artwork
	// again, and the keypress lit a focus ring. So focus that lands while the pointer is driving is
	// dropped, including a menu handing it back to its trigger on close. Fields keep it, and so do open
	// menus, lists and dialogs, and track select mode (its rows and bar take Space, Escape and Ctrl+A
	// themselves). Tab hands focus back to the keyboard until the next click, so the app can still be
	// walked with a ring.
	let pointer = true;
	const onPointer = () => (pointer = true);
	const onTab = (e: KeyboardEvent) => {
		if (e.key === 'Tab') pointer = false;
	};
	const onFocus = (e: FocusEvent) => {
		const t = e.target;
		if (!pointer || !(t instanceof HTMLElement) || typing(t)) return;
		if (t.closest(KEEPS_FOCUS)) return;
		// On the next tick, not here: WebKit and Chromium count focus moved inside a focus handler
		// as a blocked focus change and swallow the mousedown that caused it, so no drag could start
		// on a track row, a queue row or a card. Unless focus has already gone somewhere else.
		setTimeout(() => document.activeElement === t && t.blur());
	};
	window.addEventListener('keydown', onKey);
	window.addEventListener('pointerdown', onPointer, true);
	window.addEventListener('keydown', onTab, true);
	window.addEventListener('focusin', onFocus);
	return () => {
		window.removeEventListener('keydown', onKey);
		window.removeEventListener('pointerdown', onPointer, true);
		window.removeEventListener('keydown', onTab, true);
		window.removeEventListener('focusin', onFocus);
	};
}
