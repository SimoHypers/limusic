// Spotify import (#375): the dialog's state, what a background import says while the dialog is
// closed, and the ways in from elsewhere (a file or link dropped on the window, a Spotify link in
// Open link or on the command line). The work itself is Rust's (`import.rs`); this only follows
// the `import-progress` events.
import { goto } from '$app/navigation';
import * as api from './api';
import { hrefFor } from './browse';
import { t, type TranslationKey } from './i18n.svelte';
import {
	auth,
	library,
	refreshLocalPlaylists,
	refreshView,
	startRadio,
	toast
} from './player.svelte';

export const imp = $state({
	open: false,
	/** The running or finished import. `null` when there is none, and then the dialog reads. */
	snapshot: null as api.ImportSnapshot | null,
	/** What the last read found, before an import starts from it. */
	preview: null as api.ImportPreview | null,
	/** The lists picked at the last start, so "Try again" starts the same import. */
	picked: [] as number[],
	reading: false
});

/** Loose on purpose: Rust does the real parsing, and answers `not_spotify` when it can't. */
export const isSpotifyLink = (s: string) =>
	/(^|\/\/|\s)(open|play)\.spotify\.com\//i.test(s) || /^\s*spotify:/i.test(s);

/** A unix second as a time of day, in the user's own format. */
export const clockTime = (secs: number) =>
	new Date(secs * 1000).toLocaleTimeString(undefined, { timeStyle: 'short' });

/** When a `cooldown:<unix seconds>` rejection lets imports reach YouTube again. */
export function cooldownUntil(message?: string | null): number | null {
	const m = /^cooldown:(\d+)$/.exec(message ?? '');
	return m ? Number(m[1]) : null;
}

/** A rejection from an import command, in the user's language when it is one of ours. */
export function importError(e: unknown): string {
	const raw = String(e ?? '');
	const until = cooldownUntil(raw);
	const key = `import.errors.${until ? 'cooldown' : raw}` as TranslationKey;
	const text = t(key, until ? { time: clockTime(until) } : undefined);
	return text === key ? raw : text;
}

// The newest job generation an `import-progress` event has spoken for. Starting an import or an
// update answers with the job's first snapshot, but the job runs as soon as it is spawned: with
// every song already matched it reaches review (or an update finishes) before that answer is
// back. Applied late, the answer put the dialog back at "0 of 7" for good.
let seen = 0;
const unlessNewer = (s: api.ImportSnapshot | null) => {
	if (s && s.gen > seen) imp.snapshot = s;
};

/** An import (not an update) is holding the dialog. */
const holding = () => !!imp.snapshot && !imp.snapshot.update;

async function read(how: () => Promise<api.ImportPreview>) {
	imp.reading = true;
	try {
		imp.preview = await how();
	} catch (e) {
		toast.error(importError(e));
	} finally {
		imp.reading = false;
	}
}

export const readLink = (link: string) => read(() => api.importReadLink(link.trim()));
export const readFile = (file: File | string) =>
	read(() => (typeof file === 'string' ? api.importReadPath(file) : api.importReadFile(file)));

/** Show the dialog, reading `link` straight away when nothing is holding it. */
export function openImport(link?: string) {
	imp.open = true;
	if (link && !holding()) {
		imp.preview = null;
		readLink(link);
	}
}

/**
 * The window's drop handler: the export zip, one of its JSON files, a CSV, or a Spotify link
 * (dragged out of the Spotify app, or a browser's address bar). Answers whether it took the drop.
 */
export function handleImportDrop(e: DragEvent): boolean {
	const dt = e.dataTransfer;
	if (!dt) return false;
	const file = Array.from(dt.files).find((f) => /\.(zip|json|csv)$/i.test(f.name));
	const link = file
		? ''
		: ((dt.getData('text/uri-list') || dt.getData('text/plain'))
				.split('\n')
				.map((l) => l.trim())
				.find((l) => l && !l.startsWith('#')) ?? '');
	if (!file && !isSpotifyLink(link)) return false;
	e.preventDefault();
	imp.open = true;
	if (holding()) return true;
	imp.preview = null;
	if (file) readFile(file);
	else readLink(link);
	return true;
}

export async function startImport(lists: number[]) {
	imp.picked = lists;
	try {
		unlessNewer(await api.importStart(lists));
	} catch (e) {
		toast.error(importError(e));
	}
}

/** Put a finished import away, and with it what was read for it. */
export async function dismissImport() {
	await api.importCancel().catch(() => {});
	imp.snapshot = null;
	imp.preview = null;
}

export async function updateFromSpotify(playlistId: string) {
	try {
		unlessNewer(await api.importUpdate(playlistId));
	} catch (e) {
		toast.error(importError(e));
	}
}

/**
 * A Spotify link from Open link or the command line: a playlist goes to the import, a track plays
 * (with its radio, like any song you click), an album or artist opens its YouTube Music page.
 */
export async function openSpotifyLink(link: string) {
	toast(t('import.resolving'));
	try {
		const r = await api.importResolve(link);
		if (r.kind === 'playlist') openImport(link);
		else if (r.kind === 'song') startRadio('song', r.song.video_id);
		else goto(hrefFor({ kind: r.kind, id: r.id, title: '' }));
	} catch (e) {
		toast.error(importError(e));
	}
}

// The new playlists, into the library list the sidebar and Library page draw from. Local ones are
// read back from SQLite; account ones are added by hand, because YouTube's library browse lags a
// brand-new playlist by a few seconds (the same as `createLibraryPlaylist`).
function landed(s: api.ImportSnapshot) {
	if (s.results.some((r) => r.local)) refreshLocalPlaylists();
	const owner = auth.account?.name;
	const fresh: api.BrowseItem[] = s.results
		.filter((r) => !r.local && !library.items.some((i) => i.id === r.id))
		.map((r) => ({
			kind: 'playlist',
			id: r.id,
			title: r.name,
			subtitle: owner ? `${owner} • ${r.added} tracks` : undefined
		}));
	if (fresh.length) library.items = [...fresh, ...library.items];
}

// An update has nothing to review: it says what changed and puts itself away. One the background
// check started only speaks up when it changed something, and Rust puts it away itself.
function updated(s: api.ImportSnapshot) {
	if (s.phase !== 'done' && s.phase !== 'failed') return;
	const r = s.results[0];
	const changed = !!r && (r.added > 0 || r.removed > 0);
	if (s.phase === 'failed') {
		if (!s.auto) toast.error(importError(s.message));
	} else {
		if (changed || !s.auto)
			toast.success(
				!changed
					? t('import.update_none')
					: s.auto
						? t('import.auto_update_done', { name: r.name, added: r.added, removed: r.removed })
						: t('import.update_done', { added: r.added, removed: r.removed })
			);
		if (s.update && location.pathname.includes(encodeURIComponent(s.update))) refreshView();
		if (r?.local) refreshLocalPlaylists();
	}
	if (!s.auto) api.importCancel().catch(() => {});
	imp.snapshot = null;
}

function onSnapshot(s: api.ImportSnapshot) {
	seen = Math.max(seen, s.gen);
	const before = imp.snapshot;
	imp.snapshot = s;
	if (before?.phase === s.phase && before?.update === s.update) return;
	if (s.update) return updated(s);
	if (s.phase === 'done') landed(s);
	if (imp.open) return;
	if (s.phase === 'review') toast(t('import.toast_review'));
	else if (s.phase === 'done') toast.success(t('import.toast_done'));
	else if (s.phase === 'failed') toast.error(importError(s.message));
}

let started = false;

/** Once, from the root layout: follow the events, and pick up an import already under way (the
 *  webview reloaded while Rust kept going). */
export function initImport() {
	if (started) return;
	started = true;
	api.onImportProgress(onSnapshot);
	api
		.importStatus()
		.then(unlessNewer)
		.catch(() => {});
}
