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
		ArrowLeft01Icon,
		ArrowTurnBackwardIcon,
		ArrowUp01Icon,
		ClipboardIcon,
		Delete02Icon,
		FileImportIcon,
		MagicWand01Icon,
		MergeIcon,
		Mic01Icon,
		MusicNote01Icon,
		PencilEdit02Icon,
		ScissorIcon,
		SentIcon,
		Tap01Icon,
		Tick02Icon,
		ViewIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Slider } from '$lib/components/ui/slider';
	import { Switch } from '$lib/components/ui/switch';
	import { Textarea } from '$lib/components/ui/textarea';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import LyricTimeline from '$lib/components/LyricTimeline.svelte';
	import LyricsView from '$lib/components/LyricsView.svelte';
	import * as api from '$lib/api';
	import { isLocalId } from '$lib/api';
	import { np, playAt, playback, seekTo, toast, ui } from '$lib/player.svelte';
	import { inRoom } from '$lib/lt.svelte';
	import { link } from '$lib/link.svelte';
	import { submitLyrics } from '$lib/limusic.svelte';
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
		joinLines,
		kindOf,
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

	function flush() {
		clearTimeout(saveTimer);
		saveTimer = undefined;
		if (!track || !lines.length) return;
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
			baseRevision = null;
		} catch {
			start = { lyrics: null };
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
		clearTimeout(saveTimer);
		if (track) store(draftKey(track.videoId), null);
		lines = [];
		saveFailed = false;
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
		mode === 'edit' && onTrack && !blocked && visible && !np.open && !ui.theaterOpen
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
	function resetTake() {
		takeGen++;
		taps = [];
		tapCount = 0;
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
		if (!active || !line || playback.paused) return;
		// First thing, before anything can delay it: when the key actually went down.
		const wall = eventWallMs(Date.now(), performance.now(), e.timeStamp);
		const id = line.id;
		if (owesTime) {
			const at = cur;
			// Through an untimed lyric the cursor walks on with the song.
			if (lines[at + 1] && needsLineTime(lines[at + 1])) cur = at + 1;
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
		const end = range ? range.end : undefined;
		updateLine(id, (l) => {
			const words = wordsFromTaps(l.words, taps, l.end ?? end);
			return words ? withWords(l, words) : l;
		});
	}

	// --- Text, singer, background ---------------------------------------------------------------
	let field = $state('');
	let fieldEl = $state<HTMLInputElement | null>(null);
	let editing = $state(false);
	let bgMode = $state(false);
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

	// --- Submitting -----------------------------------------------------------------------------
	let submitting = $state(false);
	async function submit() {
		if (!track || !ready(miss) || submitting) return;
		commitField();
		flush();
		submitting = true;
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
			if (outcome.kind === 'unavailable') toast(t('lyrics_editor.submit_unavailable'));
		} finally {
			submitting = false;
		}
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
	const CARD = 'rounded-xl border bg-card';
	const LABEL = 'text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const KEY = 'rounded-md border bg-muted px-1.5 py-0.5 font-mono text-[11px] font-medium text-foreground';
	const TOOL =
		'flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-40';
</script>

<div class="mx-auto flex h-full max-w-5xl flex-col gap-4 p-6">
	<header class="flex flex-wrap items-center gap-x-3 gap-y-2">
		<Button variant="ghost" size="sm" onclick={() => history.back()}>
			<HugeiconsIcon icon={ArrowLeft01Icon} class="h-4 w-4" />
			{t('common.back')}
		</Button>
		<div class="min-w-0 flex-1">
			<h1 class="truncate font-heading text-lg font-semibold">
				{improving ? t('lyrics_editor.title_improve') : t('lyrics_editor.title_sync')}
			</h1>
			{#if opened}
				<p class="truncate text-sm text-muted-foreground">{opened.title} · {opened.artists}</p>
			{/if}
		</div>
		{#if mode === 'edit' || mode === 'preview'}
			<Button variant="ghost" size="sm" class="text-muted-foreground" onclick={() => (discardOpen = true)}>
				<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
				{t('lyrics_editor.discard')}
			</Button>
			<Button
				variant="outline"
				size="sm"
				onclick={() => {
					commitField();
					mode = mode === 'edit' ? 'preview' : 'edit';
				}}
			>
				<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
				<HugeiconsIcon icon={ViewIcon} altIcon={PencilEdit02Icon} showAlt={mode === 'preview'} class="h-4 w-4" />
				{mode === 'edit' ? t('lyrics_editor.preview') : t('lyrics_editor.edit')}
			</Button>
			<Button
				size="sm"
				disabled={!ready(miss) || submitting}
				title={ready(miss) ? undefined : t('lyrics_editor.submit_not_ready')}
				onclick={submit}
			>
				<HugeiconsIcon icon={SentIcon} class="h-4 w-4" />
				{t('lyrics_editor.submit')}
			</Button>
		{/if}
	</header>

	{#if !track}
		<div class="{CARD} flex flex-col items-center gap-2 px-6 py-12 text-center">
			<HugeiconsIcon icon={MusicNote01Icon} class="h-8 w-8 text-muted-foreground" />
			<p class="text-sm font-medium">{t('lyrics_editor.no_song')}</p>
			<p class="max-w-md text-xs leading-relaxed text-muted-foreground">{t('lyrics_editor.no_song_hint')}</p>
		</div>
	{:else if mode === 'loading'}
		<div class="h-40 animate-pulse rounded-xl border bg-card/40"></div>
	{:else if mode === 'start'}
		<section>
			<h2 class="{LABEL} mb-2 px-1">{t('lyrics_editor.start_title')}</h2>
			<div class="{CARD} divide-y divide-border/60 overflow-hidden">
				<div class="flex items-center gap-4 px-4 py-3.5">
					<span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/12 text-primary">
						<HugeiconsIcon icon={MagicWand01Icon} class="h-4 w-4" />
					</span>
					<div class="min-w-0 flex-1">
						{#if start?.lyrics && startLines.length}
							<p class="flex items-center gap-2 text-sm font-medium">
								<span class="truncate">{t('lyrics_editor.start_current', { source: start.lyrics.source })}</span>
								<span class="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide {KIND_CLASS[startKind]}">
									{t(KIND_LABEL[startKind])}
								</span>
							</p>
							<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">
								{t(`lyrics_editor.start_current_${startKind}`)}
							</p>
						{:else}
							<p class="text-sm font-medium">{t('lyrics_editor.start_none')}</p>
							<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t('lyrics_editor.start_none_hint')}</p>
						{/if}
					</div>
					<Button
						size="sm"
						disabled={!startLines.length}
						onclick={() => begin(startLines, startKind === 'words')}
					>
						{t('lyrics_editor.start_use')}
					</Button>
				</div>
				<div class="px-4 py-3.5">
					<div class="flex items-center gap-4">
						<span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
							<HugeiconsIcon icon={ClipboardIcon} class="h-4 w-4" />
						</span>
						<div class="min-w-0 flex-1">
							<p class="text-sm font-medium">{t('lyrics_editor.start_paste')}</p>
							<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t('lyrics_editor.start_paste_hint')}</p>
						</div>
						<Button size="sm" variant="outline" disabled={!pasted.trim()} onclick={() => importText(pasted)}>
							{t('lyrics_editor.start_use')}
						</Button>
					</div>
					<Textarea
						bind:value={pasted}
						class="mt-3 max-h-56 min-h-24 overflow-y-auto"
						placeholder={t('lyrics_editor.start_paste_placeholder')}
						aria-label={t('lyrics_editor.start_paste')}
					/>
				</div>
				<div class="flex items-center gap-4 px-4 py-3.5">
					<span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
						<HugeiconsIcon icon={FileImportIcon} class="h-4 w-4" />
					</span>
					<div class="min-w-0 flex-1">
						<p class="text-sm font-medium">{t('lyrics_editor.start_file')}</p>
						<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t('lyrics_editor.start_file_hint')}</p>
					</div>
					<input
						bind:this={fileInput}
						type="file"
						accept=".ttml,.xml,.lrc,.elrc,.txt,.json"
						class="hidden"
						onchange={pickFile}
					/>
					<Button size="sm" variant="outline" onclick={() => fileInput?.click()}>
						{t('lyrics_editor.start_file_go')}
					</Button>
				</div>
			</div>
		</section>
	{:else}
		{#if blocked || !onTrack || saveFailed}
			<div class="{CARD} flex items-center gap-3 px-4 py-3 text-sm" role="status">
				<HugeiconsIcon icon={Alert02Icon} class="h-4 w-4 shrink-0 text-muted-foreground" />
				<p class="min-w-0 flex-1 leading-relaxed">
					{#if saveFailed}
						{t('lyrics_editor.save_failed')}
					{:else if blocked === 'room'}
						{t('lyrics_editor.blocked_room')}
					{:else if blocked === 'remote'}
						{t('lyrics_editor.blocked_remote')}
					{:else}
						{t('lyrics_editor.other_song', { title: track.title })}
					{/if}
				</p>
				{#if !saveFailed && !blocked && !onTrack}
					<Button
						size="sm"
						variant="outline"
						onclick={() => playAt(track.videoId, (range?.start ?? 0) / 1000, track.title)}
					>
						{t('lyrics_editor.other_song_play')}
					</Button>
				{/if}
			</div>
		{/if}

		{#if mode === 'preview'}
			<div class="{CARD} flex min-h-0 flex-1 flex-col overflow-hidden pt-2">
				<LyricsView expanded preview={previewLyrics} />
			</div>
		{:else}
			<section class="{CARD} shrink-0">
				<!-- Where we are, and the editing tools: tempo, loop, tap offset. -->
				<div class="flex flex-wrap items-center gap-x-6 gap-y-3 border-b border-border/60 px-4 py-3">
					<div class="flex items-center gap-1">
						<button type="button" class={TOOL} disabled={cur === 0} onclick={() => select(cur - 1)} aria-label={t('lyrics_editor.prev_line')} title={t('lyrics_editor.prev_line')}>
							<HugeiconsIcon icon={ArrowUp01Icon} class="h-4 w-4" />
						</button>
						<span class="min-w-28 text-center text-sm font-medium tabular-nums">
							{t('lyrics_editor.line_of', { n: cur + 1, total: lines.length })}
						</span>
						<button type="button" class={TOOL} disabled={cur >= lines.length - 1} onclick={() => select(cur + 1)} aria-label={t('lyrics_editor.next_line')} title={t('lyrics_editor.next_line')}>
							<HugeiconsIcon icon={ArrowDown01Icon} class="h-4 w-4" />
						</button>
					</div>
					<div class="flex min-w-48 flex-1 items-center gap-3">
						<span class="shrink-0 text-sm text-muted-foreground">{t('lyrics_editor.speed')}</span>
						<Slider
							type="single"
							min={0.5}
							max={1}
							step={0.05}
							value={speedDraft ?? editSpeed}
							onValueChange={(v) => (speedDraft = v)}
							onValueCommit={commitSpeed}
							aria-label={t('lyrics_editor.speed')}
						/>
						<span class="w-11 shrink-0 text-right text-sm font-medium tabular-nums">
							{speedLabel(speedDraft ?? editSpeed)}
						</span>
					</div>
					<label class="flex cursor-pointer items-center gap-2.5 text-sm select-none" title={t('lyrics_editor.loop_hint')}>
						<Switch bind:checked={loopOn} />
						{t('lyrics_editor.loop')}
					</label>
					<!-- A small value, so a field in the sentence rather than a boxed input. -->
					<label class="flex items-center gap-2 text-sm" title={t('lyrics_editor.tap_offset_hint')}>
						<span class="text-muted-foreground">{t('lyrics_editor.tap_offset')}</span>
						<input
							class="w-12 border-b border-border bg-transparent px-0.5 py-0.5 text-right tabular-nums outline-none focus:border-primary"
							inputmode="numeric"
							bind:value={offsetText}
							onblur={commitOffset}
							onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
						/>
						<span class="text-muted-foreground">{t('lyrics_editor.ms_unit')}</span>
					</label>
				</div>

				<div class="px-4 py-4">
					{#if line && line.words.length}
						<LyricTimeline
							words={line.words}
							range={owesTime ? null : range}
							{durationMs}
							running={active && !playback.paused}
							{position}
							nextTap={active && !owesTime && tapCount < line.words.length ? tapCount : -1}
							{bgMode}
							onwords={(words: EdWord[]) => updateLine(line.id, (l) => withWords(l, words))}
							onbg={(k: number) => updateLine(line.id, (l) => toggleBg(l, k))}
							onseek={(ms: number) => active && jump(ms)}
						/>
					{:else}
						<p class="py-2 text-sm text-muted-foreground">{t('lyrics_editor.empty_line')}</p>
					{/if}

					<div class="mt-4 flex flex-wrap items-center gap-x-4 gap-y-3">
						<!-- pointerdown, not click: a click fires on release, a whole key-travel late. A
						     click with no pointer behind it is the keyboard pressing the focused button. -->
						<Button
							size="lg"
							class="min-w-28"
							disabled={!active || !line?.words.length}
							onpointerdown={(e) => e.button === 0 && tap(e)}
							onclick={(e) => e.detail === 0 && tap(e)}
						>
							<HugeiconsIcon icon={Tap01Icon} class="h-4 w-4" />
							{t('lyrics_editor.tap')}
						</Button>
						<button type="button" class={TOOL} disabled={!active || !range} onclick={restart} aria-label={t('lyrics_editor.restart')} title={t('lyrics_editor.restart')}>
							<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-4 w-4" />
						</button>
						<p class="min-w-0 flex-1 text-sm leading-relaxed text-muted-foreground">
							{#if active && playback.paused}
								{t('lyrics_editor.paused_hint')}
							{:else if owesTime}
								{t('lyrics_editor.tap_hint_line')}
							{:else}
								{t('lyrics_editor.tap_hint_words')}
							{/if}
						</p>
					</div>

					<div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
						<span><kbd class={KEY}>F</kbd> <kbd class={KEY}>J</kbd> {t('lyrics_editor.key_tap')}</span>
						<span><kbd class={KEY}>↑</kbd> <kbd class={KEY}>↓</kbd> {t('lyrics_editor.key_lines')}</span>
						<span><kbd class={KEY}>R</kbd> {t('lyrics_editor.restart')}</span>
						<span><kbd class={KEY}>L</kbd> {t('lyrics_editor.loop')}</span>
						<span><kbd class={KEY}>B</kbd> {t('lyrics_editor.bg')}</span>
						<span><kbd class={KEY}>{t('lyrics_editor.key_space')}</kbd> {t('lyrics_editor.key_play')}</span>
					</div>
				</div>

				<!-- The line's text, singer and background vocals. -->
				<div class="space-y-3 border-t border-border/60 px-4 py-3">
					<div class="flex items-center gap-1">
						<input
							bind:this={fieldEl}
							bind:value={field}
							class="h-8 min-w-0 flex-1 rounded-lg border border-input bg-input/30 px-2.5 text-sm outline-none focus-visible:border-ring"
							placeholder={t('lyrics_editor.text_placeholder')}
							aria-label={t('lyrics_editor.text')}
							onfocus={() => (editing = true)}
							onblur={() => {
								editing = false;
								commitField();
							}}
							onkeydown={(e) => {
								if (e.key === 'Enter') e.currentTarget.blur();
								else if (e.key === 'Escape') {
									field = line ? fieldText(line.words) : '';
									e.currentTarget.blur();
								}
							}}
						/>
						<!-- mousedown is swallowed so the field keeps its focus and its caret. -->
						<button type="button" class={TOOL} onmousedown={(e) => e.preventDefault()} onclick={splitHere} aria-label={t('lyrics_editor.split')} title={t('lyrics_editor.split')}>
							<HugeiconsIcon icon={ScissorIcon} class="h-4 w-4" />
						</button>
						<button type="button" class={TOOL} disabled={cur >= lines.length - 1} onclick={joinNext} aria-label={t('lyrics_editor.join')} title={t('lyrics_editor.join')}>
							<HugeiconsIcon icon={MergeIcon} class="h-4 w-4" />
						</button>
						<button type="button" class={TOOL} onclick={resplit} aria-label={t('lyrics_editor.resplit')} title={t('lyrics_editor.resplit')}>
							<HugeiconsIcon icon={MagicWand01Icon} class="h-4 w-4" />
						</button>
						<button type="button" class={TOOL} onclick={addBelow} aria-label={t('lyrics_editor.add_line')} title={t('lyrics_editor.add_line')}>
							<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" />
						</button>
						<button type="button" class={TOOL} disabled={lines.length < 2} onclick={removeLine} aria-label={t('lyrics_editor.delete_line')} title={t('lyrics_editor.delete_line')}>
							<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
						</button>
					</div>
					<p class="text-xs leading-relaxed text-muted-foreground">{t('lyrics_editor.text_hint')}</p>
					<div class="flex flex-wrap items-center gap-x-6 gap-y-2">
						<div class="flex items-center gap-2" role="radiogroup" aria-label={t('lyrics_editor.singer')}>
							<span class="text-sm text-muted-foreground">{t('lyrics_editor.singer')}</span>
							<div class="flex items-center rounded-full border bg-muted/40 p-0.5">
								{#each [undefined, ...agents, 'v1000'] as a (a ?? '')}
									<button
										type="button"
										role="radio"
										aria-checked={(line?.agent ?? undefined) === a}
										disabled={!line}
										onclick={() => line && updateLine(line.id, (l) => setAgent(l, a))}
										class="cursor-pointer rounded-full px-2.5 py-0.5 text-xs font-medium {(line?.agent ??
											undefined) === a
											? 'bg-primary text-primary-foreground'
											: 'text-muted-foreground hover:text-foreground'}"
									>
										{a ? agentLabel(a) : t('lyrics_editor.singer_none')}
									</button>
								{/each}
							</div>
						</div>
						<label class="flex cursor-pointer items-center gap-2.5 text-sm select-none" title={t('lyrics_editor.bg_hint')}>
							<Switch bind:checked={bgMode} />
							<HugeiconsIcon icon={Mic01Icon} class="h-4 w-4 text-muted-foreground" />
							{t('lyrics_editor.bg')}
						</label>
						{#if bgMode}
							<span class="text-xs text-muted-foreground">{t('lyrics_editor.bg_hint')}</span>
						{/if}
					</div>
				</div>
			</section>

			<section class="flex min-h-40 flex-1 flex-col">
				<div class="mb-2 flex items-center gap-3 px-1">
					<h2 class={LABEL}>{t('lyrics_editor.lines')}</h2>
					<p class="min-w-0 flex-1 truncate text-xs text-muted-foreground">
						{#if miss.tooMany}
							{t('lyrics_editor.too_many')}
						{:else if miss.tooLong.length}
							{t('lyrics_editor.too_long', { n: lines.findIndex((l) => l.id === miss.tooLong[0]) + 1 })}
						{:else if miss.untimed.length}
							{t('lyrics_editor.todo', { count: miss.untimed.length, total: sungCount })}
						{:else if !miss.empty}
							{t('lyrics_editor.todo_none')}
						{/if}
					</p>
					{#if miss.untimed.length}
						<button type="button" class="shrink-0 cursor-pointer text-xs font-medium text-primary hover:underline" onclick={nextTodo}>
							{t('lyrics_editor.todo_next')}
						</button>
					{/if}
				</div>
				<!-- No transitions on the rows (docs/UI-PERFORMANCE.md): a long song is a few hundred. -->
				<div bind:this={list} class="{CARD} min-h-0 flex-1 overflow-y-auto p-1.5">
					{#each lines as l, i (l.id)}
						{@const done = wordsTimed(l)}
						<button
							type="button"
							data-line={i}
							aria-current={i === cur}
							onclick={() => select(i)}
							class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-2.5 py-1.5 text-left text-sm {i ===
							cur
								? 'bg-primary/12 font-medium'
								: 'hover:bg-muted/60'}"
						>
							<span class="w-8 shrink-0 text-right text-xs tabular-nums text-muted-foreground">{i + 1}</span>
							<span class="min-w-0 flex-1 truncate {l.words.length ? '' : 'text-muted-foreground'}">
								{lineText(l) || '♪'}
							</span>
							{#if l.agent}
								<span class="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[10px] font-semibold text-muted-foreground">
									{agentLabel(l.agent)}
								</span>
							{/if}
							{#if !l.words.length}
								<span class="w-4 shrink-0"></span>
							{:else if done}
								<span class="shrink-0 text-primary" title={t('lyrics.kind_words')}>
									<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4" />
								</span>
							{:else}
								<span class="shrink-0 text-[11px] text-muted-foreground">
									{needsLineTime(l) ? t('lyrics_editor.line_no_time') : t('lyrics_editor.line_todo')}
								</span>
							{/if}
						</button>
					{/each}
				</div>
			</section>
		{/if}
	{/if}
</div>

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
