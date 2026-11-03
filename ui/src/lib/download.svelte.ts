// Download manager reactive state and actions
import * as api from './api';
import type { DownloadTask, SongItem } from './api';
import { toast } from './player.svelte';
import { t } from './i18n.svelte';

export type DownloadState = 'idle' | 'downloading' | 'downloaded';

export const downloadQueue = $state({
	tasks: [] as DownloadTask[],
	isOpen: false,
	get activeCount() {
		return this.tasks.filter(
			(task) =>
				task.status === 'downloading' || task.status === 'queued' || task.status === 'tagging'
		).length;
	}
});

let downloadedSet = $state(new Set<string>());

export function isDownloaded(videoId: string | undefined | null): boolean {
	if (!videoId) return false;
	return downloadedSet.has(videoId);
}

export function isDownloading(videoId: string | undefined | null): boolean {
	if (!videoId) return false;
	return downloadQueue.tasks.some(
		(task) =>
			task.video_id === videoId &&
			(task.status === 'downloading' || task.status === 'queued' || task.status === 'tagging')
	);
}

export function getSongDownloadState(videoId: string | undefined | null): DownloadState {
	if (!videoId) return 'idle';
	if (isDownloading(videoId)) return 'downloading';
	if (isDownloaded(videoId)) return 'downloaded';
	return 'idle';
}

export function getCollectionDownloadState(videoIds: (string | undefined | null)[]): DownloadState {
	if (!videoIds || videoIds.length === 0) return 'idle';
	const validIds = videoIds.filter(Boolean) as string[];
	if (validIds.length === 0) return 'idle';
	if (validIds.some((id) => isDownloading(id))) return 'downloading';
	if (validIds.every((id) => isDownloaded(id))) return 'downloaded';
	return 'idle';
}

let initialized = false;

export async function initDownloads() {
	if (initialized) return;
	initialized = true;

	try {
		const [list, ids] = await Promise.all([
			api.getDownloads(),
			api.getDownloadedIds()
		]);
		downloadQueue.tasks = list.filter((t) => t.status !== 'cancelled');
		const nextSet = new Set<string>(ids);
		for (const t of list) {
			if (t.status === 'completed') {
				nextSet.add(t.video_id);
			}
		}
		downloadedSet = nextSet;
	} catch (e) {
		console.error('Failed to get initial downloads', e);
	}

	await api.onDownloadProgress((task) => {
		if (task.status === 'cancelled') {
			downloadQueue.tasks = downloadQueue.tasks.filter((t) => t.id !== task.id);
			return;
		}

		const idx = downloadQueue.tasks.findIndex((t) => t.id === task.id);
		if (idx >= 0) {
			const prev = downloadQueue.tasks[idx];
			downloadQueue.tasks[idx] = task;
			if (prev.status !== 'completed' && task.status === 'completed') {
				downloadedSet.add(task.video_id);
				downloadedSet = new Set(downloadedSet);
				toast(`${t('download.downloaded')}: ${task.title}`);
			} else if (prev.status !== 'failed' && task.status === 'failed') {
				toast(`${t('download.status_failed')}: ${task.title}`);
			}
		} else {
			downloadQueue.tasks.push(task);
			if (task.status === 'completed') {
				downloadedSet.add(task.video_id);
				downloadedSet = new Set(downloadedSet);
			}
		}
	});
}

export async function downloadSong(song: SongItem, subfolder?: string) {
	try {
		await api.downloadTrack(song, subfolder);
		toast(`${t('download.downloading')}: ${song.title}`);
	} catch (e) {
		toast(`${t('download.status_failed')}: ${e}`);
	}
}

export async function downloadPlaylistById(playlistId: string, title?: string) {
	try {
		toast(t('download.downloading'));
		const count = await api.downloadPlaylist(playlistId, title);
		toast(`${count} ${t('common.songs')} ${t('download.queued')}`);
	} catch (e) {
		toast(`${t('download.status_failed')}: ${e}`);
	}
}

export async function downloadAlbumById(albumId: string, title?: string) {
	try {
		toast(t('download.downloading'));
		const count = await api.downloadAlbum(albumId, title);
		toast(`${count} ${t('common.songs')} ${t('download.queued')}`);
	} catch (e) {
		toast(`${t('download.status_failed')}: ${e}`);
	}
}

export async function cancelTask(id: string) {
	try {
		downloadQueue.tasks = downloadQueue.tasks.filter((t) => t.id !== id);
		await api.cancelDownload(id);
	} catch (e) {
		console.error(e);
	}
}

export async function cancelAll() {
	try {
		downloadQueue.tasks = downloadQueue.tasks.filter(
			(t) =>
				t.status !== 'downloading' && t.status !== 'queued' && t.status !== 'tagging'
		);
		await api.cancelAllDownloads();
	} catch (e) {
		console.error(e);
	}
}

export async function clearCompleted() {
	try {
		await api.clearCompletedDownloads();
		downloadQueue.tasks = downloadQueue.tasks.filter(
			(task) =>
				task.status !== 'completed' && task.status !== 'failed' && task.status !== 'cancelled'
		);
	} catch (e) {
		console.error(e);
	}
}

export async function deleteAllDownloads() {
	try {
		await api.deleteAllDownloads();
		downloadQueue.tasks = [];
		downloadedSet = new Set();
	} catch (e) {
		console.error(e);
	}
}
