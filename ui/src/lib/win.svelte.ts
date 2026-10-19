// Shared window-frame state: the resize borders hide when maximized, and the root container drops
// its rounded corners. `chrome` says whether the compositor is drawing the frame instead of us.
// One listener, initialized once by the root layout.
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getSettings, onUiVisible } from '$lib/api';
import { setUiVisible } from '$lib/theme.svelte';
import { invoke } from '@tauri-apps/api/core';
import { ui } from '$lib/player.svelte';

/** Who draws the window frame. `off` = our custom titlebar owns it (the default), `on` = the
 *  compositor does (the "system title bar" setting, Linux/Windows), `overlay` = macOS traffic
 *  lights float over our bar. Anything but `off` hides our window buttons and corner rounding.
 *  Derived in Rust (`native_chrome` in commands.rs) so the SPA needs no platform check. */
export const win = $state({
	maximized: false,
	chrome: 'off' as 'off' | 'on' | 'overlay',
	// The OS rounds our undecorated window itself (Windows 11, `os_corners` in commands.rs, #504).
	osCorners: false,
	// Which of our window buttons the desktop's GTK layout names (Linux, `BUTTON_LAYOUT` in
	// commands.rs, #481). Close always shows.
	minimize: true,
	maximize: true
});

/** The window's corner rounding, as a class. The root layout wears it, and so does every backdrop
 *  that paints the whole window: those portal to <body>, outside the root's clip, and squared the
 *  corners off while a dialog was open (#504). Theater mode counts as maximized (#139).
 *  12px, not `rounded-lg`: that resolves to --radius, which every theme sets differently. This is
 *  the GNOME/Adwaita value (#65). */
export function windowCorners(): string {
	return win.maximized || ui.theaterOpen || win.chrome !== 'off' || win.osCorners
		? ''
		: 'rounded-[12px]';
}

let started = false;

export function initWin(): () => void {
	if (started) return () => {};
	started = true;
	const w = getCurrentWindow();
	// The window is created hidden (tauri.conf.json) so the window-state plugin can restore the
	// saved size before anything is on screen: it only restores once the webview is ready, so a
	// visible window would flash at the config's 1200x800, white, then snap (#45). By here the SPA
	// has mounted, so showing it now costs nothing and skips the flash.
	//
	// Never swallow this one. The reveal used to be `w.show()`, which is ACL-gated, and a silent
	// catch here hid a missing `core:window:allow-show` for a week: it fell through to lib.rs's
	// safety net, so every launch sat on an empty desktop while the tray and the media keys already
	// worked (#122). `show_main` also keeps the window in the tray on a login launch with "Start
	// minimized to tray" on, and says which it did, so the page knows whether anyone can see it.
	// Settled before the reveal, so the window never flashes rounded corners and a second set of
	// window buttons on its way to the system frame. `finally`: the show must happen even if the
	// settings call fails, or a backend hiccup leaves the app with no window at all (#122).
	getSettings()
		.then((s) => {
			if (s.native_chrome === 'on' || s.native_chrome === 'overlay') win.chrome = s.native_chrome;
			win.osCorners = s.os_corners === 'true';
			const layout = s.button_layout;
			if (layout !== undefined) {
				win.minimize = layout.includes('minimize');
				win.maximize = layout.includes('maximize');
			}
		})
		.catch(() => {})
		.finally(() =>
			invoke<boolean>('show_main')
				.then((visible) => setUiVisible(visible))
				.catch((e) => console.error('window show failed', e))
		);
	const sync = () =>
		w
			.isMaximized()
			.then((m) => (win.maximized = m))
			.catch(() => {});
	sync();
	const un = w.onResized(sync);
	// Hidden to the tray is not a state the page can see for itself (WebKitGTK keeps reporting the
	// document as visible), and work done for a hidden window is memory the web process keeps until
	// it is shown again. See `theme.svelte.ts`.
	const unVis = onUiVisible(setUiVisible);
	return () => {
		un.then((u) => u());
		unVis.then((u) => u());
	};
}
