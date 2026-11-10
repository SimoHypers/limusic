// The download engine: every entry point (a track row's menu, the player bar, a playlist's menu)
// enqueues jobs here, and the titlebar's Downloads popover watches them. Jobs run in the
// background, up to MAX_CONCURRENT at a time, so the setup dialog closes the moment you confirm and
// you can start more downloads while the first ones are still going. Progress comes from the
// backend's `download-progress` events, keyed by job id.
import { browser } from '$app/environment';
import * as api from './api';
import type { DownloadFormat, DownloadProgress, DownloadQuality, SongItem } from './api';
import { downloadableSongs, isDownloadable } from './download';
import { prefs, toast } from './player.svelte';
import { t } from './i18n.svelte';

export type DownloadTarget = { videoId: string; title: string; artists: string };
export type JobState = 'queued' | 'downloading' | 'done' | 'failed';

export interface DownloadJob {
	id: string;
	batch: number;
	videoId: string;
	title: string;
	artists: string;
	playlist?: string;
	dest?: string;
	format: DownloadFormat;
	quality: DownloadQuality;
	state: JobState;
	downloaded: number;
	total: number | null;
	percent: number;
	speed: number | null;
	path?: string;
	skipped?: boolean;
	error?: string;
}

/** Three yt-dlp processes at a time: enough to feel parallel without starving the network. */
const MAX_CONCURRENT = 3;

export const downloads = $state({
	/** The setup dialog (format, quality, remembering it). */
	open: false,
	target: null as DownloadTarget | null,
	/** Set for a "download playlist" run; the folder name and the whole track list. */
	playlist: null as { name: string; songs: SongItem[] } | null,
	jobs: [] as DownloadJob[],
	/** Checked once when the first setup dialog opens; a miss is shown as an install hint. */
	ytdlp: { checked: false, version: '', error: '' }
});

const queue: string[] = [];
const batches = new Map<number, { total: number; done: number; failed: number }>();
let active = 0;
let seq = 0;
let batchSeq = 0;

if (browser) {
	api.onDownloadProgress((p: DownloadProgress) => {
		patch(p.id, {
			downloaded: p.downloaded,
			total: p.total,
			percent: p.percent,
			speed: p.speed
		});
	}).catch(() => {});
}

function patch(id: string, changes: Partial<DownloadJob>) {
	const at = downloads.jobs.findIndex((job) => job.id === id);
	if (at >= 0) downloads.jobs[at] = { ...downloads.jobs[at], ...changes };
}

export function activeJobs(): DownloadJob[] {
	return downloads.jobs.filter((job) => job.state === 'queued' || job.state === 'downloading');
}

export function clearFinished() {
	downloads.jobs = downloads.jobs.filter(
		(job) => job.state === 'queued' || job.state === 'downloading'
	);
}

export function openDownload(target: DownloadTarget) {
	downloads.target = target;
	downloads.playlist = null;
	downloads.open = true;
}

export function openPlaylistDownload(name: string, songs: SongItem[]) {
	downloads.target = null;
	downloads.playlist = { name, songs: downloadableSongs(songs) };
	downloads.open = true;
}

export function closeDownloads() {
	downloads.open = false;
	downloads.target = null;
	downloads.playlist = null;
}

/** Runs `yt-dlp --version` once per app session, on the first dialog open. */
export async function checkYtdlp() {
	if (downloads.ytdlp.checked) return;
	downloads.ytdlp.checked = true;
	try {
		downloads.ytdlp.version = await api.checkYtdlp();
		downloads.ytdlp.error = '';
	} catch (e) {
		downloads.ytdlp.error = String(e);
	}
}

function itemsToDownload(): DownloadTarget[] {
	if (downloads.playlist) {
		return downloads.playlist.songs.map((s) => ({
			videoId: s.video_id,
			title: s.title,
			artists: s.artists ?? ''
		}));
	}
	const target = downloads.target;
	return target && isDownloadable(target.videoId) ? [target] : [];
}

function enqueue(
	items: DownloadTarget[],
	opts: { format: DownloadFormat; quality: DownloadQuality },
	playlist?: string,
	dest?: string
) {
	const batch = ++batchSeq;
	batches.set(batch, { total: items.length, done: 0, failed: 0 });
	const jobs: DownloadJob[] = items.map((item) => ({
		id: `dl-${++seq}`,
		batch,
		videoId: item.videoId,
		title: item.title,
		artists: item.artists,
		playlist,
		dest,
		format: opts.format,
		quality: opts.quality,
		state: 'queued',
		downloaded: 0,
		total: null,
		percent: 0,
		speed: null
	}));
	downloads.jobs = [...downloads.jobs, ...jobs];
	queue.push(...jobs.map((job) => job.id));
	pump();
}

function pump() {
	while (active < MAX_CONCURRENT && queue.length) {
		const id = queue.shift();
		if (!id) return;
		active += 1;
		void runJob(id).finally(() => {
			active -= 1;
			pump();
		});
	}
}

async function runJob(id: string) {
	const job = downloads.jobs.find((candidate) => candidate.id === id);
	if (!job) return;
	patch(id, { state: 'downloading' });
	try {
		const outcome = await api.downloadTrack(
			job.videoId,
			job.artists,
			job.title,
			job.format,
			job.quality,
			{ id: job.id, playlist: job.playlist, dest: job.dest }
		);
		const current = downloads.jobs.find((candidate) => candidate.id === id);
		const total = current?.total ?? null;
		patch(id, {
			state: 'done',
			percent: 100,
			total,
			downloaded: total ?? current?.downloaded ?? 0,
			speed: null,
			path: outcome.path,
			skipped: outcome.skipped
		});
		finishJob(job.batch, null, outcome);
	} catch (e) {
		const error = String(e);
		patch(id, { state: 'failed', speed: null, error });
		finishJob(job.batch, error, null);
	}
}

/** One toast per batch: the file's path for a single track, a count for a playlist. */
function finishJob(batch: number, error: string | null, outcome: api.DownloadOutcome | null) {
	const info = batches.get(batch);
	if (!info) return;
	if (error) info.failed += 1;
	else info.done += 1;
	if (info.done + info.failed < info.total) return;
	batches.delete(batch);

	if (info.failed) {
		toast.error(
			info.total === 1
				? (error ?? t('player.download_failed'))
				: t('player.download_summary_partial', { failed: info.failed, total: info.total })
		);
	} else if (info.total === 1 && outcome) {
		toast.success(
			t(outcome.skipped ? 'player.download_skipped' : 'player.download_saved', {
				path: outcome.path
			})
		);
	} else {
		toast.success(t('player.download_summary_ok', { count: info.total }));
	}
}

function rememberChoice(opts: { format: DownloadFormat; quality: DownloadQuality }) {
	prefs.downloadFormat = opts.format;
	prefs.downloadQuality = opts.quality;
	prefs.downloadRemember = true;
	Promise.all([
		api.setSetting('download_format', opts.format),
		api.setSetting('download_quality', opts.quality),
		api.setSetting('download_remember', 'true')
	]).catch((e) => toast.error(String(e)));
}

/**
 * Starts the chosen downloads in the background and closes the setup dialog. A remembered choice
 * needs no dialog at all; `quickDownload` is that path for a track menu.
 */
export function startDownload(opts: {
	format: DownloadFormat;
	quality: DownloadQuality;
	remember: boolean;
	dest?: string;
}): void {
	const items = itemsToDownload();
	if (!items.length) {
		toast.error(t('player.download_nothing'));
		return;
	}
	if (opts.remember) rememberChoice(opts);
	const playlist = downloads.playlist?.name;
	enqueue(items, opts, playlist, playlist ? undefined : opts.dest);
	closeDownloads();
}

/** A track menu click with a remembered format: enqueue now, no dialog. */
export function quickDownload(target: DownloadTarget) {
	downloads.target = target;
	downloads.playlist = null;
	startDownload({
		format: prefs.downloadFormat,
		quality: prefs.downloadQuality,
		remember: true
	});
}
