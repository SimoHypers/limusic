// Auto-update via Tauri's updater plugin. Checks a signed latest.json on GitHub Releases; the
// startup check is silent unless an update exists, the Settings check always reports a result.
// Only self-updates the AppImage build on Linux (Tauri limitation) — .deb, .rpm and distro packages
// update through their package manager, so they get a download link instead. See `canInstall`.
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { toast } from './player.svelte';
import { t } from './i18n.svelte';
import { friendlyNetError } from './neterr';
import { canSelfUpdate, getSettings, openExternal, releaseNotes } from './api';
import { getVersion } from '@tauri-apps/api/app';

const RELEASES_URL = 'https://github.com/SimoHypers/limusic/releases/latest';

/** How often the quiet check repeats while the app stays open. */
export const QUIET_INTERVAL_MS = 6 * 60 * 60 * 1000;

export const updateState = $state({
	// Set when Latest differs from this build. `rollback` is true when Latest is OLDER than this
	// build, which is what a pulled release looks like from the inside (see `look()`).
	available: null as { version: string; rollback: boolean } | null,
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
		// `allowDowngrades` is the rollback lever. Without it the plugin compares `remote > current`,
		// so once a broken release installs itself there is no way back: marking an older release
		// Latest moves nobody, and the only fix is shipping another release on top of the broken
		// one. With it the plugin's own comparator becomes `remote != current` (tauri-plugin-updater
		// 2.10.1, `commands.rs`), so whatever release is marked Latest is what every client
		// converges on, newer or older. A rollback is `gh release edit <good-tag> --latest`, plus
		// demoting the pulled release to a prerelease for the fallback below.
		//
		// It has to be armed in the release that might need rescuing, not the rescue: a build
		// without it never takes a downgrade, so this only protects releases from 1.0.0 on.
		//
		// What makes it safe is that nothing moves until the owner moves Latest, and the release
		// workflows only flip Latest once all three platforms are in its latest.json, so Latest is
		// always a complete, signed release. Once a client is on it, `remote != current` is false
		// and it stays put: no loop. The side effect is that any build AHEAD of Latest is offered
		// Latest as a rollback: a dev build after the version bump, or a prerelease if we ever
		// publish one. The first only costs a banner in dev. The second would offer every prerelease
		// tester a downgrade to stable, so publishing prereleases means revisiting this.
		u = await check({ allowDowngrades: true });
	} catch (e) {
		// The plugin resolves this platform's entry in latest.json BEFORE it compares versions, so a
		// release whose manifest is missing the entry (a CI leg failed, or is still running) makes
		// every check throw. The quiet check swallows that, which silently leaves the whole platform
		// with no update prompt until some later release fixes the manifest. v0.6.6 shipped without
		// `darwin-aarch64` and did exactly that to every Mac. So ask the releases API instead: it
		// doesn't read the manifest. Nothing signed is reachable for us to install, so the banner
		// can only offer the download page. If that call fails too (offline, rate-limited), its
		// error propagates and the check reports as failed, which it did.
		//
		// Any difference counts, like the main path above: `isNewer` here would refuse a rollback on
		// exactly the platforms whose manifest is broken. Note the releases API lists by creation
		// date, not by the Latest flag, so this only follows a rollback when the pulled release is
		// also demoted to a prerelease (which `release_notes` filters out).
		console.error('update manifest unusable, falling back to the releases API', e);
		const latest = (await releaseNotes())[0]?.version;
		const current = await getVersion();
		if (!latest || latest === current) return false;
		updateState.canInstall = false;
		updateState.available = { version: latest, rollback: isNewer(current, latest) };
		return true;
	}
	if (u) {
		pending = u;
		// Before `available`, so the banner never renders with the wrong button for a frame. On the
		// (unlikely) IPC failure, fall back to the download link: it works everywhere, while
		// "Update now" on a packaged build does not.
		updateState.canInstall = await canSelfUpdate().catch(() => false);
		updateState.available = { version: u.version, rollback: isNewer(u.currentVersion, u.version) };
		return true;
	}
	return false;
}

/** The line announcing an available version. A rollback worded as "Version 0.8.2 is available!"
 *  to someone on 1.0.0 reads as a bug, so it says what is actually happening. */
export function availableMessage(a: { version: string; rollback: boolean }): string {
	return t(a.rollback ? 'settings.about.rollback_available' : 'settings.about.update_available', {
		version: a.version
	});
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
		if (await look()) return { message: availableMessage(updateState.available!), error: false };
		return { message: t('settings.about.up_to_date'), error: false };
	} catch (e) {
		// Rendered inline in the dialog rather than as a toast, so it misses the toast's own
		// network wording and has to ask for it here.
		const detail = friendlyNetError(String(e), t('errors.unreachable'));
		return { message: t('settings.about.update_check_failed', { error: detail }), error: true };
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
