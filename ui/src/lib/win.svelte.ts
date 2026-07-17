// Shared window-maximized state: the resize borders hide when maximized, and the root container
// drops its rounded corners. One listener, initialized once by the root layout.
import { getCurrentWindow } from '@tauri-apps/api/window';

export const win = $state({ maximized: false });

let started = false;

export function initWin(): () => void {
	if (started) return () => {};
	started = true;
	const w = getCurrentWindow();
	const sync = () =>
		w
			.isMaximized()
			.then((m) => (win.maximized = m))
			.catch(() => {});
	sync();
	const un = w.onResized(sync);
	return () => un.then((u) => u());
}
