<script module lang="ts">
	// The tempo the user had before the editor slowed the song, kept outside the component: the
	// root layout remounts the page on a refresh or a sign-in, and a second instance that asked
	// the player for "the current tempo" a moment after the first one let go would read the
	// editor's own 0.75x and restore to that forever.
	let homeSpeed: number | null = null;
	let engaged = false;
	/** Tempo changes in the order they were asked for: Rust takes them one at a time, but two
	 *  commands sent back to back are not promised to arrive in that order. */
	let speedChain: Promise<void> = Promise.resolve();
</script>

<script lang="ts">
	// The lyric editor: sync a song word by word, one line at a time, on a loop.
	//
	// This file holds the page's state and wires it to the player. Every rule (splitting words,
	// placing a tap, the loop, dragging an edge, what goes out) is a pure function in
	// `$lib/lyricsedit.ts`, checked by `lyricsedit.check.ts`.
	//
	// What it does to playback, and how each is undone:
	//   - tempo: set while `active`, put back by the same effect's cleanup, which runs when the page
	//     is left, another song starts, the window goes to the tray, the player view or theater
	//     covers the page, or a Listen Together room is joined;
	//   - position: the loop seeks. It is a timer on this page, so it stops with the page.
	// Nothing else: it never pauses, and never touches the queue or repeat.
	import { onDestroy, onMount, tick, untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		Alert02Icon,
		ArrowDown01Icon,
		ArrowDown02Icon,
		ArrowLeft01Icon,
		ArrowTurnBackwardIcon,
		ArrowUp01Icon,
		ClipboardIcon,
		ComputerIcon,
		Delete02Icon,
		FileImportIcon,
		InformationCircleIcon,
		Loading03Icon,
		MagicWand01Icon,
		MergeIcon,
		Mic01Icon,
		MoreHorizontalIcon,
		MusicNote01Icon,
		PencilEdit02Icon,
		ScissorIcon,
		SentIcon,
		Tap01Icon,
		TextFontIcon,
		Tick02Icon,
		UserGroupIcon,
		UserMultiple02Icon,
		ViewIcon
	} from '@hugeicons/core-free-icons';
	import { Button, buttonVariants } from '$lib/components/ui/button';
	import { Slider } from '$lib/components/ui/slider';
	import { Switch } from '$lib/components/ui/switch';
	import { Textarea } from '$lib/components/ui/textarea';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as Popover from '$lib/components/ui/popover';
	import LimusicNotice from '$lib/components/LimusicNotice.svelte';
	import LyricTimeline from '$lib/components/LyricTimeline.svelte';
	import LyricsView from '$lib/components/LyricsView.svelte';
	import * as api from '$lib/api';
	import { isLocalId } from '$lib/api';
	import { np, playAt, playback, seekTo, toast, ui } from '$lib/player.svelte';
	import { inRoom } from '$lib/lt.svelte';
	import { link } from '$lib/link.svelte';
	import { limusic, liveRevision, refreshAccount, submitLyrics } from '$lib/limusic.svelte';
	import { submitMessage, type Message } from '$lib/limusic';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { keyOf, typing } from '$lib/shortcuts';
	import { KIND_CLASS, KIND_LABEL } from '$lib/lyricsSources';
	import {
		buildLines,
		draftKey,
		estimateMs,
		eventWallMs,
		fieldText,
		fromLyrics,
		fromText,
		gapLine,
		joinLines,
		kindOf,
		lineEnd,
		lineSpan,
		lineText,
		loopRange,
		loopStep,
		missing,
		needsLineTime,
		packDraft,
		ready,
		resplitLine,
		setAgent,
		setLineText,
		setLineTime,
		splitLine,
		stampTap,
		toggleBg,
		unpackDraft,
		withWords,
		wordsFromTaps,
		wordsTimed,
		type EdLine,
		type EdWord
	} from '$lib/lyricsedit';

	const SPEED_KEY = 'lyrics_editor_speed';
	const OFFSET_KEY = 'lyrics_editor_tap_offset';
	const MAX_OFFSET_MS = 500;

	function stored(key: string, fallback: number, lo: number, hi: number): number {
		try {
			const raw = localStorage.getItem(key);
			const v = raw === null ? NaN : Number(raw);
			return Number.isFinite(v) && v >= lo && v <= hi ? v : fallback;
		} catch {
			return fallback;
		}
	}
	function store(key: string, value: string | null) {
		try {
			if (value === null) localStorage.removeItem(key);
			else localStorage.setItem(key, value);
			return true;
		} catch {
			return false;
		}
	}

	/** "3:21" / "1:02:03" in seconds. */
	function clockSecs(d?: string): number {
		const parts = (d ?? '').split(':').map(Number);
		return parts.length && !parts.some(Number.isNaN) ? parts.reduce((a, b) => a * 60 + b, 0) : 0;
	}

	// --- The song -----------------------------------------------------------------------------
	// The one playing when the page opened, for as long as the page lives. A local file has no
	// video id the service knows, and a broadcast has no timeline.
	const opened = playback.now;
	const track: api.LyricsTrack | null =
		opened && !isLocalId(opened.videoId) && !opened.live
			? {
					videoId: opened.videoId,
					title: opened.title,
					artists: opened.artists,
					album: opened.album ?? undefined,
					duration: clockSecs(opened.duration) || (playback.duration > 0 ? playback.duration : undefined)
				}
			: null;

	const onTrack = $derived(!!track && playback.now?.videoId === track.videoId);
	/** The song's length in ms, 0 while unknown. mpv's while the song is the one playing. */
	const durationMs = $derived(
		(onTrack && playback.duration > 0 ? playback.duration : (track?.duration ?? 0)) * 1000
	);

	// --- The draft ----------------------------------------------------------------------------
	type Mode = 'loading' | 'start' | 'edit' | 'preview';
	let mode = $state<Mode>('loading');
	// Raw, replaced whole on every edit: a few hundred lines of words read through a deep proxy on
	// each frame of a drag is work nothing here needs.
	let lines = $state.raw<EdLine[]>([]);
	let nextId = 1;
	let baseRevision: number | null = null;
	let improving = $state(false);
	let cur = $state(0);
	let saveFailed = $state(false);
	let saveTimer: ReturnType<typeof setTimeout> | undefined;
	/** The lyric is on the service: the draft is gone and nothing writes it back. */
	let sent = false;
	/** The draft on screen came from an earlier visit: said once, above the stage. */
	let restored = $state(false);
	/** Every line just got its words: the stage says so, once, until a line is picked again. */
	let celebrate = $state(false);

	function flush() {
		clearTimeout(saveTimer);
		saveTimer = undefined;
		if (!track || !lines.length || sent) return;
		const json = packDraft({ v: 1, videoId: track.videoId, lines, nextId, baseRevision, improving });
		saveFailed = !store(draftKey(track.videoId), json);
	}

	/** Every change to the draft lands here: on screen at once, on disk within half a second. */
	function setLines(next: EdLine[]) {
		lines = next;
		clearTimeout(saveTimer);
		saveTimer = setTimeout(flush, 400);
	}

	function updateLine(id: number, fn: (l: EdLine) => EdLine) {
		const i = lines.findIndex((l) => l.id === id);
		if (i < 0) return;
		const next = fn(lines[i]);
		if (next === lines[i]) return;
		const copy = lines.slice();
		copy[i] = next;
		setLines(copy);
	}

	/** The first line with something left to do, else the first. */
	const firstTodo = (ls: EdLine[]) =>
		Math.max(
			0,
			ls.findIndex((l) => l.words.length > 0 && !wordsTimed(l))
		);

	function begin(ls: EdLine[], isImproving: boolean) {
		if (!ls.length) {
			toast.error(t('lyrics_editor.start_failed'));
			return;
		}
		nextId = Math.max(...ls.map((l) => l.id)) + 1;
		improving = isImproving;
		cur = firstTodo(ls);
		setLines(ls);
		flush();
		mode = 'edit';
		select(cur);
	}

	// --- Starting points ----------------------------------------------------------------------
	let start = $state.raw<api.LyricsEditorStart | null>(null);
	let pasted = $state('');
	let fileInput = $state<HTMLInputElement | null>(null);
	const startLines = $derived(start?.lyrics ? fromLyrics(start.lyrics.lines) : []);
	const startKind = $derived(kindOf(startLines));

	async function loadStart() {
		if (!track) return;
		try {
			start = await api.lyricsEditorStart(track);
			baseRevision = start.base_revision;
		} catch {
			start = { lyrics: null, base_revision: null };
		}
	}

	/** Pasted or picked text: a lyric file if Rust's parsers read one out of it, else plain lines. */
	async function importText(text: string) {
		if (!text.trim()) return;
		try {
			const parsed = await api.lyricsParse(text);
			begin(parsed.length ? fromLyrics(parsed) : fromText(text), false);
		} catch (e) {
			toast.error(t(String(e) === 'too_large' ? 'lyrics_editor.too_large' : 'lyrics_editor.start_failed'));
		}
	}

	async function pickFile(e: Event) {
		const input = e.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		if (file.size > 1024 * 1024) return toast.error(t('lyrics_editor.too_large'));
		importText(await file.text());
	}

	let discardOpen = $state(false);
	function discard() {
		discardOpen = false;
		clearTimeout(saveTimer);
		if (track) store(draftKey(track.videoId), null);
		lines = [];
		saveFailed = false;
		restored = false;
		celebrate = false;
		cur = 0;
		mode = 'start';
		loadStart();
	}

	// --- What the editor may do to the player --------------------------------------------------
	let visible = $state(true);
	/** Why the tools are off, when they are. A room keeps everyone on one clock and refuses a tempo
	 *  change; a host's seeks go out to every guest. A linked device is not this player at all. */
	const blocked = $derived(inRoom() ? 'room' : link.remote ? 'remote' : null);
	/** The editor is driving playback: it is on screen, on its song, and nothing forbids it. */
	const active = $derived(
		mode === 'edit' && onTrack && !blocked && visible && !np.open && !ui.theaterOpen && !celebrate
	);

	let editSpeed = $state(stored(SPEED_KEY, 0.75, 0.5, 1));
	let speedDraft = $state<number | null>(null);
	let loopOn = $state(true);
	const savedOffset = stored(OFFSET_KEY, 0, -MAX_OFFSET_MS, MAX_OFFSET_MS);
	let offsetMs = $state(savedOffset);
	let offsetText = $state(String(savedOffset));

	function applySpeed(speed: number, quiet: boolean, then?: () => void) {
		speedChain = speedChain
			.then(async () => {
				if (playback.speed === speed) return;
				try {
					await api.setPlaybackParams(speed, playback.semitones, playback.reverb);
					// Confirmed here as well as by the event, so the next read is never the old value.
					playback.speed = speed;
				} catch (e) {
					// Putting the tempo back can be refused (a room has already reset it): nothing to say.
					if (!quiet) toast.error(String(e));
				}
			})
			.then(then);
	}

	// One effect owns the tempo. Its cleanup is the single way out, whatever ended `active`.
	$effect(() => {
		if (!active) return;
		engaged = true;
		homeSpeed ??= untrack(() => playback.speed);
		applySpeed(
			untrack(() => editSpeed),
			false
		);
		return () => {
			engaged = false;
			const home = homeSpeed;
			if (home === null) return;
			applySpeed(home, true, () => {
				if (!engaged) homeSpeed = null;
			});
		};
	});

	function commitSpeed(v: number) {
		editSpeed = Math.round(v * 20) / 20;
		speedDraft = null;
		store(SPEED_KEY, String(editSpeed));
		if (active) applySpeed(editSpeed, false);
	}

	function commitOffset() {
		const v = Math.round(Number(offsetText));
		offsetMs = Number.isFinite(v) ? Math.max(-MAX_OFFSET_MS, Math.min(MAX_OFFSET_MS, v)) : 0;
		offsetText = String(offsetMs);
		store(OFFSET_KEY, String(offsetMs));
	}

	const position = () =>
		estimateMs(playback.position, playback.positionAt, performance.now(), playback.speed, playback.paused);

	// --- The current line, its loop and its taps ------------------------------------------------
	const line = $derived<EdLine | undefined>(lines[cur]);
	const owesTime = $derived(!!line && needsLineTime(line));
	const range = $derived(line && !owesTime ? loopRange(lines, cur, durationMs) : null);

	/** One pass of taps over the current line. `tapCount` moves at the keypress so the next word
	 *  lights at once; each tap's time arrives a few ms later, into the slot it claimed. A new
	 *  generation (the loop wrapped, the line changed) orphans whatever is still in flight. */
	let taps: (number | undefined)[] = [];
	let takeGen = 0;
	let tapCount = $state(0);
	/** Where the taps of the pass in progress fell, for the track: gone once its words are placed. */
	let marks = $state.raw<number[]>([]);
	function resetTake() {
		takeGen++;
		taps = [];
		tapCount = 0;
		marks = [];
	}

	let lastSeekAt = -Infinity;
	function jump(ms: number) {
		lastSeekAt = performance.now();
		seekTo(ms / 1000);
	}

	// The loop. Re-run by every position report, which is also what corrects its timer.
	$effect(() => {
		if (!active || !loopOn || !range || playback.paused) return;
		const r = range;
		void playback.position;
		const speed = playback.speed;
		let timer: ReturnType<typeof setTimeout>;
		const look = () => {
			const step = loopStep(r, position(), speed, performance.now() - lastSeekAt);
			if ('seek' in step) {
				resetTake();
				jump(step.seek);
				return; // the seek moves `playback.position`, which runs this effect again
			}
			timer = setTimeout(look, step.wake);
		};
		untrack(look);
		return () => clearTimeout(timer);
	});

	function select(i: number, seek = true) {
		if (i < 0 || i >= lines.length) return;
		commitField();
		cur = i;
		resetTake();
		// Picking a line is how the "every line is timed" panel is left.
		celebrate = false;
		// Narrow, the sheet is under the stage: a line picked there is brought back into view.
		if (!wide) body?.scrollTo({ top: 0 });
		if (!seek || !active) return;
		const l = lines[i];
		if (needsLineTime(l)) {
			// No start to loop from: play up to it from the line before, and let the user tap.
			jump(lines.slice(0, i).findLast((p) => p.time !== undefined)?.time ?? 0);
		} else {
			const r = loopRange(lines, i, durationMs);
			if (r) jump(r.start);
		}
	}

	function restart() {
		resetTake();
		if (active && range) jump(range.start);
	}

	async function clockAt(wall: number): Promise<number | null> {
		const sample = await api.playbackClock().catch(() => null);
		return sample ? stampTap(sample, wall, offsetMs) : null;
	}

	async function tap(e: Event) {
		// First thing, before anything can delay it: when the key actually went down.
		const wall = eventWallMs(Date.now(), performance.now(), e.timeStamp);
		if (!active || !line || playback.paused) return;
		const id = line.id;
		// An instrumental break has nothing but a start: a tap sets it, and another moves it.
		if (owesTime || line.gap) {
			const at = cur;
			// Through an untimed lyric the cursor walks on with the song.
			if (owesTime && lines[at + 1] && needsLineTime(lines[at + 1])) cur = at + 1;
			const ms = await clockAt(wall);
			if (ms === null) cur = at;
			else updateLine(id, (l) => setLineTime(l, ms));
			return;
		}
		const n = line.words.length;
		if (!n || tapCount > n) return;
		const gen = takeGen;
		const slot = tapCount++;
		const ms = await clockAt(wall);
		if (gen !== takeGen) return;
		if (ms === null) return resetTake();
		taps[slot] = ms;
		marks = slot >= n - 1 ? [] : taps.filter((x): x is number => x !== undefined);
		// Where the line itself ends, not the loop: the lead-out belongs to the next line.
		const end = lineSpan(lines, cur, durationMs)?.end;
		updateLine(id, (l) => {
			const words = wordsFromTaps(l.words, taps, l.end ?? end);
			return words ? withWords(l, words) : l;
		});
	}

	// --- Text, singer, background ---------------------------------------------------------------
	let field = $state('');
	let fieldEl = $state<HTMLInputElement | null>(null);
	let editing = $state(false);
	/** The big line is a text field: "Edit text" was pressed. An empty line is one without asking. */
	let textOpen = $state(false);
	let bgMode = $state(false);
	async function openText() {
		textOpen = true;
		await tick();
		fieldEl?.focus();
	}
	/** The line `field` holds the text of. A commit for any other line would write one line's text
	 *  into another, or empty it: the field lags the cursor by an effect. */
	let fieldFor: number | null = null;
	$effect(() => {
		const text = line ? fieldText(line.words) : '';
		if (untrack(() => editing)) return;
		field = text;
		fieldFor = line?.id ?? null;
	});

	function commitField() {
		if (!line || fieldFor !== line.id) return;
		const before = line;
		updateLine(line.id, (l) => setLineText(l, field));
		if (lines[cur] !== before) resetTake();
	}

	function splitHere() {
		if (!line) return;
		const caret = fieldEl?.selectionStart ?? field.length;
		const [a, b] = splitLine(line, field, caret, nextId++);
		if (!a.words.length || !b.words.length) return;
		const copy = lines.slice();
		copy.splice(cur, 1, a, b);
		setLines(copy);
		field = fieldText(a.words);
		resetTake();
	}

	function joinNext() {
		const next = lines[cur + 1];
		if (!line || !next) return;
		commitField();
		const copy = lines.slice();
		copy.splice(cur, 2, joinLines(lines[cur], next));
		setLines(copy);
		field = fieldText(copy[cur].words);
		resetTake();
	}

	async function addBelow() {
		commitField();
		const copy = lines.slice();
		copy.splice(cur + 1, 0, { id: nextId++, words: [], dirty: true });
		setLines(copy);
		cur++;
		resetTake();
		await tick();
		fieldEl?.focus();
	}

	/** An instrumental break after this line: a music note in the lyrics until the next line
	 *  begins. It starts where this line's singing ends when that is known, and otherwise waits
	 *  for a tap. */
	function addGap() {
		if (!line) return;
		commitField();
		const copy = lines.slice();
		copy.splice(cur + 1, 0, gapLine(nextId++, lineEnd(lines[cur])));
		setLines(copy);
		select(cur + 1);
	}

	function removeLine() {
		if (!line || lines.length < 2) return;
		const copy = lines.slice();
		copy.splice(cur, 1);
		setLines(copy);
		cur = Math.min(cur, copy.length - 1);
		field = fieldText(copy[cur].words);
		fieldFor = copy[cur].id;
		resetTake();
	}

	function resplit() {
		if (!line) return;
		commitField();
		updateLine(lines[cur].id, resplitLine);
		resetTake();
	}

	/** Singer 1, singer 2, any other the lyric already names, and everyone together. */
	const agents = $derived(
		[
			...new Set(['v1', 'v2', ...lines.map((l) => l.agent).filter((a): a is string => !!a && a !== 'v1000')])
		].sort((a, b) => Number(a.slice(1)) - Number(b.slice(1)))
	);

	// --- Progress -------------------------------------------------------------------------------
	const miss = $derived(missing(lines));
	const sungCount = $derived(lines.filter((l) => l.words.length).length);
	function nextTodo() {
		const ids = new Set(miss.untimed);
		const after = lines.findIndex((l, i) => i > cur && ids.has(l.id));
		select(after >= 0 ? after : lines.findIndex((l) => ids.has(l.id)));
	}

	// --- The moment the song is ready ------------------------------------------------------------
	// `celebrate` only on the change from not ready to ready, so a lyric opened to be improved
	// does not start on that panel.
	let wasReady: boolean | null = null;
	$effect(() => {
		if (mode === 'start' || mode === 'loading') {
			wasReady = null;
			return;
		}
		const r = ready(miss);
		if (r && wasReady === false) celebrate = true;
		if (!r) celebrate = false;
		wasReady = r;
	});

	// --- Sending ---------------------------------------------------------------------------------
	let sendOpen = $state(false);
	let submitting = $state(false);
	/** What the service said to the last try, shown in the send dialog until the next one. */
	let result = $state.raw<Message | null>(null);

	function openSend() {
		if (!track || !ready(miss)) return;
		commitField();
		result = null;
		sendOpen = true;
		// Who it goes out as, and whether it is reviewed first: fresh, it is on the dialog.
		refreshAccount();
	}

	async function submit() {
		if (!track || !ready(miss) || submitting) return;
		commitField();
		flush();
		submitting = true;
		result = null;
		try {
			const outcome = await submitLyrics({
				videoId: track.videoId,
				lines: buildLines(lines),
				title: track.title,
				artist: track.artists,
				album: track.album,
				duration: track.duration,
				baseRevision
			});
			const m = submitMessage(outcome);
			if (m.sent) {
				// It is on the service now: the draft has done its job.
				clearTimeout(saveTimer);
				saveTimer = undefined;
				sent = true;
				store(draftKey(track.videoId), null);
			}
			result = m;
			// Closed while it was on its way: the answer is still owed.
			sendOpen = true;
		} finally {
			submitting = false;
		}
	}

	/** "Send mine" after a stale answer: the draft stays as it is and is sent on top of what is
	 *  live now. The new version is replaced by it, which is what the user asked for. */
	async function keepMine() {
		if (!track || submitting) return;
		try {
			baseRevision = await liveRevision(track.videoId);
		} catch {
			result = { title: 'lyrics_editor.err_unavailable_title', key: 'lyrics_editor.err_unavailable', kind: 'error' };
			return;
		}
		flush();
		await submit();
	}

	/** The dialog closing after a lyric went out is the way off the page. Once: the button and the
	 *  dialog's own close can both get here. */
	let left = false;
	function sendClosed() {
		if (!result?.sent || left) return;
		left = true;
		history.back();
	}

	const previewLyrics = $derived<api.Lyrics>({
		source: '',
		provider: '',
		synced: true,
		instrumental: false,
		pinned: false,
		offset_ms: 0,
		lines: buildLines(lines.filter((l) => l.time !== undefined || wordsTimed(l)))
	});

	// --- The list follows the current line ------------------------------------------------------
	let list = $state<HTMLElement | null>(null);
	$effect(() => {
		const i = cur;
		if (mode !== 'edit' || !list) return;
		const row = list.querySelector<HTMLElement>(`[data-line="${i}"]`);
		if (!row) return;
		// The list's own scroller, never `scrollIntoView`, which scrolls every ancestor too (#168).
		const a = row.getBoundingClientRect();
		const b = list.getBoundingClientRect();
		if (a.top < b.top || a.bottom > b.bottom) {
			list.scrollTop += a.top - b.top - (b.height - a.height) / 2;
		}
	});

	// --- Keys -----------------------------------------------------------------------------------
	// None of these collide with the app's own: every global shortcut but Space and `;` hangs off
	// Ctrl or Cmd, and Space stays play/pause here.
	function onKey(e: KeyboardEvent) {
		if (mode !== 'edit' || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
		if (celebrate && e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
		const el = e.target;
		if (typing(el)) return;
		if (el instanceof HTMLElement && el.closest('[role=dialog],[role=alertdialog],[role=menu],[role=listbox],[role=slider]')) return;
		const k = keyOf(e);
		if (k === 'f' || k === 'j') {
			// A held key is one tap.
			if (!e.repeat) tap(e);
		} else if (k === 'ArrowDown') select(cur + 1);
		else if (k === 'ArrowUp') select(cur - 1);
		else if (k === 'r') restart();
		else if (k === 'l') loopOn = !loopOn;
		else if (k === 'b') bgMode = !bgMode;
		else return;
		e.preventDefault();
	}

	onMount(() => {
		if (!track) {
			mode = 'start';
			return;
		}
		const draft = unpackDraft(
			(() => {
				try {
					return localStorage.getItem(draftKey(track.videoId));
				} catch {
					return null;
				}
			})(),
			track.videoId
		);
		if (draft) {
			lines = draft.lines;
			nextId = draft.nextId;
			baseRevision = draft.baseRevision;
			improving = draft.improving;
			cur = firstTodo(draft.lines);
			restored = true;
			mode = 'edit';
		} else {
			loadStart().then(() => {
				if (mode === 'loading') mode = 'start';
			});
		}
		// Hidden to the tray is not something the page can see for itself (see `win.svelte.ts`).
		const unVisible = api.onUiVisible((v) => {
			visible = v;
			if (!v) flush();
		});
		window.addEventListener('keydown', onKey);
		window.addEventListener('pagehide', flush);
		return () => {
			unVisible.then((u) => u());
			window.removeEventListener('keydown', onKey);
			window.removeEventListener('pagehide', flush);
		};
	});
	onDestroy(() => {
		if (saveTimer !== undefined) flush();
	});

	const speedLabel = (s: number) => `${+s.toFixed(2)}x`;
	const agentLabel = (a: string) =>
		a === 'v1000' ? t('lyrics_editor.singer_all') : t('lyrics_editor.singer_n', { n: Number(a.slice(1)) });

	// --- Drawing ---------------------------------------------------------------------------------
	/** The page's own width, from a ResizeObserver: the main area shrinks with the sidebar and the
	 *  side panels, which a viewport breakpoint cannot see. Narrow, the sheet goes under the stage. */
	let pageW = $state(0);
	const wide = $derived(pageW === 0 || pageW >= 860);
	let body = $state<HTMLElement | null>(null);
	const cover = opened?.thumbnail;
	const sung = $derived(lines.filter((l) => l.words.length));
	const timedCount = $derived(sung.filter(wordsTimed).length);
	const lineDone = $derived(
		!!line &&
			(line.gap ? line.time !== undefined : wordsTimed(line) && (tapCount === 0 || tapCount > line.words.length))
	);
	/** What stands for a line in a list: its text, or a note for an instrumental break. */
	const lineLabel = (l: EdLine | undefined) => (l ? lineText(l) || (l.gap ? '♪' : '') : '');
	/** One segment per line while they fit, else one bar. */
	const SEGMENTS_MAX = 100;
	const lineNo = (id: number) => lines.findIndex((l) => l.id === id) + 1;
	let singerOpen = $state(false);
	let lineMenu = $state(false);
	let headMenu = $state(false);

	// A fixed corner: `rounded-md` follows the theme's radius, which rounds a cap this small into a
	// circle in the rounder themes, and then it no longer reads as a key.
	const KEY =
		'inline-flex h-[22px] min-w-[24px] items-center justify-center rounded-[6px] border border-b-[3px] bg-muted px-1.5 text-[11px] font-semibold text-foreground';
	/** A key cap on the primary colour. */
	const KEY_ON =
		'inline-flex h-[22px] min-w-[24px] items-center justify-center rounded-[6px] border border-b-[3px] border-primary-foreground/50 bg-primary-foreground/20 px-1.5 text-[11px] font-semibold text-primary-foreground';
	const MENU_ITEM =
		'flex w-full cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm hover:bg-muted disabled:pointer-events-none disabled:opacity-40';
	const GHOST = buttonVariants({ variant: 'ghost', size: 'sm' });
</script>

{#snippet stop(n: number, label: string, state: 'idle' | 'on' | 'done' | 'go')}
	<span class="flex shrink-0 items-center gap-1.5 font-medium {state === 'idle' ? 'text-muted-foreground' : 'text-foreground'}">
		<span
			class="flex size-5 shrink-0 items-center justify-center rounded-full text-[11px] font-bold {state === 'done'
				? 'bg-foreground text-background'
				: state === 'go'
					? 'bg-primary text-primary-foreground'
					: state === 'on'
						? 'border-2 border-foreground'
						: 'border-2 border-border'}"
		>
			{#if state === 'done'}<HugeiconsIcon icon={Tick02Icon} class="size-3" />{:else}{n}{/if}
		</span>
		<span class={wide ? '' : 'sr-only'}>{label}</span>
	</span>
{/snippet}

{#snippet glyph(l: EdLine)}
	{#if !l.words.length && !l.gap}
		<span class="size-4 shrink-0"></span>
	{:else if l.gap ? l.time !== undefined : wordsTimed(l)}
		<span class="flex size-4 shrink-0 items-center justify-center rounded-full bg-foreground text-background" title={t('lyrics_editor.line_timed')}>
			<HugeiconsIcon icon={Tick02Icon} class="size-[11px]" />
		</span>
	{:else if needsLineTime(l)}
		<span class="size-4 shrink-0 rounded-full border-2 border-muted-foreground/50" title={t('lyrics_editor.line_no_time')}></span>
	{:else}
		<span
			class="size-4 shrink-0 rounded-full border-2 border-muted-foreground bg-[linear-gradient(to_right,var(--muted-foreground)_50%,transparent_50%)]"
			title={t('lyrics_editor.line_todo')}
		></span>
	{/if}
{/snippet}

<div class="flex h-full flex-col" bind:clientWidth={pageW}>
	<!-- The song, and where the job stands. The wash is the album page's: a 96px thumbnail blurred
	     once, with its fade, in one clipped wrapper. It stays up here: nothing under the timeline. -->
	<header class="relative shrink-0 px-6 pt-4 pb-3.5">
		{#if cover}
			<div class="art-wash absolute inset-0 overflow-hidden">
				<img src={thumb(cover, 96)} alt="" class="absolute inset-0 h-full w-full scale-125 object-cover opacity-50 blur-2xl" />
				<div class="absolute inset-0 bg-gradient-to-t from-background via-background/75 to-background/40"></div>
			</div>
		{/if}
		<div class="relative flex flex-wrap items-center gap-x-3.5 gap-y-2">
			<Button variant="ghost" size="icon" class="-ml-2" aria-label={t('common.back')} title={t('common.back')} onclick={() => history.back()}>
				<HugeiconsIcon icon={ArrowLeft01Icon} class="size-[18px]" />
			</Button>
			{#if cover}
				<img src={thumb(cover, 120)} alt="" style="width:3.5rem;height:3.5rem" class="shrink-0 rounded-lg object-cover shadow-lg" draggable="false" />
			{/if}
			<div class="min-w-0 flex-1">
				<h1 class="truncate font-heading text-[1.375rem] leading-tight font-bold tracking-tight">
					{opened?.title ?? t('lyrics.sync')}
				</h1>
				{#if opened}<p class="truncate text-sm text-muted-foreground">{opened.artists}</p>{/if}
			</div>
			{#if mode === 'edit' || mode === 'preview'}
				<div class="flex items-center gap-2">
					<Button
						variant="outline"
						aria-label={mode === 'edit' ? t('lyrics_editor.preview') : t('lyrics_editor.edit')}
						onclick={() => {
							commitField();
							mode = mode === 'edit' ? 'preview' : 'edit';
						}}
					>
						<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
						<HugeiconsIcon icon={ViewIcon} altIcon={PencilEdit02Icon} showAlt={mode === 'preview'} class="size-4" />
						<span class={wide ? '' : 'sr-only'}>{mode === 'edit' ? t('lyrics_editor.preview') : t('lyrics_editor.edit')}</span>
					</Button>
					<Button
						disabled={!ready(miss)}
						aria-label={t('lyrics_editor.submit')}
						title={ready(miss) ? undefined : t('lyrics_editor.submit_not_ready')}
						onclick={openSend}
					>
						<HugeiconsIcon icon={SentIcon} class="size-4" />
						<span class={wide ? '' : 'sr-only'}>{t('lyrics_editor.submit')}</span>
					</Button>
					<Popover.Root bind:open={headMenu}>
						<Popover.Trigger class={buttonVariants({ variant: 'ghost', size: 'icon' })} aria-label={t('lyrics_editor.more')} title={t('lyrics_editor.more')}>
							<HugeiconsIcon icon={MoreHorizontalIcon} class="size-[18px]" />
						</Popover.Trigger>
						<Popover.Content align="end" class="w-52 gap-0 p-1.5">
							<button
								type="button"
								class="{MENU_ITEM} text-destructive"
								onclick={() => {
									headMenu = false;
									discardOpen = true;
								}}
							>
								<HugeiconsIcon icon={Delete02Icon} class="size-4" />
								{t('lyrics_editor.discard')}
							</button>
						</Popover.Content>
					</Popover.Root>
				</div>
			{/if}
		</div>
		{#if track && mode !== 'loading'}
			<!-- The flow: three stops, and the middle one is the song, a segment per line. -->
			<div class="relative mt-3.5 flex items-center gap-3 text-[13px]">
				{@render stop(1, t('lyrics_editor.step_text'), mode === 'start' ? 'on' : 'done')}
				{@render stop(2, t('lyrics_editor.step_timing'), mode === 'start' ? 'idle' : ready(miss) ? 'done' : 'on')}
				{#if mode === 'start'}
					<div class="h-2 min-w-16 flex-1 rounded-full bg-foreground/14"></div>
				{:else if sung.length > SEGMENTS_MAX}
					<div class="h-2 min-w-16 flex-1 overflow-hidden rounded-full bg-foreground/14">
						<div class="h-full rounded-full bg-primary" style="width:{(timedCount / sung.length) * 100}%"></div>
					</div>
				{:else}
					<div class="flex h-2 min-w-16 flex-1 gap-[3px]" aria-hidden="true">
						{#each sung as l (l.id)}
							<i
								class="min-w-0 flex-1 rounded-[3px] {wordsTimed(l)
									? 'bg-primary'
									: l.time !== undefined
										? 'bg-primary/45'
										: 'bg-foreground/14'} {l.id === line?.id && mode === 'edit' && !celebrate
									? 'outline-2 outline-offset-1 outline-foreground outline-solid'
									: ''}"
							></i>
						{/each}
					</div>
				{/if}
				{#if mode !== 'start'}
					<span class="shrink-0 font-semibold tabular-nums">
						{t('lyrics_editor.progress', { done: timedCount, total: sung.length })}
					</span>
				{/if}
				{@render stop(3, t('lyrics_editor.step_send'), mode !== 'start' && ready(miss) ? 'go' : 'idle')}
			</div>
		{/if}
	</header>

	{#if !track}
		<div class="flex flex-1 flex-col items-center justify-center gap-1.5 border-t px-6 text-center">
			<span class="mb-1 flex size-12 items-center justify-center rounded-[13px] bg-muted">
				<HugeiconsIcon icon={MusicNote01Icon} class="size-[22px]" />
			</span>
			<p class="font-heading text-base font-semibold">{t('lyrics_editor.no_song')}</p>
			<p class="max-w-sm text-[13px] leading-relaxed text-muted-foreground">{t('lyrics_editor.no_song_hint')}</p>
		</div>
	{:else if mode === 'loading'}
		<div class="flex-1 border-t px-6 py-6">
			<div class="mx-auto max-w-[760px] space-y-3">
				<Skeleton class="h-8 w-2/3 rounded-lg" />
				<Skeleton class="h-4 w-1/2 rounded" />
				<Skeleton class="mt-6 h-20 rounded-xl" />
				<div class="grid grid-cols-2 gap-3"><Skeleton class="h-32 rounded-xl" /><Skeleton class="h-32 rounded-xl" /></div>
			</div>
		</div>
	{:else if mode === 'start'}
		<div class="flex-1 overflow-y-auto border-t px-6 py-6">
			<div class="mx-auto max-w-[760px]">
				<h2 class="font-heading text-[1.625rem] leading-tight font-bold tracking-tight">{t('lyrics_editor.start_title')}</h2>
				<p class="mt-1.5 max-w-[58ch] text-[15px] text-muted-foreground">{t('lyrics_editor.start_lead')}</p>
				<!-- The whole job in three pictures. -->
				<div class="mt-5 mb-6 grid gap-2.5 {wide ? 'grid-cols-3' : ''}">
					{#each [{ title: t('lyrics_editor.how_text'), hint: t('lyrics_editor.how_text_hint'), icon: TextFontIcon }, { title: t('lyrics_editor.how_tap'), hint: t('lyrics_editor.how_tap_hint'), icon: null }, { title: t('lyrics_editor.how_send'), hint: t('lyrics_editor.how_send_hint'), icon: SentIcon }] as h (h.title)}
						<div class="flex items-center gap-2.5 text-[13px] text-muted-foreground">
							<span class="flex h-[34px] min-w-14 shrink-0 items-center justify-center gap-1 rounded-[9px] bg-muted px-2 text-foreground">
								{#if h.icon}<HugeiconsIcon icon={h.icon} class="size-[18px]" />{:else}<kbd class={KEY}>F</kbd><kbd class={KEY}>J</kbd>{/if}
							</span>
							<span><b class="block font-semibold text-foreground">{h.title}</b>{h.hint}</span>
						</div>
					{/each}
				</div>

				<div class="flex flex-wrap items-center gap-3.5 rounded-xl border bg-card p-4 {startLines.length ? 'border-primary/60' : ''}">
					<span
						class="flex size-[42px] shrink-0 items-center justify-center rounded-[11px] {startLines.length
							? 'bg-primary text-primary-foreground'
							: 'bg-muted text-muted-foreground'}"
					>
						<HugeiconsIcon icon={MagicWand01Icon} class="size-5" />
					</span>
					<div class="min-w-48 flex-1">
						{#if start?.lyrics && startLines.length}
							<p class="flex flex-wrap items-center gap-2 text-[15px] font-semibold">
								{t('lyrics_editor.start_current', { source: start.lyrics.source })}
								<span class="rounded-full px-2 py-0.5 text-[11px] font-semibold {KIND_CLASS[startKind]}">{t(KIND_LABEL[startKind])}</span>
							</p>
							<p class="mt-0.5 text-[13px] text-muted-foreground">
								{t(`lyrics_editor.start_current_${startKind}`, { count: startLines.length })}
							</p>
						{:else}
							<p class="text-[15px] font-semibold">{t('lyrics_editor.start_none')}</p>
							<p class="mt-0.5 text-[13px] text-muted-foreground">{t('lyrics_editor.start_none_hint')}</p>
						{/if}
					</div>
					{#if startLines.length}
						<Button onclick={() => begin(startLines, startKind === 'words')}>{t('lyrics_editor.start_use')}</Button>
					{/if}
				</div>

				<div class="mt-3 grid gap-3 {wide ? 'grid-cols-[1.4fr_1fr]' : ''}">
					<div class="flex flex-col gap-2.5 rounded-xl border bg-card p-4">
						<div class="flex items-center gap-3">
							<span class="flex size-[42px] shrink-0 items-center justify-center rounded-[11px] bg-muted">
								<HugeiconsIcon icon={ClipboardIcon} class="size-5" />
							</span>
							<div class="min-w-0 flex-1">
								<p class="text-[15px] font-semibold">{t('lyrics_editor.start_paste')}</p>
								<p class="mt-0.5 text-[13px] text-muted-foreground">{t('lyrics_editor.start_paste_hint')}</p>
							</div>
						</div>
						<Textarea
							bind:value={pasted}
							class="max-h-56 min-h-[74px] overflow-y-auto"
							placeholder={t('lyrics_editor.start_paste_placeholder')}
							aria-label={t('lyrics_editor.start_paste')}
						/>
						{#if pasted.trim()}
							<div><Button onclick={() => importText(pasted)}>{t('lyrics_editor.start_paste_go')}</Button></div>
						{/if}
					</div>
					<div class="flex flex-col gap-2.5 rounded-xl border bg-card p-4">
						<div class="flex items-center gap-3">
							<span class="flex size-[42px] shrink-0 items-center justify-center rounded-[11px] bg-muted">
								<HugeiconsIcon icon={FileImportIcon} class="size-5" />
							</span>
							<div class="min-w-0 flex-1">
								<p class="text-[15px] font-semibold">{t('lyrics_editor.start_file')}</p>
								<p class="mt-0.5 text-[13px] text-muted-foreground">{t('lyrics_editor.start_file_hint')}</p>
							</div>
						</div>
						<input bind:this={fileInput} type="file" accept=".ttml,.xml,.lrc,.elrc,.txt,.json" class="hidden" onchange={pickFile} />
						<div class="mt-auto"><Button variant="outline" onclick={() => fileInput?.click()}>{t('lyrics_editor.start_file_go')}</Button></div>
					</div>
				</div>
			</div>
		</div>
	{:else if mode === 'preview'}
		<p class="flex items-center gap-2.5 border-t px-6 py-2.5 text-[13px] text-muted-foreground">
			<HugeiconsIcon icon={ViewIcon} class="size-4 shrink-0" />
			{t('lyrics_editor.preview_note')}
		</p>
		<div class="flex min-h-0 flex-1 flex-col overflow-hidden border-t pt-2">
			<LyricsView expanded preview={previewLyrics} />
		</div>
	{:else}
		<div bind:this={body} class="flex min-h-0 flex-1 border-t {wide ? '' : 'flex-col overflow-y-auto'}">
			<section class="flex min-w-0 flex-1 flex-col {wide ? 'overflow-y-auto' : 'shrink-0'}">
				<!-- What stands in the way, said once each, with the one button that helps. -->
				{#if restored || saveFailed || blocked || !onTrack || miss.tooMany || miss.tooLong.length}
					<div class="space-y-2 px-6 pt-4">
						{#if saveFailed}
							<LimusicNotice icon={Alert02Icon} tone="error" title={t('lyrics_editor.save_failed_title')} body={t('lyrics_editor.save_failed')} />
						{/if}
						{#if blocked === 'room'}
							<LimusicNotice icon={UserMultiple02Icon} title={t('lyrics_editor.blocked_room_title')} body={t('lyrics_editor.blocked_room')} />
						{:else if blocked === 'remote'}
							<LimusicNotice icon={ComputerIcon} title={t('lyrics_editor.blocked_remote_title')} body={t('lyrics_editor.blocked_remote')} />
						{:else if !onTrack}
							<LimusicNotice title={t('lyrics_editor.other_song_title', { title: track.title })} body={t('lyrics_editor.other_song')}>
								{#snippet lead()}
									{#if cover}
										<img src={thumb(cover, 120)} alt="" class="size-8 shrink-0 rounded-md object-cover" draggable="false" />
									{/if}
								{/snippet}
								{#snippet actions()}
									<Button size="sm" onclick={() => playAt(track.videoId, (range?.start ?? 0) / 1000, track.title)}>
										{t('lyrics_editor.other_song_play')}
									</Button>
								{/snippet}
							</LimusicNotice>
						{/if}
						{#if miss.tooMany}
							<LimusicNotice icon={Alert02Icon} tone="error" title={t('lyrics_editor.too_many_title')} body={t('lyrics_editor.too_many')} />
						{:else if miss.tooLong.length}
							{@const n = lineNo(miss.tooLong[0])}
							<LimusicNotice icon={ScissorIcon} tone="error" title={t('lyrics_editor.too_long_title', { n })} body={t('lyrics_editor.too_long')}>
								{#snippet actions()}
									<Button size="sm" variant="outline" onclick={() => select(n - 1)}>{t('lyrics_editor.too_long_go', { n })}</Button>
								{/snippet}
							</LimusicNotice>
						{/if}
						{#if restored}
							<LimusicNotice
								icon={PencilEdit02Icon}
								title={t('lyrics_editor.restored_title')}
								body={t('lyrics_editor.restored_body', { done: timedCount, total: sung.length })}
							>
								{#snippet actions()}
									<Button size="sm" variant="ghost" class="text-muted-foreground" onclick={() => (discardOpen = true)}>
										{t('lyrics_editor.start_over')}
									</Button>
									<Button size="sm" variant="outline" onclick={() => (restored = false)}>{t('lyrics_editor.restored_ok')}</Button>
								{/snippet}
							</LimusicNotice>
						{/if}
					</div>
				{/if}

				{#if celebrate}
					<!-- The moment the song is ready. -->
					<div class="flex flex-1 flex-col items-center justify-center px-6 py-10 text-center">
						<span class="mb-4 flex size-[68px] items-center justify-center rounded-full bg-primary text-primary-foreground">
							<HugeiconsIcon icon={Tick02Icon} class="size-[34px]" />
						</span>
						<h2 class="font-heading text-3xl leading-tight font-bold tracking-tight">{t('lyrics_editor.ready_title')}</h2>
						<p class="mt-2 mb-5 max-w-[44ch] text-[15px] text-muted-foreground">{t('lyrics_editor.ready_body')}</p>
						<div class="flex flex-wrap justify-center gap-2.5">
							<Button size="lg" variant="outline" onclick={() => (mode = 'preview')}>
								<HugeiconsIcon icon={ViewIcon} class="size-4" />
								{t('lyrics_editor.ready_preview')}
							</Button>
							<Button size="lg" onclick={openSend}>
								<HugeiconsIcon icon={SentIcon} class="size-4" />
								{t('lyrics_editor.submit')}
							</Button>
						</div>
					</div>
				{:else}
					<div class="flex flex-1 flex-col px-6 pt-4 {blocked || !onTrack ? 'opacity-60' : ''}">
						<!-- Which line, and the rare jobs: its text, its singer, background vocals, the rest. -->
						<div class="flex flex-wrap items-center gap-x-1 gap-y-1.5">
							<Button variant="ghost" size="icon-sm" class="-ml-2 text-muted-foreground" disabled={cur === 0} onclick={() => select(cur - 1)} aria-label={t('lyrics_editor.prev_line')} title={t('lyrics_editor.prev_line')}>
								<HugeiconsIcon icon={ArrowUp01Icon} class="size-4" />
							</Button>
							<Button variant="ghost" size="icon-sm" class="text-muted-foreground" disabled={cur >= lines.length - 1} onclick={() => select(cur + 1)} aria-label={t('lyrics_editor.next_line')} title={t('lyrics_editor.next_line')}>
								<HugeiconsIcon icon={ArrowDown01Icon} class="size-4" />
							</Button>
							<span class="mx-1.5 text-sm font-semibold tabular-nums">{t('lyrics_editor.line_of', { n: cur + 1, total: lines.length })}</span>
							{#if line && (line.gap ? line.time !== undefined : wordsTimed(line))}
								<span class="flex h-5 items-center gap-1 rounded-full bg-primary/16 px-2 text-[11px] font-semibold">
									<HugeiconsIcon icon={Tick02Icon} class="size-3" />
									{t('lyrics_editor.line_timed')}
								</span>
							{/if}
							<span class="flex-1"></span>
							<Button variant="ghost" size="sm" class={textOpen ? 'bg-muted text-foreground' : 'text-muted-foreground'} aria-label={t('lyrics_editor.edit_text')} onclick={openText}>
								<HugeiconsIcon icon={PencilEdit02Icon} class="size-4" />
								<span class={wide ? '' : 'sr-only'}>{t('lyrics_editor.edit_text')}</span>
							</Button>
							<Popover.Root bind:open={singerOpen}>
								<Popover.Trigger class="{GHOST} {line?.agent ? 'text-foreground' : 'text-muted-foreground'}" aria-label={t('lyrics_editor.singer')} disabled={!line}>
									<HugeiconsIcon icon={Mic01Icon} class="size-4" />
									<span class={wide ? '' : 'sr-only'}>{line?.agent ? agentLabel(line.agent) : t('lyrics_editor.singer')}</span>
								</Popover.Trigger>
								<Popover.Content align="end" class="w-48 gap-0 p-1.5">
									<div role="radiogroup" aria-label={t('lyrics_editor.singer')}>
										{#each [undefined, ...agents, 'v1000'] as a (a ?? '')}
											{@const on = (line?.agent ?? undefined) === a}
											<button
												type="button"
												role="radio"
												aria-checked={on}
												class="{MENU_ITEM} {on ? 'bg-muted font-semibold' : ''}"
												onclick={() => {
													if (line) updateLine(line.id, (l) => setAgent(l, a));
													singerOpen = false;
												}}
											>
												<span class="flex size-4 items-center justify-center">
													{#if on}<HugeiconsIcon icon={Tick02Icon} class="size-3.5" />{/if}
												</span>
												{a ? agentLabel(a) : t('lyrics_editor.singer_none')}
											</button>
										{/each}
									</div>
								</Popover.Content>
							</Popover.Root>
							<Button
								variant={bgMode ? 'default' : 'ghost'}
								size="sm"
								class={bgMode ? '' : 'text-muted-foreground'}
								aria-pressed={bgMode}
								aria-label={t('lyrics_editor.bg_long')}
								title={t('lyrics_editor.bg_long')}
								onclick={() => (bgMode = !bgMode)}
							>
								<HugeiconsIcon icon={UserGroupIcon} class="size-4" />
								<span class={wide ? '' : 'sr-only'}>{t('lyrics_editor.bg')}</span>
								{#if bgMode}<kbd class={KEY_ON}>B</kbd>{/if}
							</Button>
							<Popover.Root bind:open={lineMenu}>
								<Popover.Trigger class="{buttonVariants({ variant: 'ghost', size: 'icon-sm' })} text-muted-foreground" aria-label={t('lyrics_editor.line_more')} title={t('lyrics_editor.line_more')}>
									<HugeiconsIcon icon={MoreHorizontalIcon} class="size-4" />
								</Popover.Trigger>
								<Popover.Content align="end" class="w-60 gap-0 p-1.5">
									<button type="button" class={MENU_ITEM} disabled={cur >= lines.length - 1} onclick={() => ((lineMenu = false), joinNext())}>
										<HugeiconsIcon icon={MergeIcon} class="size-4" />{t('lyrics_editor.join')}
									</button>
									<button type="button" class={MENU_ITEM} onclick={() => ((lineMenu = false), addBelow())}>
										<HugeiconsIcon icon={Add01Icon} class="size-4" />{t('lyrics_editor.add_line')}
									</button>
									<!-- Not after a break, or before one: two in a row are one. -->
									<button type="button" class={MENU_ITEM} disabled={!line || !!line.gap || !!lines[cur + 1]?.gap} onclick={() => ((lineMenu = false), addGap())}>
										<HugeiconsIcon icon={MusicNote01Icon} class="size-4" />{t('lyrics_editor.add_gap')}
									</button>
									<button type="button" class={MENU_ITEM} onclick={() => ((lineMenu = false), resplit())}>
										<HugeiconsIcon icon={MagicWand01Icon} class="size-4" />{t('lyrics_editor.resplit')}
									</button>
									<div class="my-1 h-px bg-border"></div>
									<button type="button" class="{MENU_ITEM} text-destructive" disabled={lines.length < 2} onclick={() => ((lineMenu = false), removeLine())}>
										<HugeiconsIcon icon={Delete02Icon} class="size-4" />{t('lyrics_editor.delete_line')}
									</button>
								</Popover.Content>
							</Popover.Root>
						</div>

						<!-- The line before, the line, the line after: the song around what is being timed. -->
						<p class="mt-3.5 mb-2 min-h-[1.3em] truncate font-heading text-base leading-snug font-semibold text-muted-foreground/60">
							{lineLabel(lines[cur - 1])}
						</p>
						{#snippet textEditor()}
							<div class="max-w-[760px]">
								<input
									bind:this={fieldEl}
									bind:value={field}
									class="w-full border-0 border-b-2 border-primary bg-transparent pt-0.5 pb-1.5 font-heading leading-snug font-bold outline-none {wide ? 'text-3xl' : 'text-2xl'}"
									placeholder={t('lyrics_editor.text_placeholder')}
									aria-label={t('lyrics_editor.text')}
									onfocus={() => (editing = true)}
									onblur={() => {
										editing = false;
										commitField();
										textOpen = false;
									}}
									onkeydown={(e) => {
										if (e.key === 'Enter') e.currentTarget.blur();
										else if (e.key === 'Escape') {
											field = line ? fieldText(line.words) : '';
											e.currentTarget.blur();
										}
									}}
								/>
								<p class="mt-2.5 text-[13px] text-muted-foreground">{t('lyrics_editor.text_hint')}</p>
								<!-- mousedown is swallowed so the field keeps its focus and its caret. -->
								<div class="mt-3 flex flex-wrap gap-2">
									<Button size="sm" onmousedown={(e) => e.preventDefault()} onclick={() => fieldEl?.blur()}>
										{t('lyrics_editor.text_done')} <kbd class={KEY_ON}>Enter</kbd>
									</Button>
									<Button size="sm" variant="outline" onmousedown={(e) => e.preventDefault()} onclick={splitHere}>
										<HugeiconsIcon icon={ScissorIcon} class="size-4" />
										{t('lyrics_editor.split')}
									</Button>
								</div>
							</div>
						{/snippet}
						{#snippet nextLine()}
							<p class="mt-2 min-h-[1.3em] truncate font-heading text-base leading-snug font-semibold text-muted-foreground/60">
								{lineLabel(lines[cur + 1])}
							</p>
						{/snippet}
						<!-- An instrumental break has no words to show: it says what it is, and what listeners
						     get. "Edit text" still opens the field, and text makes it a line. -->
						{#snippet gapStage()}
							<div class="flex max-w-[760px] items-center gap-3.5">
								<span class="flex size-12 shrink-0 items-center justify-center rounded-[13px] bg-muted">
									<HugeiconsIcon icon={MusicNote01Icon} class="size-6" />
								</span>
								<div class="min-w-0">
									<p class="font-heading leading-snug font-bold {wide ? 'text-3xl' : 'text-2xl'}">{t('lyrics_editor.gap')}</p>
									<p class="text-[13px] text-muted-foreground">{t('lyrics_editor.gap_hint')}</p>
								</div>
							</div>
						{/snippet}
						{#if line}
							<!-- Keyed on the line: a new line starts with no edge held and its track unscrolled. -->
							{#key line.id}
							<LyricTimeline
								words={line.words}
								range={owesTime ? null : range}
								{durationMs}
								running={active && !playback.paused}
								{position}
								nextTap={active && !owesTime && !lineDone && tapCount < line.words.length ? tapCount : -1}
								tapped={owesTime ? 0 : tapCount}
								{marks}
								{bgMode}
								compact={!wide}
								editor={textOpen ? textEditor : line.gap ? gapStage : !line.words.length ? textEditor : undefined}
								gap={!!line.gap}
								between={nextLine}
								onwords={(words: EdWord[]) => updateLine(line.id, (l) => withWords(l, words))}
								onbg={(k: number) => updateLine(line.id, (l) => toggleBg(l, k))}
								onseek={(ms: number) => active && jump(ms)}
							/>
							{/key}
						{/if}

						<!-- The tap. Positions never change, so a hand on the mouse never lands on the wrong
						     button: only which of the two is filled does. -->
						<div class="mt-3.5 flex flex-wrap items-center gap-x-3 gap-y-2.5">
							<!-- pointerdown, not click: a click fires on release, a whole key-travel late. A
							     click with no pointer behind it is the keyboard pressing the focused button. -->
							<Button
								variant={lineDone ? 'outline' : 'default'}
								class="h-[52px] gap-2.5 px-5 font-heading text-[17px] font-bold {wide ? 'min-w-48' : 'flex-1'}"
								disabled={!active || !(line?.words.length || line?.gap)}
								onpointerdown={(e) => e.button === 0 && tap(e)}
								onclick={(e) => e.detail === 0 && tap(e)}
							>
								<HugeiconsIcon icon={Tap01Icon} class="size-5" />
								{lineDone ? t('lyrics_editor.tap_again') : t('lyrics_editor.tap')}
								<span class="flex gap-1"><kbd class={lineDone ? KEY : KEY_ON}>F</kbd><kbd class={lineDone ? KEY : KEY_ON}>J</kbd></span>
							</Button>
							<Button variant="outline" disabled={!active || !range} onclick={restart} title={t('lyrics_editor.restart')}>
								<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="size-4" />
								{t('lyrics_editor.again')} <kbd class={KEY}>R</kbd>
							</Button>
							<p class="min-w-44 flex-1 text-sm">
								{#if active && playback.paused}
									{t('lyrics_editor.say_paused')}<span class="block text-[13px] text-muted-foreground">{t('lyrics_editor.say_paused_sub')}</span>
								{:else if line?.gap}
									{#if owesTime}
										{t('lyrics_editor.say_gap_todo')}
									{:else}
										{t('lyrics_editor.say_gap')}<span class="block text-[13px] text-muted-foreground">{t('lyrics_editor.say_gap_sub')}</span>
									{/if}
								{:else if !line?.words.length}
									{t('lyrics_editor.empty_line')}
								{:else if owesTime}
									{t('lyrics_editor.say_line')}<span class="block text-[13px] text-muted-foreground">{t('lyrics_editor.say_line_sub')}</span>
								{:else if lineDone}
									{t('lyrics_editor.say_timed')}<span class="block text-[13px] text-muted-foreground">{t('lyrics_editor.say_timed_sub')}</span>
								{:else if line && tapCount === line.words.length}
									{t('lyrics_editor.say_last')}<span class="block text-[13px] text-muted-foreground">{t('lyrics_editor.say_last_sub')}</span>
								{:else}
									{t('lyrics_editor.say_words')}<span class="block text-[13px] text-muted-foreground">{t('lyrics_editor.say_words_sub')}</span>
								{/if}
							</p>
							<Button variant={lineDone ? 'default' : 'outline'} disabled={cur >= lines.length - 1} onclick={() => select(cur + 1)}>
								{t('lyrics_editor.next_line')}
								<!-- An icon, not the font's arrow: Oxanium draws that one a few pixels tall. -->
								<kbd class={lineDone ? KEY_ON : KEY}><HugeiconsIcon icon={ArrowDown02Icon} strokeWidth={2.25} class="size-3.5" /></kbd>
							</Button>
						</div>

						<!-- How the song plays while timing: quiet, at the bottom, set once. -->
						<div class="min-h-4 flex-1"></div>
						<div class="-mx-6 flex flex-wrap items-center gap-x-6 gap-y-2 border-t px-6 pt-2.5 pb-3 text-[13px] text-muted-foreground">
							<div class="flex items-center gap-2.5">
								<span>{t('lyrics_editor.speed')}</span>
								<Slider
									type="single"
									class="w-28"
									min={0.5}
									max={1}
									step={0.05}
									value={speedDraft ?? editSpeed}
									onValueChange={(v) => (speedDraft = v)}
									onValueCommit={commitSpeed}
									aria-label={t('lyrics_editor.speed')}
								/>
								<span class="w-10 font-semibold text-foreground tabular-nums">{speedLabel(speedDraft ?? editSpeed)}</span>
							</div>
							<label class="flex cursor-pointer items-center gap-2 select-none" title={t('lyrics_editor.loop_hint')}>
								<Switch bind:checked={loopOn} />
								{t('lyrics_editor.loop')} <kbd class={KEY}>L</kbd>
							</label>
							<!-- A small value, so a field in the sentence rather than a boxed input. -->
							<label class="flex items-center gap-2" title={t('lyrics_editor.tap_offset_hint')}>
								{t('lyrics_editor.tap_offset')}
								<input
									class="w-12 border-b border-border bg-transparent px-0.5 py-0.5 text-right text-foreground tabular-nums outline-none focus:border-primary"
									inputmode="numeric"
									bind:value={offsetText}
									onblur={commitOffset}
									onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
								/>
								{t('lyrics_editor.ms_unit')}
							</label>
						</div>
					</div>
				{/if}
			</section>

			<!-- The sheet: the whole song, and what is left of it. -->
			<aside class="flex min-h-0 shrink-0 flex-col bg-card/60 {wide ? 'w-[300px] border-l' : 'border-t'}">
				<div class="flex items-center gap-2 px-3.5 pt-3 pb-2">
					<h2 class="font-heading text-sm font-semibold">{t('lyrics_editor.lines')}</h2>
					<span class="min-w-0 flex-1 truncate text-[13px] text-muted-foreground">
						{#if miss.untimed.length}
							{t('lyrics_editor.to_go', { count: miss.untimed.length })}
						{:else if !miss.empty}
							{t('lyrics_editor.all_timed', { count: sung.length })}
						{/if}
					</span>
					{#if miss.untimed.length}
						<Button size="sm" variant="outline" onclick={nextTodo}>{t('lyrics_editor.todo_next')}</Button>
					{/if}
				</div>
				<!-- No transitions on the rows (docs/UI-PERFORMANCE.md): a long song is a few hundred. -->
				<div bind:this={list} class="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2 {wide ? '' : 'max-h-72'}">
					{#each lines as l, i (l.id)}
						<button
							type="button"
							data-line={i}
							aria-current={i === cur}
							onclick={() => select(i)}
							class="relative flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2 py-[7px] text-left text-[13.5px] {i === cur
								? 'bg-primary/16 font-semibold before:absolute before:inset-y-[7px] before:left-0 before:w-[3px] before:rounded-full before:bg-primary'
								: 'hover:bg-muted/60'}"
						>
							<span class="w-5 shrink-0 text-right text-[11.5px] font-normal text-muted-foreground tabular-nums">{i + 1}</span>
							{#if l.gap}
								<span class="flex min-w-0 flex-1 items-center gap-1.5 text-muted-foreground">
									<HugeiconsIcon icon={MusicNote01Icon} class="size-3.5 shrink-0" />
									<span class="truncate">{t('lyrics_editor.gap')}</span>
								</span>
							{:else}
								<span class="min-w-0 flex-1 truncate {i === cur || wordsTimed(l) ? '' : 'text-muted-foreground'}">
									{lineText(l) || '♪'}
								</span>
							{/if}
							{#if l.agent}
								<span class="shrink-0 rounded-full border px-1.5 text-[10.5px] font-semibold text-muted-foreground">{agentLabel(l.agent)}</span>
							{/if}
							{@render glyph(l)}
						</button>
					{/each}
				</div>
				<div class="flex flex-wrap gap-x-3 gap-y-1 border-t px-3.5 py-2 text-[11.5px] text-muted-foreground">
					<span class="flex items-center gap-1.5"><span class="size-3.5 rounded-full bg-foreground"></span>{t('lyrics_editor.line_timed')}</span>
					<span class="flex items-center gap-1.5"><span class="size-3.5 rounded-full border-2 border-muted-foreground bg-[linear-gradient(to_right,var(--muted-foreground)_50%,transparent_50%)]"></span>{t('lyrics_editor.line_todo')}</span>
					<span class="flex items-center gap-1.5"><span class="size-3.5 rounded-full border-2 border-muted-foreground/50"></span>{t('lyrics_editor.line_no_time')}</span>
				</div>
			</aside>
		</div>
	{/if}
</div>

<!-- Sending: who it goes out as, what happens next, and the service's answer in the same place. -->
<Dialog.Root bind:open={sendOpen} onOpenChange={(o) => !o && sendClosed()}>
	<Dialog.Content class="sm:max-w-md" showCloseButton={false}>
		{#if result}
			<Dialog.Header>
				<div class="flex items-center gap-3.5 text-left">
					<span
						class="flex size-12 shrink-0 items-center justify-center rounded-full {result.kind === 'success'
							? 'bg-primary text-primary-foreground'
							: result.kind === 'error'
								? 'bg-destructive/15 text-destructive'
								: 'bg-muted'}"
					>
						<!-- One icon per answer: the block is keyed on the answer, so `icon` never has to change. -->
						{#if result.kind === 'success'}
							<HugeiconsIcon icon={Tick02Icon} class="size-6" />
						{:else if result.kind === 'error'}
							<HugeiconsIcon icon={Alert02Icon} class="size-6" />
						{:else}
							<HugeiconsIcon icon={InformationCircleIcon} class="size-6" />
						{/if}
					</span>
					<div class="min-w-0">
						<Dialog.Title class="font-heading text-lg leading-tight font-bold">{t(result.title)}</Dialog.Title>
						{#if result.sent && track}<p class="truncate text-sm text-muted-foreground">{track.title}</p>{/if}
					</div>
				</div>
				<Dialog.Description class="pt-1 text-left text-[13.5px] leading-relaxed">{t(result.key, result.params)}</Dialog.Description>
			</Dialog.Header>
			<Dialog.Footer>
				{#if result.sent}
					<Button
						onclick={() => {
							sendOpen = false;
							sendClosed();
						}}
					>
						{t('lyrics_editor.done')}
					</Button>
				{:else if result.stale}
					<Button
						variant="outline"
						onclick={() => {
							sendOpen = false;
							discardOpen = true;
						}}
					>
						{t('lyrics_editor.stale_load')}
					</Button>
					<Button disabled={submitting} onclick={keepMine}>{t('lyrics_editor.stale_keep')}</Button>
				{:else}
					<Button variant="ghost" onclick={() => (sendOpen = false)}>{t('common.close')}</Button>
					{#if result.key === 'lyrics_editor.err_unavailable' || result.key === 'lyrics_editor.err_signed_out'}
						<Button disabled={submitting} onclick={submit}>{t('common.retry')}</Button>
					{/if}
				{/if}
			</Dialog.Footer>
		{:else if track}
			<Dialog.Header>
				<div class="flex items-center gap-3.5 text-left">
					{#if cover}
						<img src={thumb(cover, 120)} alt="" class="size-12 shrink-0 rounded-lg object-cover" draggable="false" />
					{/if}
					<div class="min-w-0">
						<Dialog.Title class="font-heading text-lg leading-tight font-bold">{t('lyrics_editor.send_title')}</Dialog.Title>
						<p class="truncate text-sm text-muted-foreground">{track.title}</p>
					</div>
				</div>
			</Dialog.Header>
			<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[13.5px]">
				<dt class="text-muted-foreground">{t('lyrics_editor.send_lines')}</dt>
				<dd>{t('lyrics_editor.send_lines_value', { count: sung.length })}</dd>
				<dt class="text-muted-foreground">{t('lyrics_editor.send_as')}</dt>
				<dd>{limusic.signedIn && !limusic.anonymous && limusic.handle ? limusic.handle : t('lyrics_editor.send_as_anonymous')}</dd>
				<dt class="text-muted-foreground">{t('lyrics_editor.send_then')}</dt>
				<dd>
					{limusic.signedIn && !limusic.anonymous && limusic.account?.trusted
						? t('lyrics_editor.send_then_live')
						: t('lyrics_editor.send_then_review')}
				</dd>
			</dl>
			<Dialog.Footer>
				<Button variant="ghost" disabled={submitting} onclick={() => (sendOpen = false)}>{t('common.cancel')}</Button>
				<Button disabled={submitting} onclick={submit}>
					<HugeiconsIcon icon={SentIcon} altIcon={Loading03Icon} showAlt={submitting} class="size-4 {submitting ? 'animate-spin' : ''}" />
					{submitting ? t('lyrics_editor.sending') : t('lyrics_editor.submit')}
				</Button>
			</Dialog.Footer>
		{/if}
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root bind:open={discardOpen}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('lyrics_editor.discard_title')}</AlertDialog.Title>
			<AlertDialog.Description>{t('lyrics_editor.discard_body')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action onclick={discard}>{t('lyrics_editor.discard')}</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
