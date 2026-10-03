import * as api from './api.ts';
import type { ImportTrack, SongItem } from './api.ts';
import { applyMatchResult, type ImportMatch } from './importer-helpers.ts';
import { t, type TranslationKey } from './i18n.svelte';

export const importerState = $state({
	source: '',
	owner: null as string | null,
	fromSpotify: false,
	retryable: false,
	loading: false,
	tracks: [] as ImportTrack[],
	results: [] as (ImportMatch | undefined)[],
	choice: [] as (number | null)[],
	picked: [] as boolean[],
	running: false,
	done: 0,
	total: 0,
	error: null as string | null
});

let activeJobId: string | null = null;
let loadGen = 0;
let lastLoad: (() => Promise<void>) | null = null;
let unlistenMatch: (() => void) | null = null;
let unlistenDone: (() => void) | null = null;

export function reset() {
	loadGen++;
	if (activeJobId) {
		api.importCancel(activeJobId).catch(() => {});
	}
	if (unlistenMatch) {
		unlistenMatch();
		unlistenMatch = null;
	}
	if (unlistenDone) {
		unlistenDone();
		unlistenDone = null;
	}
	activeJobId = null;
	lastLoad = null;

	importerState.source = '';
	importerState.owner = null;
	importerState.fromSpotify = false;
	importerState.retryable = false;
	importerState.loading = false;
	importerState.tracks = [];
	importerState.results = [];
	importerState.choice = [];
	importerState.picked = [];
	importerState.running = false;
	importerState.done = 0;
	importerState.total = 0;
	importerState.error = null;
}

export async function loadFromFile(path: string) {
	reset();
	const gen = ++loadGen;
	lastLoad = () => loadFromFile(path);
	try {
		const tracks = await api.importLoadFile(path);
		if (gen !== loadGen) return;
		importerState.source = path.split(/[/\\]/).pop() || path;
		importerState.tracks = tracks;
		importerState.results = new Array(tracks.length).fill(undefined);
		importerState.choice = new Array(tracks.length).fill(null);
		importerState.picked = new Array(tracks.length).fill(false);
		importerState.total = tracks.length;
	} catch (e: any) {
		if (gen !== loadGen) return;
		importerState.error = String(e);
		importerState.retryable = false;
	}
}

export async function loadFromText(csv: string) {
	reset();
	const gen = ++loadGen;
	lastLoad = () => loadFromText(csv);
	try {
		const tracks = await api.importParseCsv(csv);
		if (gen !== loadGen) return;
		importerState.source = 'Clipboard / Text';
		importerState.tracks = tracks;
		importerState.results = new Array(tracks.length).fill(undefined);
		importerState.choice = new Array(tracks.length).fill(null);
		importerState.picked = new Array(tracks.length).fill(false);
		importerState.total = tracks.length;
	} catch (e: any) {
		if (gen !== loadGen) return;
		importerState.error = String(e);
		importerState.retryable = false;
	}
}

export async function loadFromSpotifyLink(url: string) {
	reset();
	const gen = ++loadGen;
	lastLoad = () => loadFromSpotifyLink(url);
	importerState.loading = true;
	try {
		const playlist = await api.importFetchSpotify(url);
		if (gen !== loadGen) return;
		const tracks = playlist.tracks;
		importerState.source = playlist.name;
		importerState.owner = playlist.owner ?? null;
		importerState.fromSpotify = true;
		importerState.tracks = tracks;
		importerState.results = new Array(tracks.length).fill(undefined);
		importerState.choice = new Array(tracks.length).fill(null);
		importerState.picked = new Array(tracks.length).fill(false);
		importerState.total = tracks.length;
	} catch (e: any) {
		if (gen !== loadGen) return;
		const errStr = String(e);
		const SPOTIFY_ERRORS = {
			spotify_invalid_link: 'import.err_spotify_invalid_link',
			spotify_unavailable: 'import.err_spotify_unavailable',
			spotify_changed: 'import.err_spotify_changed',
			spotify_empty: 'import.err_spotify_empty',
			spotify_network: 'import.err_spotify_network',
		} as const satisfies Record<string, TranslationKey>;
		importerState.error = errStr in SPOTIFY_ERRORS ? t(SPOTIFY_ERRORS[errStr as keyof typeof SPOTIFY_ERRORS]) : errStr;
		importerState.retryable = (errStr === 'spotify_network');
	} finally {
		if (gen !== loadGen) return;
		importerState.loading = false;
	}
}

export async function retryLastLoad() {
	if (lastLoad) {
		await lastLoad();
	}
}

export async function startMatching() {
	if (importerState.running || importerState.tracks.length === 0) return;
	importerState.running = true;
	importerState.done = 0;
	importerState.error = null;

	const jobId = 'import_' + Math.random().toString(36).slice(2) + Date.now().toString(36);
	activeJobId = jobId;

	if (unlistenMatch) {
		unlistenMatch();
		unlistenMatch = null;
	}
	if (unlistenDone) {
		unlistenDone();
		unlistenDone = null;
	}

	try {
		unlistenMatch = await api.onImportMatch((event) => {
			if (event.job_id !== activeJobId) return;
			const index = event.result.index;
			if (index < 0 || index >= importerState.tracks.length) return;
			importerState.done = event.done;
			applyMatchResult(importerState, event.result);
		});

		unlistenDone = await api.onImportDone((event) => {
			if (event.job_id !== activeJobId) return;
			importerState.running = false;
			if (unlistenMatch) {
				unlistenMatch();
				unlistenMatch = null;
			}
			if (unlistenDone) {
				unlistenDone();
				unlistenDone = null;
			}
			if (activeJobId === jobId) {
				activeJobId = null;
			}
		});

		await api.importMatch(jobId, importerState.tracks);
	} catch (e: any) {
		importerState.error = String(e);
		importerState.running = false;
		if (unlistenMatch) {
			unlistenMatch();
			unlistenMatch = null;
		}
		if (unlistenDone) {
			unlistenDone();
			unlistenDone = null;
		}
		if (activeJobId === jobId) {
			activeJobId = null;
		}
	}
}

export async function cancel() {
	if (!activeJobId || !importerState.running) return;
	try {
		await api.importCancel(activeJobId);
	} catch (e) {
		console.error('Failed to cancel import job', e);
	}
}

export function setChoice(i: number, k: number | null) {
	const newChoice = [...importerState.choice];
	newChoice[i] = k;
	importerState.choice = newChoice;

	const newPicked = [...importerState.picked];
	newPicked[i] = k !== null;
	importerState.picked = newPicked;
}

export function togglePicked(i: number) {
	if (importerState.choice[i] === null) return;
	const newPicked = [...importerState.picked];
	newPicked[i] = !newPicked[i];
	importerState.picked = newPicked;
}

export function pickAllMatched() {
	const newPicked = [...importerState.picked];
	for (let i = 0; i < importerState.tracks.length; i++) {
		const res = importerState.results[i];
		if (res && res.status === 'matched' && importerState.choice[i] !== null) {
			newPicked[i] = true;
		}
	}
	importerState.picked = newPicked;
}

export function selectedSongs(): SongItem[] {
	const songs: SongItem[] = [];
	for (let i = 0; i < importerState.tracks.length; i++) {
		if (!importerState.picked[i]) continue;
		const k = importerState.choice[i];
		if (k === null || k === undefined) continue;
		const res = importerState.results[i];
		if (!res) continue;
		const candidate = res.candidates[k];
		if (candidate && candidate.song) {
			songs.push(candidate.song as SongItem);
		}
	}
	return songs;
}
