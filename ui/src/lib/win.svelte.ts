// Shared window-maximized state: the resize borders hide when maximized, and the root container
// drops its rounded corners. One listener, initialized once by the root layout.
import { getCurrentWindow } from '@tauri-apps/api/window';
import { onUiVisible } from '$lib/api';
import { setUiVisible } from '$lib/theme.svelte';

export const win = $state({ maximized: false });

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
	// Never swallow this one. `show` is ACL-gated, and a silent catch here hid a missing
	// `core:window:allow-show` for a week: the reveal fell through to lib.rs's safety net, so every
	// launch sat on an empty desktop while the tray and the media keys already worked (#122).
	w.show().catch((e) => console.error('window show failed', e));
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
