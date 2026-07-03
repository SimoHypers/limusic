// The UI's only door to Rust. context/11 UI contract — commands in, events out. The UI never
// touches YouTube; everything here is a Tauri command or event payload.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface SongItem {
	video_id: string;
	title: string;
	artists: string;
	album?: string;
	duration?: string;
	thumbnail?: string;
}

export interface NowPlaying {
	videoId: string;
	title: string;
	artists: string;
	thumbnail?: string;
	duration?: string;
	streamClient: string;
}

export interface QueueState {
	items: SongItem[];
	currentIndex: number;
}

// --- commands (context/11) -----------------------------------------------------------------
export const search = (query: string) => invoke<SongItem[]>('search', { query });
export const play = (item: SongItem) => invoke<void>('play', { item });
export const playIndex = (index: number) => invoke<void>('play_index', { index });
export const nextTrack = () => invoke<void>('next_track');
export const prevTrack = () => invoke<void>('prev_track');
export const togglePause = () => invoke<void>('toggle_pause');
export const seek = (position: number) => invoke<void>('seek', { position });
export const setVolume = (volume: number) => invoke<void>('set_volume', { volume });
export const getQueue = () => invoke<QueueState>('get_queue');

// --- events (context/11). Each returns an unlisten fn; call it on component teardown. --------
export const onNowPlaying = (cb: (n: NowPlaying) => void): Promise<UnlistenFn> =>
	listen<NowPlaying>('now-playing', (e) => cb(e.payload));
export const onQueueChanged = (cb: (q: QueueState) => void): Promise<UnlistenFn> =>
	listen<QueueState>('queue-changed', (e) => cb(e.payload));
export const onPosition = (cb: (p: number) => void): Promise<UnlistenFn> =>
	listen<{ position: number }>('position', (e) => cb(e.payload.position));
export const onDuration = (cb: (d: number) => void): Promise<UnlistenFn> =>
	listen<{ duration: number }>('duration', (e) => cb(e.payload.duration));
export const onPlaybackState = (cb: (s: 'playing' | 'paused') => void): Promise<UnlistenFn> =>
	listen<'playing' | 'paused'>('playback-state', (e) => cb(e.payload));
export const onPlaybackError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<{ message: string }>('playback-error', (e) => cb(e.payload.message));
