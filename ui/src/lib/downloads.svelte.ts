// The download queue, live. One module-level store, hydrated once from `downloads_list()` and kept
// in step by the two backend events; every surface (row badges, menus, the Downloads page) reads
// the same list, so a pause clicked in one place shows up in all of them.
//
// Set up from the root layout (`initDownloads`), same deal as `initApp`: the subscription is
// global, and the layout's teardown owns it.
import * as api from './api';
import type { DownloadItem, SongItem } from './api';
import {
	clampDownloadConcurrency,
	DEFAULT_DOWNLOAD_CONCURRENCY,
	DEFAULT_DOWNLOAD_QUALITY,
	downloadFor,
	parseDownloadQuality,
	type DownloadQuality
} from './downloads';
import { t } from './i18n.svelte';
import { toast } from './player.svelte';

export const downloads = $state({
	items: [] as DownloadItem[],
	/** True once `downloads_list()` has answered (an empty answer counts). */
	loaded: false,
	/** The last hydration failure, for the page's empty state. Cleared by a successful read. */
	error: null as string | null,
	/** The user's defaults, mirrored here so menus and the picker agree with Settings instantly. */
	quality: DEFAULT_DOWNLOAD_QUALITY as DownloadQuality,
	concurrency: DEFAULT_DOWNLOAD_CONCURRENCY as number
});

/** Per-id in-flight guard: a control can't fire twice while its write is on the way. */
export const dlBusy = $state<Record<string, true>>({});

let started = false;
let subs: Promise<() => void>[] = [];

/** Idempotent; answers the teardown the layout calls on destroy. */
export function initDownloads(): () => void {
	if (started) return () => {};
	started = true;
	subs = [
		api.onDownloadsChanged((items) => {
			downloads.items = items;
			downloads.loaded = true;
			downloads.error = null;
		}),
		api.onDownloadProgress((p) => {
			const i = downloads.items.findIndex((d) => d.id === p.id);
			if (i < 0) return; // a row we never saw (a raced list refresh will bring it)
			downloads.items[i] = {
				...downloads.items[i],
				bytes_done: p.bytes_done,
				bytes_total: p.bytes_total
			};
		})
	];
	refreshDownloads();
	// The settings are read here as well as in the settings modal: the menu picker's default and
	// every batch call go through `downloads.quality`, and they must agree with what was saved.
	api
		.getSettings()
		.then((s) => {
			downloads.quality = parseDownloadQuality(s.download_quality);
			downloads.concurrency = clampDownloadConcurrency(s.download_concurrency);
		})
		.catch(() => {});
	return () => {
		subs.forEach((u) => u.then((f) => f()).catch(() => {}));
		subs = [];
		started = false;
	};
}

/** Re-read the whole list (the page's Try again, and the initial hydration). */
export async function refreshDownloads(): Promise<void> {
	try {
		downloads.items = await api.downloadsList();
		downloads.loaded = true;
		downloads.error = null;
	} catch (e) {
		downloads.error = String(e);
		if (!downloads.loaded) downloads.loaded = true; // an error state, not a spinner forever
	}
}

/** Insert or replace rows from an answer to `download_items` (the events land the same rows). */
function mergeItems(items: DownloadItem[]) {
	for (const item of items) {
		const i = downloads.items.findIndex((d) => d.id === item.id);
		if (i < 0) downloads.items.push(item);
		else downloads.items[i] = item;
	}
}

/**
 * Queue songs. One entry point for the single-song menu and the selection bar: a single track is
 * a one-element batch, exactly what the backend contract says. Answers whether the call was made,
 * so a caller can keep its own spinner honest; failures are toasted here.
 */
export async function downloadSongs(
	songs: SongItem[],
	quality: DownloadQuality = downloads.quality
): Promise<boolean> {
	if (!songs.length) return false;
	try {
		const items = await api.downloadItems(songs, quality);
		mergeItems(items);
		// The backend answers tracks it already holds as-is and never re-queues a done one, so the
		// message counts what actually went in rather than the rows that came back.
		const active = items.filter(
			(d) => d.state === 'queued' || d.state === 'downloading' || d.state === 'paused'
		).length;
		if (active === 0) toast(t('toasts.download_already'));
		else
			toast.success(
				items.length === 1
					? t('toasts.download_queued_one')
					: t('toasts.download_queued', { count: active })
			);
		return true;
	} catch (e) {
		toast.error(String(e));
		return false;
	}
}

/** One song's menu entry: same path, wrapped so the call sites read clearly. */
export function downloadSong(song: SongItem, quality?: DownloadQuality): Promise<boolean> {
	return downloadSongs([song], quality);
}

/** Queue a playlist — `count` null is the whole list. Answers how many the backend took. */
export async function downloadPlaylist(
	playlistId: string,
	count: number | null,
	quality: DownloadQuality = downloads.quality
): Promise<number | null> {
	try {
		const queued = await api.downloadPlaylist(playlistId, count, quality);
		// 0 means every track in range was already downloaded (done rows are not re-queued).
		if (queued === 0) toast(t('toasts.download_nothing_new'));
		else
			toast.success(
				count === null
					? t('toasts.download_playlist_all', { count: queued })
					: t('toasts.download_playlist_count', { count: queued })
			);
		return queued;
	} catch (e) {
		toast.error(String(e));
		return null;
	}
}

export type DownloadAction = 'pause' | 'resume' | 'cancel' | 'retry' | 'remove';

/**
 * One of the queue controls. Every control goes through here, so the in-flight guard, the error
 * toast and the optimistic row removal behave the same in the menu, the row and anywhere else.
 */
export async function actOnDownload(id: string, action: DownloadAction): Promise<boolean> {
	if (dlBusy[id]) return false;
	dlBusy[id] = true;
	try {
		await api.downloadsAction(id, action);
		// `remove` is a deletion: the row goes now rather than on the next event, so the list
		// never shows a file that is already gone.
		if (action === 'remove') {
			downloads.items = downloads.items.filter((d) => d.id !== id);
			toast.success(t('toasts.download_removed'));
		}
		return true;
	} catch (e) {
		toast.error(String(e));
		return false;
	} finally {
		delete dlBusy[id];
	}
}

/** The store's own read of one track, for badge components. */
export function downloadOf(videoId: string): DownloadItem | undefined {
	return downloadFor(downloads.items, videoId);
}

/**
 * Bulk retry: the rows that stopped without finishing, one independent call each. The point of
 * doing it here rather than looping `actOnDownload` at the call site is the feedback — one toast
 * for what went back in the queue and one for what did not, instead of one per row, and every id
 * stays flagged busy until the last call has answered so no row can be double-fired.
 */
export async function retryDownloads(ids: string[]): Promise<boolean> {
	const targets = ids.filter((id) => !dlBusy[id]);
	if (!targets.length) return false;
	for (const id of targets) dlBusy[id] = true;
	try {
		const results = await Promise.allSettled(
			targets.map((id) => api.downloadsAction(id, 'retry'))
		);
		const queued = results.filter((r) => r.status === 'fulfilled').length;
		const failure = results.find((r) => r.status === 'rejected');
		if (queued > 0) toast.success(t('toasts.download_queued', { count: queued }));
		if (failure?.status === 'rejected') toast.error(String(failure.reason));
		return queued > 0;
	} finally {
		for (const id of targets) delete dlBusy[id];
	}
}

/**
 * Bulk "clear finished": the caller has confirmed (it deletes real files), this does the work —
 * one call per finished row, the rows dropped from the list as soon as the backend has answered
 * them (the `downloads-changed` snapshot would land the same removal anyway), and a single toast
 * with the count. A partial failure keeps its rows and is reported, never silently swallowed.
 */
export async function removeDownloads(ids: string[]): Promise<boolean> {
	const targets = ids.filter((id) => !dlBusy[id]);
	if (!targets.length) return false;
	for (const id of targets) dlBusy[id] = true;
	try {
		const results = await Promise.allSettled(
			targets.map((id) => api.downloadsAction(id, 'remove'))
		);
		const gone = new Set(
			targets.filter((_, i) => results[i].status === 'fulfilled')
		);
		const failure = results.find((r) => r.status === 'rejected');
		if (gone.size > 0) {
			downloads.items = downloads.items.filter((d) => !gone.has(d.id));
			toast.success(t('toasts.downloads_cleared', { count: gone.size }));
		}
		if (failure?.status === 'rejected') toast.error(String(failure.reason));
		return gone.size > 0;
	} finally {
		for (const id of targets) delete dlBusy[id];
	}
}