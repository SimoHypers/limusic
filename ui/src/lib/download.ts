// Pure download decisions, kept out of the `.svelte.ts` store so `node --experimental-strip-types`
// can check them (`download.check.ts`). `api.ts` is type-only here: a runtime import would pull
// Tauri into the check runner.
import type { DownloadFormat, DownloadQuality, SongItem } from './api';

export const DOWNLOAD_FORMATS: DownloadFormat[] = ['mp3', 'm4a', 'flac', 'opus', 'wav'];
export const DOWNLOAD_QUALITIES: DownloadQuality[] = ['best', '320', '192', '128', '64'];

export const DOWNLOAD_DEFAULTS = {
	format: 'mp3' as DownloadFormat,
	quality: 'best' as DownloadQuality,
	remember: false
};

/** A byte count for the download popover: `1.5 KB`, `11.7 MB`. `—` when nothing is known yet. */
export function formatBytes(bytes: number | null | undefined): string {
	if (!bytes || bytes <= 0) return '—';
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	const at = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
	return `${(bytes / 1024 ** at).toFixed(at === 0 ? 0 : 1)} ${units[at]}`;
}

/** yt-dlp reports bytes per second; empty when it has not measured the speed yet. */
export function formatSpeed(bytesPerSecond: number | null | undefined): string {
	return bytesPerSecond && bytesPerSecond > 0 ? `${formatBytes(bytesPerSecond)}/s` : '';
}

/** Mirrors `api.isLocalId` without importing it: a local file has no YouTube id to download. */
const LOCAL_PREFIXES = ['LOCAL:', 'LOCALALBUM:', 'LOCALARTIST:', 'LOCALPLAYLIST:'];

/** A YouTube id can be fetched; a local file/folder id cannot. */
export function isDownloadable(id: string | undefined | null): boolean {
	return !!id && !LOCAL_PREFIXES.some((prefix) => id.startsWith(prefix));
}

/** The remembered choice, with anything unknown or missing falling back to the defaults. */
export function parseDownloadPrefs(settings: Record<string, string>): {
	format: DownloadFormat;
	quality: DownloadQuality;
	remember: boolean;
} {
	const format = settings.download_format as DownloadFormat;
	const quality = settings.download_quality as DownloadQuality;
	return {
		format: DOWNLOAD_FORMATS.includes(format) ? format : DOWNLOAD_DEFAULTS.format,
		quality: DOWNLOAD_QUALITIES.includes(quality) ? quality : DOWNLOAD_DEFAULTS.quality,
		remember: settings.download_remember === 'true'
	};
}

/**
 * What a playlist download actually requests: local rows are dropped (no YouTube id) and a video
 * that appears twice is fetched once, in the playlist's order.
 */
export function downloadableSongs(items: SongItem[]): SongItem[] {
	const seen = new Set<string>();
	const out: SongItem[] = [];
	for (const song of items) {
		if (!isDownloadable(song.video_id) || seen.has(song.video_id)) continue;
		seen.add(song.video_id);
		out.push(song);
	}
	return out;
}
