// The UI's only door to Rust. context/11 UI contract — commands in, events out. The UI never
// touches YouTube; everything here is a Tauri command or event payload.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** How the signed-in user rated a track (innertube `Rating`). The three states are mutually
 *  exclusive: liking a disliked track clears the dislike, and vice versa. */
export type Rating = 'like' | 'dislike' | 'indifferent';

/** One run of an artist line: its text, plus a channel id when that run links an artist. */
export interface ArtistRun {
	text: string;
	id?: string;
}

export interface SongItem {
	video_id: string;
	title: string;
	artists: string;
	/** Primary artist's channel browseId (`UC…`), when linked — makes the artist name navigable. */
	artist_id?: string;
	/** The artist line run by run — a collab links each name to its own page. Empty/absent when
	 * nothing is linked; render plain `artists` then. */
	artist_runs?: ArtistRun[];
	album?: string;
	/** The album's browseId (`MPRE…`), when linked — makes the album navigable. */
	album_id?: string;
	duration?: string;
	/** Play count as YouTube abbreviates it ("53M"). Album, artist and search rows. */
	play_count?: string;
	thumbnail?: string;
	/** Item id within a playlist — present only on playlist tracks; needed to remove them. */
	set_video_id?: string;
	/** Collaborative playlists only: who added this track, and their avatar. */
	added_by?: string;
	added_by_avatar?: string;
	/** The signed-in user's rating (absent when the response didn't say — same as 'indifferent'). */
	rating?: Rating;
	/** "Add to library" off the row's own menu, with a token for each direction. Absent on rows
	 *  YouTube sent no menu for, and on the ones built here (local files, On Repeat, a Listen
	 *  Together guest's queue) — the menu hides the action rather than offering a dead one.
	 *  Library ▸ Songs is not Liked Music: this is a feedback write, not a rating. */
	library?: { in_library: boolean; add_token?: string; remove_token?: string };
	/** Listen Together: name of the guest who added this queue item (session adds only). */
	queued_by?: string;
	/** Queued to play next ("Play next", or a guest's session add) — the "Next in queue" block. */
	queued?: boolean;
	/** Appended by "Add to queue" — its own block at the tail of the queue. */
	queued_end?: boolean;
	/** The album/playlist either block was added from, for its heading in the queue panel. */
	queued_from?: string;
	/** Appended by autoplay radio continuation — drives the queue's "Autoplay" divider + badge. */
	autoplay?: boolean;
	/** YouTube flags the track explicit. Browse/search rows only: `/next` carries no badge, so a
	 *  radio- or autoplay-appended track arrives without it. */
	explicit?: boolean;
	/** This row links a music video rather than the audio track. */
	is_video?: boolean;
	/** One of the user's own YouTube Music uploads. Set by Rust and passed straight back on play:
	 *  only an authenticated client can stream one, and the row is where that is known. */
	is_upload?: boolean;
}

export interface NowPlaying {
	videoId: string;
	title: string;
	artists: string;
	artistId?: string;
	/** The artist line run by run — links each artist of a collab separately. */
	artistRuns?: ArtistRun[];
	thumbnail?: string;
	duration?: string;
	streamClient: string;
	/** The user's rating of the track (null if unknown). */
	rating?: Rating | null;
	/** YouTube's `musicVideoType` says this is a video upload, not the generated audio track.
	 *  Gates the player view's music-video mode. */
	isVideo?: boolean;
}

export type RepeatMode = 'off' | 'all' | 'one';

export interface QueueState {
	items: SongItem[];
	currentIndex: number;
	/** Start of the previously-played run: `items[playedFrom..currentIndex]` has actually been
	 *  heard. Not `0..currentIndex`: a playlist opened at track 7 has six untouched tracks first. */
	playedFrom?: number;
	shuffle?: boolean;
	repeat?: RepeatMode;
	/** What seeded the queue (playlist/album title, "<song> Radio") — the "Next from" header. */
	sourceName?: string | null;
}

export interface Account {
	signedIn: boolean;
	name?: string | null;
	handle?: string | null;
	email?: string | null;
	thumbnail?: string | null;
	channelId?: string | null;
	canSwitch?: boolean;
	/** The cookie authenticated, but a multi-channel login is not complete until one is chosen. */
	selectionRequired?: boolean;
}

export interface AccountIdentity {
	/** Opaque, process-local selector. Raw delegated/data-sync ids stay in Rust. */
	selectionKey: string;
	name: string;
	handle?: string | null;
	email?: string | null;
	thumbnail?: string | null;
	channelId?: string | null;
	selected: boolean;
}

export interface BrowseItem {
	kind: 'song' | 'playlist' | 'album' | 'artist';
	/** videoId (song) or browseId (playlist/album/artist). */
	id: string;
	title: string;
	subtitle?: string;
	thumbnail?: string;
	/** "3:47" — song items from a list-style shelf only (card shelves don't carry one). */
	duration?: string;
	/** Song cards only: the artist line run by run, so a card that gets played keeps its links. */
	artistRuns?: ArtistRun[];
	/** Play count as YouTube abbreviates it ("2.5B") — search song rows only. */
	playCount?: string;
	/** YouTube flags this track/album explicit. */
	explicit?: boolean;
	/** Song cards only: one of the user's own uploads. Carried into the SongItem `asSong` builds,
	 *  because that flag is what picks the login-only client chain when it plays. */
	isUpload?: boolean;
}

export interface HomeSection {
	title: string;
	items: BrowseItem[];
	moreBrowseId?: string;
	moreParams?: string;
}
/** A mood/genre filter chip above the home feed; `params` re-fetches home filtered to it. */
export interface HomeChip {
	title: string;
	params: string;
}
export interface HomePage {
	chips: HomeChip[];
	sections: HomeSection[];
	continuation?: string;
}

/**
 * The On Repeat auto-playlist's synthetic browseId (mirrors `ON_REPEAT_ID` in state.rs). It routes
 * like any other playlist; the only thing the UI does differently is draw an icon cover, because
 * a playlist built from local play counts has no artwork of its own.
 */
export const ON_REPEAT_ID = 'LIMUSIC_ON_REPEAT';

/**
 * Liked Music's browseId. YouTube edits this one through the rating endpoint, not `edit_playlist`,
 * so it is never an add/remove/rename target: liking the song is the edit.
 */
export const LIKED_MUSIC_ID = 'VLLM';

/**
 * YouTube Music's own Library ▸ Songs, despite the name: the songs saved to the account's library.
 * It browses like a playlist (no header, no sort menu), so `getPlaylist` reads it and the Library
 * page's Songs tab pages through it with `getPlaylistMore`.
 */
export const LIBRARY_SONGS_ID = 'FEmusic_liked_videos';

/**
 * The tracks the signed-in user uploaded to YouTube Music themselves. Browses like the songs grid
 * above, so the same tab component reads it; the rows come back with `is_upload` set, which is what
 * sends them down the login-only fallback chain when they play (issue #71).
 */
export const LIBRARY_UPLOADS_ID = 'FEmusic_library_privately_owned_tracks';

/**
 * Local music (Rust `local.rs`). A file on disk is a song whose `video_id` is `LOCAL:<path>`, and
 * an album of them is a browseId `LOCALALBUM:<key>` — so local items ride every existing surface
 * (cards, queue, Shortcuts, the album page) and play with no network.
 */
export const LOCAL_SONG_PREFIX = 'LOCAL:';
export const LOCAL_ALBUM_PREFIX = 'LOCALALBUM:';
/** An artist on this disk. Renders through the album route: same page, no YouTube channel. */
export const LOCAL_ARTIST_PREFIX = 'LOCALARTIST:';
export const isLocalId = (id: string | undefined | null): boolean =>
	!!id &&
	(id.startsWith(LOCAL_SONG_PREFIX) ||
		id.startsWith(LOCAL_ALBUM_PREFIX) ||
		id.startsWith(LOCAL_ARTIST_PREFIX));

export interface LocalLibrary {
	/** Watched folders, as absolute paths. */
	folders: string[];
	albums: BrowseItem[];
	artists: BrowseItem[];
	songs: SongItem[];
	/** Song/album/artist ids that were in the library but are gone from disk since the last scan. */
	removed: string[];
}

/** The orders YouTube itself can put a playlist in — everything in `SortKey` but our own `plays`. */
export type ServerSort = 'default' | 'newest' | 'oldest' | 'title' | 'artist' | 'album' | 'top';

export interface SortMenu {
	/** The order YouTube has this list in right now, when it is one we have a name for. */
	selected?: ServerSort;
	/**
	 * The choice is a write, so storing it makes YouTube Music and every other client follow.
	 * Playlists you own only: elsewhere the menu is view-only (Liked Music remembers the last order
	 * asked for anyway, someone else's playlist does not).
	 */
	editable: boolean;
}

/** One day bucket of the play history: YouTube's own heading plus that day's rows. */
export interface HistoryGroup {
	title: string;
	items: SongItem[];
}

export interface PlaylistPage {
	title?: string;
	subtitle?: string;
	thumbnail?: string;
	/** The playlist's own blurb, which the edit dialog prefills its description with. */
	description?: string;
	/** `PUBLIC` / `PRIVATE` / `UNLISTED`. Only playlists you own report it. */
	privacy?: string;
	/** Custom artwork picked on this machine; falls back to `thumbnail` when unset. */
	cover?: string;
	items: SongItem[];
	continuation?: string;
	/** True only when the signed-in user owns this playlist (rename/delete allowed). */
	owned: boolean;
	/** Collaboration is on: others can add to it, and each person may remove only what they added. */
	collaborative: boolean;
	/** Absent on lists YouTube will not reorder: albums, its own radio mixes, On Repeat. */
	sortMenu?: SortMenu;
}
export interface PlaylistContinuation {
	items: SongItem[];
	continuation?: string;
}

export interface ArtistCarousel {
	title: string;
	items: BrowseItem[];
	moreBrowseId?: string;
	moreParams?: string;
}
export interface SearchResults {
	top: BrowseItem[];
	songs: BrowseItem[];
	albums: BrowseItem[];
	artists: BrowseItem[];
	playlists: BrowseItem[];
}

export interface AlbumPage {
	title?: string;
	artist?: string;
	artistId?: string;
	/** The artist line run by run — links each artist of a collaborative album separately. */
	artistRuns?: ArtistRun[];
	artistThumbnail?: string;
	subtitle?: string;
	secondSubtitle?: string;
	description?: string;
	thumbnail?: string;
	items: SongItem[];
	continuation?: string;
	/** The album itself is flagged explicit (the header wears the badge, not just some tracks). */
	explicit?: boolean;
	/** The album's audio playlist id (`OLAK5uy_…`) — autoplay's radio seed, and the save target. */
	playlistId?: string;
	/** Already saved to the signed-in user's library. */
	inLibrary: boolean;
	/** Card shelves under the tracks: other versions, more from the artist, related releases. */
	sections?: ArtistCarousel[];
}

export interface ArtistPage {
	name?: string;
	thumbnail?: string;
	description?: string;
	subscribers?: string;
	monthlyListeners?: string;
	channelId: string;
	subscribed: boolean;
	topSongs: SongItem[];
	/** `VL…` playlist of all the artist's top songs, behind the shelf's "See all". */
	topSongsId?: string;
	sections: ArtistCarousel[];
}

// --- commands (context/11) -----------------------------------------------------------------
export const search = (query: string) => invoke<SongItem[]>('search', { query });
/** Unfiltered search → categorized sections. */
export const searchAll = (query: string) => invoke<SearchResults>('search_all', { query });
/** Filtered "Show more" card search for one category (albums / artists / playlists). */
export const searchCards = (query: string, category: 'albums' | 'artists' | 'playlists') =>
	invoke<BrowseItem[]>('search_cards', { query, category });
export const play = (item: SongItem) => invoke<void>('play', { item });
export const playIndex = (index: number) => invoke<void>('play_index', { index });
/** Remove an upcoming track from the queue (host/local only — guests are add-only). */
export const removeFromQueue = (index: number) => invoke<void>('remove_from_queue', { index });
/** Drag-to-reorder: move the upcoming queue item at `from` to index `to` (both past the playing
 * track — the history and the playing row don't move). */
export const moveInQueue = (from: number, to: number) =>
	invoke<void>('move_in_queue', { from, to });
/**
 * "Play next": insert tracks at the front of the "Next in queue" block, behind any earlier
 * "Play next" adds. `from` is the album/playlist they came from — it heads the block in the panel.
 */
export const playNext = (items: SongItem[], from?: string) =>
	invoke<void>('play_next', { items, from });
/**
 * "Add to queue": the tracks go at the *back* of the same block — after everything already queued
 * by hand, ahead of the playing context and anything the app generated behind it.
 * `continuation` is the source page's next-page token — the backend walks the rest of a long
 * playlist into the queue in the background.
 */
export const addToQueue = (items: SongItem[], from?: string, continuation?: string) =>
	invoke<void>('add_to_queue', { items, from, continuation });
/** Clear every upcoming manually-queued track (the "Next in queue" section). */
export const clearQueued = () => invoke<void>('clear_queued');
export const nextTrack = () => invoke<void>('next_track');
export const prevTrack = () => invoke<void>('prev_track');
export const toggleShuffle = () => invoke<void>('toggle_shuffle');
export const setRepeat = (mode: RepeatMode) => invoke<void>('set_repeat', { mode });
export const togglePause = () => invoke<void>('toggle_pause');
export const seek = (position: number) => invoke<void>('seek', { position });
export const setVolume = (volume: number) => invoke<void>('set_volume', { volume });
/** Tempo (0.25–2.0) + pitch (−12..=12 semitones). Not persisted: resets on restart. */
export const setPlaybackParams = (speed: number, semitones: number) =>
	invoke<void>('set_playback_params', { speed, semitones });
export const getQueue = () => invoke<QueueState>('get_queue');
/** A `limusicvideo://` URL for the track's music video, or null when there isn't one. `maxHeight`
 *  caps the picture at what the box on screen can actually show. The bytes are proxied through
 *  Rust; the webview never sees a googlevideo URL. */
export const videoStream = (videoId: string, maxHeight: number) =>
	invoke<string | null>('video_stream', { videoId, maxHeight });

/** Drop the backend's memory of this track's video URL, after the element failed to load it. */
export const forgetVideoStream = (videoId: string) =>
	invoke<void>('forget_video_stream', { videoId });

/** What the event stream already reported, for a webview that started after it did. */
export interface PlaybackSnapshot {
	now: NowPlaying | null;
	paused: boolean;
	position: number;
	duration: number;
	/** The level restored from last run (or the one another window already set). */
	volume: number;
}
export const getPlayback = () => invoke<PlaybackSnapshot>('get_playback');

// --- settings (context/11) -----------------------------------------------------------------
export const getSettings = () => invoke<Record<string, string>>('get_settings');
export const setSetting = (key: string, value: string) =>
	invoke<void>('set_setting', { key, value });
/** Streamable client keys for the "disabled clients" setting. */
export const getStreamClients = () => invoke<string[]>('get_stream_clients');
/** Wipe both cache tiers (URL cache + mpv on-disk audio cache). */
export const clearCaches = () => invoke<void>('clear_caches');
/** Grant the webview a URL for one font file the user picked, so `@font-face` can load it. */
export const allowFontFile = (path: string) => invoke<void>('allow_font_file', { path });

/** One published release: the GitHub release description, verbatim markdown. */
export interface ReleaseNote {
	version: string;
	/** `YYYY-MM-DD` */
	date: string;
	body: string;
}
/** Changelog for Settings > About, from the GitHub releases API (cached in Rust per run). */
export const releaseNotes = () => invoke<ReleaseNote[]>('release_notes');
/** False on Linux builds that aren't the AppImage (.rpm, the AUR package): they update through the
 *  package manager, so the UI offers a download link instead of an install button. */
export const canSelfUpdate = () => invoke<boolean>('can_self_update');
/** Open an http(s) link in the real browser, never in the webview itself. */
export const openExternal = (url: string) => invoke<void>('open_external', { url });

/** Environment + the redacted tail of `limusic.log`, for pasting into a bug report. */
export const diagnostics = () => invoke<string>('diagnostics');
/** Just the environment block, for prefilling the GitHub bug form. */
export const diagnosticsSummary = () => invoke<string>('diagnostics_summary');
/** The same text, written to a path the user picked in a save dialog. */
export const saveDiagnostics = (path: string) => invoke<void>('save_diagnostics', { path });

// --- auth (context/15) ---------------------------------------------------------------------
export const getAccount = () => invoke<Account>('get_account');
export const getAccountIdentities = () =>
	invoke<AccountIdentity[]>('get_account_identities');
export const switchAccount = (selectionKey: string) =>
	invoke<Account>('switch_account', { selectionKey });
export const signOut = () => invoke<void>('sign_out');
/** Open the in-app Google sign-in webview (context/15 Path A). Result arrives via onAuthChanged. */
export const loginWebview = () => invoke<void>('login_webview');

// --- mini player (Rust mini.rs) ---------------------------------------------------------------
/** Hide the app to the tray and open the floating widget (a second window running this same SPA). */
export const openMini = () => invoke<void>('open_mini');
/** Close the widget and bring the app back. */
export const closeMini = () => invoke<void>('close_mini');

// --- browse / library (context/08) ---------------------------------------------------------
/** `params` is a `HomeChip.params` token — omit for the unfiltered feed. */
export const getHome = (params?: string) => invoke<HomePage>('get_home', { params });
export const getHomeMore = (token: string) => invoke<HomePage>('get_home_more', { token });
export const getLibrary = () => invoke<BrowseItem[]>('get_library');
export const getLibraryAlbums = () => invoke<BrowseItem[]>('get_library_albums');
export const getLibraryArtists = () => invoke<BrowseItem[]>('get_library_artists');
export const getUploadAlbums = () => invoke<BrowseItem[]>('get_upload_albums');
/**
 * The account's YouTube Music play history, in YouTube's own day buckets (Today, Yesterday, …).
 * Empty when signed out.
 */
export const getHistory = () => invoke<HistoryGroup[]>('get_history');
/**
 * `sort` asks YouTube to order the tracks; omit it to get whatever order the account already has
 * the list in, which is the one a fresh visit wants (it is what YouTube Music would show).
 */
export const getPlaylist = (id: string, sort?: ServerSort, desc?: boolean) =>
	invoke<PlaylistPage>('get_playlist', { id, sort, desc });
/**
 * Store a sort order on a playlist, so YouTube Music and every other client show it the same way.
 * Only for a list whose `sortMenu.editable` is true.
 */
export const setPlaylistSort = (playlistId: string, sort: ServerSort) =>
	invoke<void>('set_playlist_sort', { playlistId, sort });
export const getPlaylistMore = (token: string) =>
	invoke<PlaylistContinuation>('get_playlist_more', { token });
/**
 * videoId → the ids of the playlists you own that hold it. Read straight from local SQLite, so it
 * answers instantly and is empty until `syncPlaylistIndex` has filled it in at least once.
 */
export const playlistIndex = () => invoke<Record<string, string[]>>('playlist_index');
/**
 * Re-walk your own playlists and answer with the rebuilt map. Skips the crawl while the stored one
 * is still inside its window, so calling this on every launch is cheap.
 */
export const syncPlaylistIndex = () => invoke<Record<string, string[]>>('sync_playlist_index');
/**
 * videoId → times played, from the local listening history. Same trailing window On Repeat uses
 * (a month): the history table is pruned to it, so there is no older data. A videoId that isn't in
 * the map has not been played inside the window.
 */
export const getPlayCounts = () => invoke<Record<string, number>>('play_counts');
/**
 * `start`: the clicked track index, or `null` for "just play it" (random opener under shuffle).
 * `sourceId`: the page's playlist/album playlist id — makes autoplay continue with that
 * context's radio (omit to fall back to song radio seeded from the queue's last track).
 * `sourceName`: the page title, for the queue panel's "Next from" header.
 * `shuffle`: turn shuffle on for this queue — pass items in their real order, Rust shuffles.
 */
export const playPlaylist = (
	items: SongItem[],
	start: number | null,
	sourceId?: string,
	sourceName?: string,
	shuffle?: boolean,
	continuation?: string
) => invoke<void>('play_playlist', { items, start, sourceId, sourceName, shuffle, continuation });
/**
 * Start a radio: an endless YouTube-generated queue seeded on this item. `id` is the videoId
 * (song) or browseId/playlistId (everything else) — Rust resolves it to a radio playlist, so the
 * UI never builds one. `name` titles the queue ("<name> Radio").
 *
 * A song radio on the track that's already playing splices in behind it (no re-buffer); every
 * other case replaces the queue. Rejects when YouTube has no radio for the item.
 */
export const startRadio = (kind: 'song' | 'artist' | 'album' | 'playlist', id: string, name?: string) =>
	invoke<void>('start_radio', { kind, id, name });
export const getAlbum = (id: string) => invoke<AlbumPage>('get_album', { id });
export const getArtist = (id: string) => invoke<ArtistPage>('get_artist', { id });
export const getBrowseGrid = (id: string, params?: string) =>
	invoke<BrowseItem[]>('get_browse_grid', { id, params });

// --- local music (local.rs) ------------------------------------------------------------------
/** Rescan the watched folders. Cheap when nothing changed (one stat per file). */
export const getLocalLibrary = () => invoke<LocalLibrary>('get_local_library');
export const addLocalFolder = (path: string) => invoke<LocalLibrary>('add_local_folder', { path });
export const removeLocalFolder = (path: string) =>
	invoke<LocalLibrary>('remove_local_folder', { path });

// --- write actions (context/01 ✎) ----------------------------------------------------------
/** Like, dislike, or clear the rating. YouTube's three states are mutually exclusive, so a dislike
 *  un-likes in the same call. */
export const rate = (videoId: string, rating: Rating) => invoke<void>('rate', { videoId, rating });
/** `false` = the playlist already had this track, so YouTube added nothing. */
export const addToPlaylist = (playlistId: string, videoId: string) =>
	invoke<boolean>('add_to_playlist', { playlistId, videoId });
export const removeFromPlaylist = (playlistId: string, videoId: string, setVideoId: string) =>
	invoke<void>('remove_from_playlist', { playlistId, videoId, setVideoId });
export const createPlaylist = (title: string) => invoke<string>('create_playlist', { title });
/** Name / description / visibility, from the "Edit playlist" dialog. Leave a field out and
 *  YouTube is never told about it, so an untouched one can't be overwritten. */
export const editPlaylistDetails = (
	playlistId: string,
	changes: { name?: string; description?: string; public?: boolean }
) => invoke<void>('edit_playlist_details', { playlistId, ...changes });
/** Custom playlist artwork. `path` is a file the user picked; `null` drops it. Answers where the
 *  local copy went, and on a removal the thumbnail YouTube rebuilt from the tracks (that one is
 *  worth waiting for: YouTube's own thumbnail is the cover being removed until it lands). */
export const setPlaylistCover = (playlistId: string, path: string | null) =>
	invoke<{ cover?: string; thumbnail?: string }>('set_playlist_cover', { playlistId, path });
export const deletePlaylist = (playlistId: string) =>
	invoke<void>('delete_playlist', { playlistId });
export const subscribe = (channelId: string, subscribed: boolean) =>
	invoke<void>('subscribe', { channelId, subscribed });
/** Add a song to Library ▸ Songs, or take it out. `token` is `SongItem.library.add_token` /
 *  `.remove_token`; YouTube mints them per row, so they come from the list the song was shown in. */
export const setSongSaved = (token: string) => invoke<void>('set_song_saved', { token });
/** Save an album to the library (or remove it). `playlistId` is `AlbumPage.playlistId`. */
export const setAlbumSaved = (playlistId: string, saved: boolean) =>
	invoke<void>('set_album_saved', { playlistId, saved });

// --- events (context/11). Each returns an unlisten fn; call it on component teardown. --------
export const onNowPlaying = (cb: (n: NowPlaying) => void): Promise<UnlistenFn> =>
	listen<NowPlaying>('now-playing', (e) => cb(e.payload));
/**
 * The backend asked YouTube what a track's rating really is and got a different answer than the
 * row we were handed (issue #93). Fires only on a change, at most once per track start.
 */
export const onRating = (cb: (videoId: string, rating: Rating) => void): Promise<UnlistenFn> =>
	listen<{ videoId: string; rating: Rating }>('rating', (e) =>
		cb(e.payload.videoId, e.payload.rating)
	);
export const onQueueChanged = (cb: (q: QueueState) => void): Promise<UnlistenFn> =>
	listen<QueueState>('queue-changed', (e) => cb(e.payload));
/**
 * The queue moved but its track list did not: only the play pointer and the flags changed.
 * Emitted instead of `queue-changed` on every advance and skip, because the full item list is
 * megabytes on a big playlist and a Tauri event delivers its payload as JavaScript *source*.
 * `current` carries the playing row so a metadata backfill (duration, artists) still lands.
 */
export interface QueueIndex {
	currentIndex: number;
	playedFrom?: number;
	shuffle?: boolean;
	repeat?: RepeatMode;
	sourceName?: string | null;
	current: SongItem | null;
}

export const onQueueIndex = (cb: (q: QueueIndex) => void): Promise<UnlistenFn> =>
	listen<QueueIndex>('queue-index', (e) => cb(e.payload));
/**
 * Autoplay topped the queue up at the tail. Carries only the new rows plus the resulting length, so
 * an endless radio session does not re-ship the whole list (which a Tauri event delivers as
 * JavaScript *source*) every twenty tracks. `len` is the resync guard: if the array we hold does
 * not reach that length once the rows are appended, an event was missed and the panel refetches.
 */
export interface QueueAppended {
	items: SongItem[];
	len: number;
	currentIndex: number;
	playedFrom?: number;
}

export const onQueueAppended = (cb: (q: QueueAppended) => void): Promise<UnlistenFn> =>
	listen<QueueAppended>('queue-appended', (e) => cb(e.payload));
/** Main window shown/hidden (close-to-tray, the mini player). WebKitGTK never tells the page. */
export const onUiVisible = (cb: (v: boolean) => void): Promise<UnlistenFn> =>
	listen<boolean>('ui-visible', (e) => cb(e.payload));
export const onPosition = (cb: (p: number) => void): Promise<UnlistenFn> =>
	listen<{ position: number }>('position', (e) => cb(e.payload.position));
export const onDuration = (cb: (d: number) => void): Promise<UnlistenFn> =>
	listen<{ duration: number }>('duration', (e) => cb(e.payload.duration));
/** Echo of every `set_volume`, so a second window's slider can't drift from what you hear. */
export const onVolume = (cb: (v: number) => void): Promise<UnlistenFn> =>
	listen<number>('volume', (e) => cb(e.payload));
export const onPlaybackState = (cb: (s: 'playing' | 'paused') => void): Promise<UnlistenFn> =>
	listen<'playing' | 'paused'>('playback-state', (e) => cb(e.payload));
export const onPlaybackError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<{ message: string }>('playback-error', (e) => cb(e.payload.message));
export const onPlaybackNotice = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<{ message: string }>('playback-notice', (e) => cb(e.payload.message));
/** Custom playlist artwork applied here but refused by YouTube Music (it syncs in the background,
 *  so the failure lands long after the picker closed). */
export const onCoverError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<{ message: string }>('cover-error', (e) => cb(e.payload.message));
export const onAuthChanged = (cb: (a: Account) => void): Promise<UnlistenFn> =>
	listen<Account>('auth-changed', (e) => cb(e.payload));
export const onAccountSelectionRequired = (cb: () => void): Promise<UnlistenFn> =>
	listen('account-selection-required', () => cb());
/**
 * Local music disappeared from disk. Fired when a play attempt finds nothing there, carrying the
 * song (and album, if that emptied it) so every view holding those ids can drop them at once.
 */
export const onLocalChanged = (cb: (removed: string[]) => void): Promise<UnlistenFn> =>
	listen<{ removed: string[] }>('local-changed', (e) => cb(e.payload.removed));
export const onLoginError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<string>('login-error', (e) => cb(e.payload));
export const onLoginDone = (cb: () => void): Promise<UnlistenFn> =>
	listen('login-done', () => cb());

// --- lyrics ---------------------------------------------------------------------------------
export interface LyricWord {
	text: string;
	start_ms: number;
	end_ms: number;
}
export interface LyricLine {
	/** Start cue in milliseconds; present ⇔ the line is synced. */
	time_ms?: number;
	end_time_ms?: number;
	text: string;
	words?: LyricWord[];
	translation?: string;
}
export interface Lyrics {
	/** Attribution for the panel footer ("LRCLIB", "Source: Musixmatch", …). */
	source: string;
	synced: boolean;
	instrumental: boolean;
	lines: LyricLine[];
}
/** Cached on the Rust side (provider chain: LRCLIB → YT Music). `null` = none found. */
export const getLyrics = (args: {
	videoId: string;
	title: string;
	artists: string;
	album?: string;
	duration?: number;
}) => invoke<Lyrics | null>('get_lyrics', args);

// --- Window ------------------------------------------------------------------------------------
/** Theater mode's fullscreen. Not `getCurrentWindow().setFullscreen` (#139): Windows needs the
 *  maximized state undone first and the frame recalculated after, in that order, on the main
 *  thread. Rust also puts the maximized state back when theater closes. */
export const theaterFullscreen = (on: boolean) => invoke<void>('theater_fullscreen', { on });

// --- Last.fm scrobbling ---------------------------------------------------------------------
export interface LastfmState {
	connected: boolean;
	username?: string | null;
	/** Set when a connect attempt failed (timeout, network, rejected) — show it as a toast. */
	error?: string | null;
}
export const lastfmStatus = () => invoke<LastfmState>('lastfm_status');
/** Opens the browser auth flow; the outcome arrives via onLastfmState, not this promise. */
export const lastfmConnect = () => invoke<void>('lastfm_connect');
/** Also cancels an in-flight connect (the auth poll checks and bails). */
export const lastfmDisconnect = () => invoke<void>('lastfm_disconnect');
export const onLastfmState = (cb: (s: LastfmState) => void): Promise<UnlistenFn> =>
	listen<LastfmState>('lastfm-state', (e) => cb(e.payload));

// --- Listen Together (context/19) -----------------------------------------------------------
export interface LtUser {
	user_id: string;
	username: string;
	is_host: boolean;
	is_connected: boolean;
}
export interface LtTrack {
	id: string;
	title: string;
	artist: string;
	thumbnail?: string | null;
	duration_ms: number;
	/** Name of the guest who added this track to the session queue. */
	queued_by?: string | null;
}
export interface LtPendingJoin {
	userId: string;
	username: string;
}
export interface LtSuggestion {
	id: string;
	from_user_id: string;
	from_username: string;
	track: LtTrack;
}
export interface LtState {
	status: 'disconnected' | 'connecting' | 'connected';
	role: 'none' | 'host' | 'guest';
	/** Asked to create/join and awaiting the room (host approval) — show a waiting state. */
	requesting: boolean;
	roomCode: string | null;
	myId: string | null;
	serverUrl: string;
	users: LtUser[];
	currentTrack: LtTrack | null;
	queue: LtTrack[];
	pendingJoins: LtPendingJoin[];
	suggestions: LtSuggestion[];
}

export const ltGetState = () => invoke<LtState>('lt_get_state');
export const ltSetServerUrl = (url: string) => invoke<void>('lt_set_server_url', { url });
export const ltCreateRoom = (username: string) => invoke<void>('lt_create_room', { username });
export const ltJoinRoom = (code: string, username: string) =>
	invoke<void>('lt_join_room', { code, username });
export const ltLeave = () => invoke<void>('lt_leave');
export const ltApproveJoin = (userId: string) => invoke<void>('lt_approve_join', { userId });
export const ltRejectJoin = (userId: string) => invoke<void>('lt_reject_join', { userId });
export const ltKick = (userId: string) => invoke<void>('lt_kick', { userId });
export const ltTransferHost = (userId: string) => invoke<void>('lt_transfer_host', { userId });
export const ltApproveSuggestion = (id: string) => invoke<void>('lt_approve_suggestion', { id });
export const ltRejectSuggestion = (id: string) => invoke<void>('lt_reject_suggestion', { id });
export const ltRequestSync = () => invoke<void>('lt_request_sync');

export const onLtState = (cb: (s: LtState) => void): Promise<UnlistenFn> =>
	listen<LtState>('lt-state', (e) => cb(e.payload));
export const onLtNotice = (cb: (msg: string) => void): Promise<UnlistenFn> =>
	listen<string>('lt-notice', (e) => cb(e.payload));
