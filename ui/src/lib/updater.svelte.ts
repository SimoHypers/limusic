// Auto-update via Tauri's updater plugin. Checks a signed latest.json on GitHub Releases; the
// startup check is silent unless an update exists, the Settings check always reports a result.
// Only self-updates the AppImage build on Linux (Tauri limitation) — the .rpm can't self-update.
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { toast } from './player.svelte';

export const updateState = $state({
	available: null as { version: string } | null, // set when a newer version is waiting
	checking: false, // Settings "Check for updates" is in flight
	installing: false // downloading/installing the update
});

// The resolved handle to download; kept out of reactive state (it's not serializable/renderable).
let pending: Update | null = null;

async function look(): Promise<boolean> {
	const u = await check();
	if (u) {
		pending = u;
		updateState.available = { version: u.version };
		return true;
	}
	return false;
}

/** On app open: show the update toast if one exists, stay silent otherwise. */
export async function checkForUpdatesQuiet() {
	try {
		await look();
	} catch (e) {
		console.error('update check failed', e); // no endpoint / offline — don't nag on launch
	}
}

/** From Settings: always tell the user the outcome. */
export async function checkForUpdatesInteractive() {
	updateState.checking = true;
	try {
		if (!(await look())) toast('You are running the latest version');
	} catch (e) {
		toast(`Update check failed: ${e}`);
	} finally {
		updateState.checking = false;
	}
}

/** Download + install the pending update, then relaunch into the new version. */
export async function installUpdate() {
	if (!pending) return;
	updateState.installing = true;
	try {
		await pending.downloadAndInstall();
		await relaunch();
	} catch (e) {
		toast(`Update failed: ${e}`);
		updateState.installing = false;
	}
}
