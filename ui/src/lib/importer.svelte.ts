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

export function reset() {
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
	activeJobId = null;
}

export async function loadFromFile(path: string) {
	reset();
	try {
		const tracks = await api.importLoadFile(path);
		importerState.source = path.split(/[/\\]/).pop() || path;
		importerState.tracks = tracks;
		importerState.results = new Array(tracks.length).fill(undefined);
		importerState.choice = new Array(tracks.length).fill(null);
		importerState.picked = new Array(tracks.length).fill(false);
		importerState.total = tracks.length;
	} catch (e: any) {
		importerState.error = String(e);
		importerState.retryable = true;
	}
}

export async function loadFromText(csv: string) {
	reset();
	try {
		const tracks = await api.importParseCsv(csv);
		importerState.source = 'Clipboard / Text';
		importerState.tracks = tracks;
		importerState.results = new Array(tracks.length).fill(undefined);
		importerState.choice = new Array(tracks.length).fill(null);
		importerState.picked = new Array(tracks.length).fill(false);
		importerState.total = tracks.length;
	} catch (e: any) {
		importerState.error = String(e);
		importerState.retryable = true;
	}
}

export async function loadFromSpotifyLink(url: string) {
	reset();
	importerState.loading = true;
	try {
		const playlist = await api.importFetchSpotify(url);
		const tracks = playlist.tracks;
		importerState.source = playlist.name;
		importerState.owner = playlist.owner || null;
		importerState.fromSpotify = true;
		importerState.tracks = tracks;
		importerState.results = new Array(tracks.length).fill(undefined);
		importerState.choice = new Array(tracks.length).fill(null);
		importerState.picked = new Array(tracks.length).fill(false);
		importerState.total = tracks.length;
	} catch (e: any) {
		const errStr = String(e);
		const SPOTIFY_ERRORS = {
			spotify_invalid_link: 'import.err_spotify_invalid_link',
			spotify_unavailable: 'import.err_spotify_unavailable',
			spotify_changed: 'import.err_spotify_changed',
			spotify_empty: 'import.err_spotify_empty',
			spotify_network: 'import.err_spotify_network',
		} as const satisfies Record<string, TranslationKey>;
		importerState.error = errStr in SPOTIFY_ERRORS ? t(SPOTIFY_ERRORS[errStr as keyof typeof SPOTIFY_ERRORS]) : errStr;
		importerState.retryable = !(errStr === 'spotify_invalid_link' || errStr === 'spotify_unavailable' || errStr === 'spotify_empty');
	} finally {
		importerState.loading = false;
	}
}

export async function startMatching() {
	if (importerState.running || importerState.tracks.length === 0) return;
	importerState.running = true;
	importerState.done = 0;
	importerState.error = null;

	const jobId = 'import_' + Math.random().toString(36).slice(2) + Date.now().toString(36);
	activeJobId = jobId;

	let unlistenMatch: (() => void) | undefined;
	let unlistenDone: (() => void) | undefined;

	try {
		unlistenMatch = await api.onImportMatch((event) => {
			if (event.job_id !== jobId) return;
			importerState.done = event.done;
			applyMatchResult(importerState, event.result);
		});

		unlistenDone = await api.onImportDone((event) => {
			if (event.job_id !== jobId) return;
			importerState.running = false;
			if (unlistenMatch) unlistenMatch();
			if (unlistenDone) unlistenDone();
		});

		await api.importMatch(jobId, importerState.tracks);
	} catch (e: any) {
		importerState.error = String(e);
		importerState.running = false;
		if (unlistenMatch) unlistenMatch();
		if (unlistenDone) unlistenDone();
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
