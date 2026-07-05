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
	/** Item id within a playlist — present only on playlist tracks; needed to remove them. */
	set_video_id?: string;
}

export interface NowPlaying {
	videoId: string;
	title: string;
	artists: string;
	thumbnail?: string;
	duration?: string;
	streamClient: string;
	/** Whether the track is in the user's Liked Music (null if unknown). */
	liked?: boolean | null;
}

export interface QueueState {
	items: SongItem[];
	currentIndex: number;
}

export interface Account {
	signedIn: boolean;
	name?: string | null;
	handle?: string | null;
	thumbnail?: string | null;
}

export interface BrowseItem {
	kind: 'song' | 'playlist' | 'album' | 'artist';
	/** videoId (song) or browseId (playlist/album/artist). */
	id: string;
	title: string;
	subtitle?: string;
	thumbnail?: string;
}

export interface HomeSection {
	title: string;
	items: BrowseItem[];
}
export interface HomePage {
	sections: HomeSection[];
}

export interface PlaylistPage {
	title?: string;
	subtitle?: string;
	thumbnail?: string;
	items: SongItem[];
	continuation?: string;
}
export interface PlaylistContinuation {
	items: SongItem[];
	continuation?: string;
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

// --- auth (context/15) ---------------------------------------------------------------------
export const setCookie = (cookie: string) => invoke<Account>('set_cookie', { cookie });
export const getAccount = () => invoke<Account>('get_account');
export const signOut = () => invoke<void>('sign_out');
/** Open the in-app Google sign-in webview (context/15 Path A). Result arrives via onAuthChanged. */
export const loginWebview = () => invoke<void>('login_webview');

// --- browse / library (context/08) ---------------------------------------------------------
export const getHome = () => invoke<HomePage>('get_home');
export const getLibrary = () => invoke<BrowseItem[]>('get_library');
export const getPlaylist = (id: string) => invoke<PlaylistPage>('get_playlist', { id });
export const getPlaylistMore = (token: string) =>
	invoke<PlaylistContinuation>('get_playlist_more', { token });
export const playPlaylist = (items: SongItem[], start: number) =>
	invoke<void>('play_playlist', { items, start });

// --- write actions (context/01 ✎) ----------------------------------------------------------
export const like = (videoId: string, liked: boolean) => invoke<void>('like', { videoId, liked });
export const addToPlaylist = (playlistId: string, videoId: string) =>
	invoke<void>('add_to_playlist', { playlistId, videoId });
export const removeFromPlaylist = (playlistId: string, videoId: string, setVideoId: string) =>
	invoke<void>('remove_from_playlist', { playlistId, videoId, setVideoId });
export const createPlaylist = (title: string) => invoke<string>('create_playlist', { title });
export const deletePlaylist = (playlistId: string) =>
	invoke<void>('delete_playlist', { playlistId });
export const subscribe = (channelId: string, subscribed: boolean) =>
	invoke<void>('subscribe', { channelId, subscribed });

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
export const onAuthChanged = (cb: (a: Account) => void): Promise<UnlistenFn> =>
	listen<Account>('auth-changed', (e) => cb(e.payload));
export const onLoginError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<string>('login-error', (e) => cb(e.payload));
export const onLoginDone = (cb: () => void): Promise<UnlistenFn> =>
	listen('login-done', () => cb());
