// Auto-update via Tauri's updater plugin. Checks a signed latest.json on GitHub Releases; the
// startup check is silent unless an update exists, the Settings check always reports a result.
// Only self-updates the AppImage build on Linux (Tauri limitation) — .deb, .rpm and distro packages
// update through their package manager, so they get a download link instead. See `canInstall`.
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { toast } from './player.svelte';
import { t } from './i18n.svelte';
import { canSelfUpdate, getSettings, openExternal } from './api';

const RELEASES_URL = 'https://github.com/SimoHypers/limusic/releases/latest';

export const updateState = $state({
	available: null as { version: string } | null, // set when a newer version is waiting
	canInstall: true, // false on packaged Linux builds; always resolved before `available` is set
	checking: false, // Settings "Check for updates" is in flight
	installing: false // downloading/installing the update
});

// The resolved handle to download; kept out of reactive state (it's not serializable/renderable).
let pending: Update | null = null;

async function look(): Promise<boolean> {
	const u = await check();
	if (u) {
		pending = u;
		// Before `available`, so the banner never renders with the wrong button for a frame. On the
		// (unlikely) IPC failure, fall back to the download link: it works everywhere, while
		// "Update now" on a packaged build does not.
		updateState.canInstall = await canSelfUpdate().catch(() => false);
		updateState.available = { version: u.version };
		return true;
	}
	return false;
}

/** On app open: show the update banner if one exists, stay silent otherwise. With `update_banner`
 *  off the check is skipped entirely (no banner, no request), leaving Settings > About > Check for
 *  updates as the only way to find one. */
export async function checkForUpdatesQuiet() {
	try {
		if ((await getSettings()).update_banner === 'false') return;
		await look();
	} catch (e) {
		console.error('update check failed', e); // no endpoint / offline — don't nag on launch
	}
}

/** From Settings: return the outcome so the modal can show it inline (a toast renders behind the
 *  dialog). `error` picks the Alert variant. */
export async function checkForUpdatesInteractive(): Promise<{ message: string; error: boolean }> {
	updateState.checking = true;
	try {
		if (await look())
			return { message: `Update available: v${updateState.available!.version}`, error: false };
		return { message: 'You are running the latest version', error: false };
	} catch (e) {
		return { message: `Update check failed: ${e}`, error: true };
	} finally {
		updateState.checking = false;
	}
}

/** Send a packaged build to the releases page. Their package manager does the actual updating; all
 *  the app can do is say a new version exists and get out of the way. */
export function openDownloadPage() {
	openExternal(RELEASES_URL).catch((e) => toast.error(t('toasts.browser_failed', { error: String(e) })));
}

/** Download + install the pending update, then relaunch into the new version. */
export async function installUpdate() {
	if (!pending) return;
	updateState.installing = true;
	try {
		await pending.downloadAndInstall();
		await relaunch();
	} catch (e) {
		toast.error(t('toasts.update_failed', { error: String(e) }));
		updateState.installing = false;
	}
}
