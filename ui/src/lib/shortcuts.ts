// App-wide keyboard shortcuts. One window listener: the unmodified keys (space, ;) bail out on a
// typing target first, everything else is gated on Ctrl/Cmd, so a key typed into a field costs a
// couple of cheap checks and falls straight through. Zoom keeps its own listener (zoom.ts) because
// it also owns the ctrl+wheel gesture.
import { browser } from '$app/environment';
import * as api from './api';
import { cycleRepeat, np, nudgeVolume, playback, toggleMute, ui } from './player.svelte';

const IS_MAC = browser && navigator.platform.startsWith('Mac');

/** How this machine writes the modifier these shortcuts hang off, for anything that shows a key
 *  hint. Mac takes the bare glyph; everywhere else the `+` is part of the spelling. */
export const MOD = IS_MAC ? '⌘' : 'Ctrl+';

/** macOS keeps ⌘H for the system "hide the window", so the shortcuts list answers to ⌘/ there. */
export const HELP_KEY = IS_MAC ? '/' : 'H';

/** The whole combo that opens the shortcuts list, spelled for this machine. */
export const HELP_COMBO = `${MOD}${HELP_KEY}`;

/** `HELP_KEY` as the event reports it. A letter arrives in either case; `/` only ever as itself. */
const isHelpKey = (key: string) => key === HELP_KEY || key === HELP_KEY.toLowerCase();

/** macOS keeps ⌘M for the system "minimize the window", so mute asks for ⇧ on top there. */
export const MUTE_COMBO = IS_MAC ? `${MOD}⇧M` : `${MOD}M`;

/** Mute's key, shift and all. Elsewhere ⇧ is ignored, the way it always was for these letters. */
const isMuteKey = (e: KeyboardEvent) => (e.key === 'm' || e.key === 'M') && (!IS_MAC || e.shiftKey);

/** Percent per press, matching a step of the volume slider's arrow keys. */
const VOLUME_STEP = 5;

/** Somewhere a bare space or `;` is a character, not a command. */
const typing = (t: EventTarget | null) =>
	t instanceof HTMLElement &&
	(t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName));

/** `mini` = the mini-player window: same transport keys, minus the ones that toggle a piece of
 *  chrome that window doesn't render (palette, shortcut list, now-playing view). */
export function initShortcuts(mini = false) {
	const onKey = (e: KeyboardEvent) => {
		if (!e.ctrlKey && !e.metaKey) {
			// Space also activates a focused button and scrolls the page, so it is swallowed either
			// way once we know it isn't being typed.
			if (e.key !== ' ' && e.key !== ';') return;
			if (typing(e.target) || e.altKey || e.shiftKey) return;
			api.togglePause();
			e.preventDefault();
			return;
		}
		if (mini && ('kKeE'.includes(e.key) || isHelpKey(e.key))) return;
		// Out of the switch because the key is per-platform: on macOS ⌘H has to fall through
		// untouched, so the window still hides.
		if (isHelpKey(e.key)) {
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
		switch (e.key) {
			// Toggles, so the key that opened the palette also dismisses it.
			case 'k':
			case 'K':
				ui.paletteOpen = !ui.paletteOpen;
				break;
			case 'e':
			case 'E':
				// With nothing playing there is no view to open (the layout renders it behind
				// `playback.now`), and flipping the flag anyway would ambush the next play.
				if (!playback.now) return;
				np.open = !np.open;
				break;
			case 'f':
			case 'F':
				api.nextTrack();
				break;
			case 'd':
			case 'D':
				api.prevTrack();
				break;
			case 's':
			case 'S':
				api.toggleShuffle();
				break;
			case 'r':
			case 'R':
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
	window.addEventListener('keydown', onKey);
	return () => window.removeEventListener('keydown', onKey);
}
