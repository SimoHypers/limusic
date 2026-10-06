// App-wide keyboard shortcuts. One window listener: the event is turned into a canonical combo
// (keycombo.ts) and looked up in the editable bindings (keybinds.svelte.ts), so what Settings > the
// shortcuts list shows is exactly what fires. Zoom keeps its own listener (zoom.svelte.ts) because
// it also owns the ctrl+wheel gesture, and the global OS-level hotkeys live in hotkeys.svelte.ts.
import * as api from './api';
import { cycleRepeat, np, nudgeVolume, playback, refreshView, toggleMute, ui } from './player.svelte';
import { keybinds } from './keybinds.svelte';
import { comboFromEvent, isLocalBindable, parseCombo, IS_MAC } from './keycombo';
import { setZoom, stepZoom } from './zoom.svelte';

export { IS_MAC };

/** Percent per press, matching a step of the volume slider's arrow keys. */
const VOLUME_STEP = 5;

/** Somewhere a bare space or `;` is a character, not a command. */
const typing = (t: EventTarget | null) =>
	t instanceof HTMLElement &&
	(t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName));

/** The mini window renders none of the chrome these toggle, and it has no page to reload, so it
 *  keeps the bindings for everything else but not these. */
const MINI_EXCLUDED: Record<string, true> = {
	// No page to reload.
	refresh: true,
	// None of this chrome exists in the widget.
	search: true,
	now_playing: true,
	show_list: true,
	// The widget is a fixed-size card; it never initializes zoom.
	zoom_in: true,
	zoom_out: true,
	reset_zoom: true
};

export function initShortcuts(mini = false) {
	keybinds.load();
	const onKey = (e: KeyboardEvent) => {
		// Focused controls (including track selection) have already handled this key.
		if (e.defaultPrevented) return;
		// A rebind owns the keyboard. It stops propagation as well; this is belt and braces.
		if (keybinds.recording) return;
		const combo = comboFromEvent(e);
		if (!combo) return;
		const action = keybinds.actionFor(combo);
		if (!action) return;
		if (mini && MINI_EXCLUDED[action]) return;
		// A combo the recorder refuses to save never fires; the shared predicate keeps the two in
		// step (it drops Shift+<bare key> and Ctrl/Super+Alt).
		if (!isLocalBindable(combo)) return;
		const bare = !(e.ctrlKey || e.metaKey || e.altKey);
		// A bare key is a character in a field. F-keys are the exception: nothing types with them,
		// so like a browser's reload, F5 works from a field too.
		if (bare && !/^F\d/.test(parseCombo(combo).key) && typing(e.target)) return;
		switch (action) {
			case 'refresh':
				refreshView();
				break;
			case 'play_pause':
				api.togglePause();
				break;
			// Toggles, so the key that opened the palette also dismisses it.
			case 'search':
				ui.paletteOpen = !ui.paletteOpen;
				break;
			// With nothing playing there is no view to open (the layout renders it behind
			// `playback.now`), and flipping the flag anyway would ambush the next play.
			case 'now_playing':
				if (!playback.now) return;
				np.open = !np.open;
				break;
			case 'next':
				api.nextTrack();
				break;
			case 'prev':
				api.prevTrack();
				break;
			case 'shuffle':
				api.toggleShuffle();
				break;
			case 'repeat':
				cycleRepeat();
				break;
			case 'mute':
				toggleMute();
				break;
			case 'volume_up':
				nudgeVolume(VOLUME_STEP);
				break;
			case 'volume_down':
				nudgeVolume(-VOLUME_STEP);
				break;
			case 'show_list':
				ui.shortcutsOpen = !ui.shortcutsOpen;
				break;
			// Zoom is matched here like everything else, but deliberately does not preventDefault:
			// the webview's own ctrl+wheel/zoom polyfill still runs underneath, and `setZoom` is what
			// clamps the level to the app's own ceiling (see zoom.svelte.ts).
			case 'zoom_in':
				stepZoom(1);
				return;
			case 'zoom_out':
				stepZoom(-1);
				return;
			case 'reset_zoom':
				setZoom(1);
				return;
			default:
				return;
		}
		e.preventDefault();
	};
	window.addEventListener('keydown', onKey);
	return () => window.removeEventListener('keydown', onKey);
}
