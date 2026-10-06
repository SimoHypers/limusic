// Offline downloads: the shapes the backend reports and the pure
// helpers every surface shares. Kept out of the `.svelte.ts` store so the logic is testable under
// node (`downloads.check.ts`), the same split as `queue.ts` / `queue.svelte.ts`.
import type { SongItem } from './api';

/** Where one queued download is in its life. Mirrors the backend's `DownloadItem.state`. */
export type DownloadState = 'queued' | 'downloading' | 'paused' | 'done' | 'error' | 'cancelled';

/**
 * One download the backend knows about. Field names are snake_case on purpose: the payload is
 * handed straight to `downloads_list` / the events, and re-spelling it here would be one more
 * place a rename can silently drop a field.
 */
export interface DownloadItem {
	id: string;
	video_id: string;
	title: string;
	artists: string;
	album: string | null;
	thumbnail: string | null;
	state: DownloadState;
	itag: number | null;
	mime: string | null;
	/** What the backend actually got, for the row's quality chip ("AAC 256kbps"). */
	quality_label: string;
	/** Absolute path of the saved file; null while there is none yet. */
	path: string | null;
	bytes_done: number;
	bytes_total: number | null;
	error: string | null;
	added_at: number;
	updated_at: number;
}

export type DownloadQuality = 'HIGH' | 'LOW' | 'AUTO';
export const DEFAULT_DOWNLOAD_QUALITY: DownloadQuality = 'HIGH';
export const DEFAULT_DOWNLOAD_CONCURRENCY = 2;
export const MIN_DOWNLOAD_CONCURRENCY = 1;
export const MAX_DOWNLOAD_CONCURRENCY = 4;

export function parseDownloadQuality(v: string | null | undefined): DownloadQuality {
	const up = (v ?? '').toUpperCase();
	return up === 'LOW' || up === 'AUTO' ? up : DEFAULT_DOWNLOAD_QUALITY;
}

/** A stored concurrency from anywhere (a hand-edited setting, an old build) lands in 1..4. */
export function clampDownloadConcurrency(v: string | number | null | undefined): number {
	const n = typeof v === 'number' ? v : Number.parseInt(v ?? '', 10);
	if (!Number.isFinite(n)) return DEFAULT_DOWNLOAD_CONCURRENCY;
	return Math.min(MAX_DOWNLOAD_CONCURRENCY, Math.max(MIN_DOWNLOAD_CONCURRENCY, Math.round(n)));
}

/** The download record for a track, when it has one. */
export function downloadFor(
	items: readonly DownloadItem[],
	videoId: string
): DownloadItem | undefined {
	return items.find((d) => d.video_id === videoId);
}

/** Same test as `isLocalId` (api.ts), inlined: this module is loaded by `downloads.check.ts` under
 *  plain node, and api.ts pulls in SvelteKit's virtual modules. A song's id is either a YouTube id
 *  or one of the local prefixes — all of which mean the file is already on this machine. */
const LOCAL_RE = /^LOCAL(ALBUM|ARTIST|PLAYLIST)?:/;

/** Can this song be handed to `download_items` at all? Local files are already on disk. */
export function canDownload(song: SongItem): boolean {
	return !!song.video_id && !LOCAL_RE.test(song.video_id);
}

/** 0..1 while the total is known; null when the backend has not reported a size (yet). */
export function progressFraction(item: DownloadItem): number | null {
	if (item.bytes_total == null || item.bytes_total <= 0) return null;
	return Math.min(1, Math.max(0, item.bytes_done / item.bytes_total));
}

/** A byte count fit for a progress line ("12.3 MB"). */
export function formatBytes(n: number): string {
	if (!Number.isFinite(n) || n <= 0) return '0 B';
	const units = ['B', 'KB', 'MB', 'GB'];
	let v = n;
	let i = 0;
	while (v >= 1024 && i < units.length - 1) {
		v /= 1024;
		i++;
	}
	return `${i === 0 ? String(Math.round(v)) : String(Number(v.toFixed(1)))} ${units[i]}`;
}

/**
 * The playlist picker's count box: "All" is a mode of its own, and in the number mode only a
 * positive whole number goes through. `max` (a playlist total that is actually known) turns a
 * number larger than the list into an error rather than a silent no-op in the backend.
 */
export function parseCount(
	raw: string,
	max?: number
): { count: number | null; error: 'empty' | 'invalid' | 'too_big' | null } {
	const text = raw.trim();
	if (!text) return { count: null, error: 'empty' };
	if (!/^\d+$/.test(text)) return { count: null, error: 'invalid' };
	const n = Number(text);
	if (n < 1) return { count: null, error: 'invalid' };
	if (max !== undefined && n > max) return { count: null, error: 'too_big' };
	return { count: n, error: null };
}

/** The first number in a subtitle like "Owner • 190 songs" / "1,024 songs", when there is one. */
export function playlistTotal(subtitle?: string | null): number | undefined {
	const m = subtitle?.match(/(\d[\d,]*)/);
	if (!m) return undefined;
	const n = Number(m[1].replace(/,/g, ''));
	return Number.isFinite(n) && n > 0 ? n : undefined;
}

/** Per-state counts for the header line and the section headings. */
export interface DownloadSummary {
	total: number;
	queued: number;
	downloading: number;
	paused: number;
	done: number;
	error: number;
	cancelled: number;
	/** Everything that is not finished: downloading + queued + paused (+ failed/cancelled). */
	unfinished: number;
}

export function summarize(items: readonly DownloadItem[]): DownloadSummary {
	const s: DownloadSummary = {
		total: items.length,
		queued: 0,
		downloading: 0,
		paused: 0,
		done: 0,
		error: 0,
		cancelled: 0,
		unfinished: 0
	};
	for (const d of items) {
		s[d.state]++;
		if (d.state !== 'done') s.unfinished++;
	}
	return s;
}

const STATE_ORDER: Record<DownloadState, number> = {
	downloading: 0,
	queued: 1,
	paused: 2,
	error: 3,
	done: 4,
	cancelled: 5
};

/**
 * The page's order: what is moving first, then what is waiting, what needs attention, and the
 * finished library last — newest activity first inside each run.
 */
export function orderedDownloads(items: readonly DownloadItem[]): DownloadItem[] {
	return [...items].sort(
		(a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state] || b.updated_at - a.updated_at
	);
}

/**
 * Bytes the downloads are holding on disk right now: finished files at their full size, partials
 * at however far they got (cancelled rows report 0, so they cannot inflate the total). This is
 * what the Downloads header's storage chip shows.
 */
export function storedBytes(items: readonly DownloadItem[]): number {
	return items.reduce((total, d) => total + (d.bytes_done > 0 ? d.bytes_done : 0), 0);
}

/**
 * The rows a bulk "Retry" may take: the two ends that stopped without finishing. Queued and
 * paused rows have their own controls, and a finished one has nothing to retry.
 */
export function retryableDownloads(items: readonly DownloadItem[]): DownloadItem[] {
	return items.filter((d) => d.state === 'error' || d.state === 'cancelled');
}

/** The two page sections, in that same order. */
export function splitDownloads(items: readonly DownloadItem[]): {
	queue: DownloadItem[];
	done: DownloadItem[];
} {
	const ordered = orderedDownloads(items);
	return {
		queue: ordered.filter((d) => d.state !== 'done'),
		done: ordered.filter((d) => d.state === 'done')
	};
}

/** A done download, as the SongItem worth playing offline: the original metadata, no stream. */
export function downloadItemToSong(item: DownloadItem): SongItem {
	return {
		video_id: item.video_id,
		title: item.title,
		artists: item.artists,
		album: item.album ?? undefined,
		thumbnail: item.thumbnail ?? undefined
	};
}

/** The t() key for a state chip. A map, not a template, so a new state is a type error here. */
export const DOWNLOAD_STATE_KEYS = {
	queued: 'downloads.states.queued',
	downloading: 'downloads.states.downloading',
	paused: 'downloads.states.paused',
	done: 'downloads.states.done',
	error: 'downloads.states.error',
	cancelled: 'downloads.states.cancelled'
} as const satisfies Record<DownloadState, string>;

/** The t() key for the badge tooltip (with the row's quality where that is the useful part). */
export const DOWNLOAD_BADGE_KEYS = {
	queued: 'downloads.badge_queued',
	downloading: 'downloads.badge_downloading',
	paused: 'downloads.badge_paused',
	done: 'downloads.badge_done',
	error: 'downloads.badge_error',
	cancelled: 'downloads.badge_cancelled'
} as const satisfies Record<DownloadState, string>;