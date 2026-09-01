// Auto-update via Tauri's updater plugin. Checks a signed latest.json on GitHub Releases; the
// startup check is silent unless an update exists, the Settings check always reports a result.
// Only self-updates the AppImage build on Linux (Tauri limitation) — .deb, .rpm and distro packages
// update through their package manager, so they get a download link instead. See `canInstall`.
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { toast } from './player.svelte';
import { t } from './i18n.svelte';
import { canSelfUpdate, getSettings, openExternal, releaseNotes } from './api';
import { getVersion } from '@tauri-apps/api/app';

const RELEASES_URL = 'https://github.com/SimoHypers/limusic/releases/latest';

/** How often the quiet check repeats while the app stays open. */
export const QUIET_INTERVAL_MS = 6 * 60 * 60 * 1000;

export const updateState = $state({
	available: null as { version: string } | null, // set when a newer version is waiting
	canInstall: true, // false on packaged Linux builds; always resolved before `available` is set
	checking: false, // Settings "Check for updates" is in flight
	installing: false // downloading/installing the update
});

// The resolved handle to download; kept out of reactive state (it's not serializable/renderable).
let pending: Update | null = null;

/** `a` is a later release than `b`. Both are plain `x.y.z` from our own releases; anything that
 *  doesn't parse compares as not-newer, so a weird tag can never invent an update. */
function isNewer(a: string, b: string): boolean {
	const pa = a.split('.').map(Number);
	const pb = b.split('.').map(Number);
	for (let i = 0; i < 3; i++) {
		const [x, y] = [pa[i] ?? 0, pb[i] ?? 0];
		if (x !== y) return x > y;
	}
	return false;
}

async function look(): Promise<boolean> {
	let u: Update | null;
	try {
		u = await check();
	} catch (e) {
		// The plugin resolves this platform's entry in latest.json BEFORE it compares versions, so a
		// release whose manifest is missing the entry (a CI leg failed, or is still running) makes
		// every check throw. The quiet check swallows that, which silently leaves the whole platform
		// with no update prompt until some later release fixes the manifest. v0.6.6 shipped without
		// `darwin-aarch64` and did exactly that to every Mac. So ask the releases API instead: it
		// doesn't read the manifest. Nothing signed is reachable for us to install, so the banner
		// can only offer the download page. If that call fails too (offline, rate-limited), its
		// error propagates and the check reports as failed, which it did.
		console.error('update manifest unusable, falling back to the releases API', e);
		const latest = (await releaseNotes())[0]?.version;
		if (!latest || !isNewer(latest, await getVersion())) return false;
		updateState.canInstall = false;
		updateState.available = { version: latest };
		return true;
	}
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

/** On app open, and every `QUIET_INTERVAL_MS` after: show the update banner if one exists, stay
 *  silent otherwise. Repeating matters because ✕ hides to tray by default, so the webview mounts
 *  once and can stay up for days: a mount-only check never sees a release published while the app
 *  is running. With `update_banner` off the check is skipped entirely (no banner, no request),
 *  leaving Settings > About > Check for updates as the only way to find one. */
export async function checkForUpdatesQuiet() {
	try {
		if (updateState.available) return; // one is already on screen; don't re-fetch behind it
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
