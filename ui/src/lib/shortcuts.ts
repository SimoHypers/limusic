// App-wide keyboard shortcuts. One window listener: the unmodified keys (space, ;) bail out on a
// typing target first, everything else is gated on Ctrl/Cmd, so a key typed into a field costs a
// couple of cheap checks and falls straight through. Zoom keeps its own listener (zoom.ts) because
// it also owns the ctrl+wheel gesture.
import { browser } from '$app/environment';
import * as api from './api';
import { cycleRepeat, np, nudgeVolume, playback, toggleMute, ui } from './player.svelte';

/** How this machine writes the modifier these shortcuts hang off, for anything that shows a key
 *  hint. Mac takes the bare glyph; everywhere else the `+` is part of the spelling. */
export const MOD = browser && navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl+';

/** Percent per press, matching a step of the volume slider's arrow keys. */
const VOLUME_STEP = 5;

/** Somewhere a bare space or `;` is a character, not a command. */
const typing = (t: EventTarget | null) =>
	t instanceof HTMLElement &&
	(t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName));

export function initShortcuts() {
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
		switch (e.key) {
			// Toggles, so the key that opened the palette also dismisses it.
			case 'k':
			case 'K':
				ui.paletteOpen = !ui.paletteOpen;
				break;
			case 'h':
			case 'H':
				ui.shortcutsOpen = !ui.shortcutsOpen;
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
			case 'm':
			case 'M':
				toggleMute();
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
