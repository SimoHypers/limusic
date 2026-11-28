<script module lang="ts">
	// Romanization (#202) and translations (#329) are one switch each: on stays on across every song
	// until it is turned off. Shared by every lyrics view and the lyric editor's hint (#676), and the
	// `storage` event carries a change over to the mini player window. Translations start on, the
	// way they always showed.
	const KEYS = { romanized: 'lyrics_romanized', translated: 'lyrics_translated' } as const;
	type Pref = keyof typeof KEYS;
	const DEFAULTS: Record<Pref, boolean> = { romanized: false, translated: true };
	const SIZE_KEY = 'lyrics_size';
	const SIZES = [0.85, 1, 1.15, 1.3, 1.5];

	function loadSize(): number {
		try {
			const v = Number(localStorage.getItem(SIZE_KEY));
			return SIZES.includes(v) ? v : 1;
		} catch {
			return 1;
		}
	}

	const size = $state({ value: loadSize() });

	function nudgeSize(delta: number) {
		const next = SIZES[SIZES.indexOf(size.value) + delta];
		if (next === undefined) return;
		size.value = next;
		try {
			localStorage.setItem(SIZE_KEY, String(next));
		} catch {
			// Private storage: the size still applies for this session.
		}
	}

	function load(pref: Pref): boolean {
		try {
			const v = localStorage.getItem(KEYS[pref]);
			return v === null ? DEFAULTS[pref] : v === '1';
		} catch {
			return DEFAULTS[pref];
		}
	}

	export const prefs = $state({ romanized: load('romanized'), translated: load('translated') });
	window.addEventListener('storage', (e) => {
		for (const pref of Object.keys(KEYS) as Pref[]) if (e.key === KEYS[pref]) prefs[pref] = load(pref);
		if (e.key === SIZE_KEY) size.value = loadSize();
	});

	export function toggle(pref: Pref) {
		prefs[pref] = !prefs[pref];
		try {
			localStorage.setItem(KEYS[pref], prefs[pref] ? '1' : '0');
		} catch {
			// Private storage: the toggle still works for this session.
		}
	}
</script>

<script lang="ts">
	import { MediaQuery } from 'svelte/reactivity';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CharacterPhoneticIcon,
		MinusSignIcon,
		MusicNote01Icon,
		PlusSignIcon,
		Search01Icon,
		TextFontIcon,
		TranslateIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { playback } from '$lib/player.svelte';
	import { lite } from '$lib/theme.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
	import { appearance } from '$lib/theme.svelte';
	import * as Popover from '$lib/components/ui/popover';
	import LyricsSourcePicker from './LyricsSourcePicker.svelte';
	import KaraokeButton from './KaraokeButton.svelte';
	import LyricsFileButton from './LyricsFileButton.svelte';
	import { link } from '$lib/link.svelte';
	import { turns, voices } from '$lib/lyricvoices';
	import { durationSecs, lyricsFor, setNowLyrics } from '$lib/nowlyrics.svelte';
	import { SCROLL_EASE, glowing, letters, scrollMs, sweep as sweepStyle } from '$lib/lyricmotion';

	// `expanded` only sizes the type and centres the column. The owner of the extra room (the side
	// panel, or the now-playing view) decides how much there is. Toggling it must not remount this
	// component, or the lyrics refetch and the scroll position is lost.
	// `compact` is the mini-player: a ~220px column with no room for the source footer or a
	// scrollbar. It only shrinks the type and chrome; the sync/auto-scroll logic is identical.
	// `missing` says the playing song has nothing to read (none found, or instrumental). It changes
	// only when a fetch answers, never while the next one loads, so theater can lay out on it.
	// `preview` shows these lyrics in place of the song's own, with no footer: the lyric editor's
	// look at a draft, drawn by the same code that will draw it once it is published.
	let {
		expanded = false,
		compact = false,
		missing = $bindable(false),
		preview
	}: { expanded?: boolean; compact?: boolean; missing?: boolean; preview?: api.Lyrics } = $props();

	let lyrics = $state<api.Lyrics | null>(null);
	let loading = $state(true);
	let scroller: HTMLElement | undefined = $state();

	const canRomanize = $derived(!!lyrics?.lines.some((l) => l.romanized));
	const showRomanized = $derived(canRomanize && prefs.romanized);
	const canTranslate = $derived(!!lyrics?.lines.some((l) => l.translation));
	const showTranslation = $derived(canTranslate && prefs.translated);
	/** Where each line sits and which of its words are background vocals. Null for a lyric with
	 *  neither (most of them): its lines are drawn by the same markup as before, with nothing
	 *  added. Worked out once per lyric, never per frame. */
	const voice = $derived(lyrics?.synced ? voices(lyrics.lines) : null);
	/** Two singers: where there is room (the enlarged views), each also keeps off the other's
	 *  edge, so the turns read at a glance. A side panel is too narrow to give any width away. */
	const duet = $derived(!!voice?.some((v) => v.far));
	/** The singer's name above the first line of each turn (#768). Null for a lyric that names
	 *  nobody, which is every lyric but some of Limusic's. */
	const turn = $derived(lyrics?.synced ? turns(lyrics.lines, lyrics.singers) : null);
	const enhanced = $derived(appearance.enhancedLyrics);
	const animateSynced = $derived(enhanced && appearance.enhancedLyricsLineSync);
	const reducedMotion = new MediaQuery('prefers-reduced-motion: reduce');
	// Nothing to resize on an instrumental or a song with no lyrics at all.
	const canResize = $derived(!!lyrics && !lyrics.instrumental);
	/** "115%". Locale formatting, so no i18n key and no `%` to place. */
	const sizeLabel = $derived(
		new Intl.NumberFormat(currentLocale.id, { style: 'percent' }).format(size.value)
	);
	const sizeChanged = $derived(size.value !== 1);
	/** This song's timing nudge, from the source picker. Positive holds the lyrics back. */
	const offsetMs = $derived(lyrics?.offset_ms ?? 0);
	let pickerOpen = $state(false);

	/** What the source picker asks the providers about. mpv's length stands in when the queue item
	 *  has none: by the time anyone opens the picker, the song is the one playing. */
	const track = $derived.by((): api.LyricsTrack | null => {
		const now = playback.now;
		if (!now) return null;
		return {
			videoId: now.videoId,
			title: now.title,
			artists: now.artists,
			album: now.album ?? undefined,
			duration: durationSecs(now.duration) ?? (playback.duration > 0 ? playback.duration : undefined)
		};
	});

	function onPicked(l: api.Lyrics | null, videoId: string) {
		if (requested !== videoId) return; // the song moved on while it was being fetched
		lyrics = l;
		missing = !l || l.instrumental;
		hasScrolled = false;
		setNowLyrics(videoId, l);
	}

	// videoId of the fetch whose result is (or will be) shown — guards stale responses.
	let requested = '';

	// Shared with the line under the now-playing cover, which writes and reads the same answer.
	const ask = (now: NonNullable<typeof playback.now>) =>
		lyricsFor(now).then((l) => (setNowLyrics(now.videoId, l), l));

	// The cache brought this song's lyrics up to date behind the ones on screen (`refresh_stale`):
	// swap them in where they are, with no loading state and no jump.
	$effect(() => {
		if (preview) return;
		const sub = api.onLyricsUpdated((id) => {
			const now = playback.now;
			if (!now || now.videoId !== id || requested !== id) return;
			ask(now)
				.then((l) => {
					if (requested !== id) return;
					lyrics = l;
					missing = !l || l.instrumental;
				})
				.catch(() => {});
		});
		return () => void sub.then((u) => u());
	});

	$effect(() => {
		if (preview) {
			requested = '';
			lyrics = preview;
			loading = false;
			return;
		}
		const now = playback.now;
		if (!now) {
			requested = '';
			lyrics = null;
			loading = false;
			return;
		}
		if (now.videoId === requested) return;
		const id = (requested = now.videoId);
		loading = true;
		lyrics = null;
		ask(now)
			.then((l) => {
				if (requested !== id) return;
				lyrics = l;
				missing = !l || l.instrumental;
				loading = false;
				hasScrolled = false; // first positioning on a new track is an instant jump
			})
			.catch(() => {
				if (requested !== id) return;
				missing = true;
				loading = false;
			});
	});

	/** Last synced line whose cue has passed by `ms` (lines arrive sorted by time). */
	function lineAt(ms: number): number {
		if (!lyrics?.synced) return -1;
		let i = -1;
		for (let j = 0; j < lyrics.lines.length; j++) {
			const t = lyrics.lines[j].time_ms;
			if (t === undefined) continue;
			if (t > ms) break;
			i = j;
		}
		return i;
	}
	const activeIndex = $derived.by(() => lineAt(posMs));
	/** A line before the one being sung whose background vocals are still going, -1 for none. It
	 *  stays lit, so an answer sung over the next line's start keeps sweeping instead of going dim
	 *  halfway. A lyric without background vocals never reads the clock here.
	 *  ponytail: the nearest such line only; two at once would need a set. */
	const lingering = $derived.by(() => {
		if (!voice) return -1;
		for (let j = activeIndex - 1; j >= Math.max(0, activeIndex - 3); j--) {
			if (posMs < voice[j].until) return j;
		}
		return -1;
	});
	// The view moves to a line before it is sung, so the line lands as the singer reaches it rather
	// than sliding in under the first word. Its highlight still waits for its cue.
	const scrollIndex = $derived.by(() => lineAt(posMs + 500));

	// Auto-scroll pauses while the user is scrolling (wheel/touch/scrollbar), resumes after 3s.
	// Tracked via input events, not `scroll`, so our own scrolls don't trip it.
	let userScrollUntil = 0;
	let hasScrolled = false;
	function onUserScroll() {
		userScrollUntil = Date.now() + 3000;
	}

	let wasLayout: string | undefined;

	$effect(() => {
		const i = scrollIndex;
		// Re-place after the layout width/font changes or the romanized lines come and go, and
		// jump rather than glide across it. (Also fires on the first run.)
		const layout = `${expanded} ${showRomanized} ${size.value}`;
		if (layout !== wasLayout) {
			wasLayout = layout;
			hasScrolled = false;
			userScrollUntil = 0;
		}
		if (i < 0 || !scroller || Date.now() < userScrollUntil) return;
		// Opening mid-song jumps straight to the line; after that, glide — unless lite mode
		// asked for line flips instead: a glide repaints the whole panel for its length.
		glide(scroller, i, hasScrolled && !lite());
		hasScrolled = true;
	});

	/** The line glides that are still running, so a jump can stop them. */
	let gliding: Animation[] = [];

	/** Puts line `i` a bit above the middle, where more of what's coming shows than of what's gone.
	 *  The scroller jumps there at once and every line on screen eases from where it was, each on its
	 *  own time (`scrollMs`), Better Lyrics' scroll. A glide that starts while one runs adds to it
	 *  (`composite: 'add'`), so lines that come fast keep moving instead of restarting from rest.
	 *  Positions are layout ones (`offsetTop`, the scroller is `relative` so it is their offsetParent),
	 *  which neither a running glide's `translate` nor a line's `scale` moves.
	 *  Scrolls the scroller itself, never `scrollIntoView`: that walks up and scrolls every
	 *  scrollable ancestor too, including theater mode's `overflow-hidden` grid (issue #168). */
	function glide(box: HTMLElement, i: number, smooth: boolean) {
		const rows = box.querySelectorAll<HTMLElement>('[data-line]');
		const line = rows[i];
		if (!line) return;
		const h = box.clientHeight;
		const from = box.scrollTop;
		const to = Math.max(
			0,
			Math.min(box.scrollHeight - h, line.offsetTop + line.offsetHeight / 2 - h * 0.37)
		);
		const delta = to - from;
		if (Math.abs(delta) < 1) return;
		box.scrollTo({ top: to, behavior: 'instant' });
		gliding = gliding.filter((a) => a.playState === 'running');
		if (!smooth) {
			for (const a of gliding) a.cancel();
			gliding = [];
			return;
		}
		// From a viewport away at most, so a long way back after a seek or a manual scroll moves
		// what will be on screen, not every line in between.
		const shift = Math.max(-h, Math.min(h, delta));
		// A jump of several lines tightens the stagger, so the lines don't run into each other.
		const near = [rows[i - 1], rows[i + 1]]
			.filter(Boolean)
			.map((r) => Math.abs(r.offsetTop - line.offsetTop));
		const step = near.length ? Math.min(...near) : line.offsetHeight;
		const stagger = reducedMotion.current ? 0 : Math.min(1, step / Math.abs(delta));
		const base = scrollMs(0, false);
		// A row still gliding may be on screen only by its glide, so it takes the next one whatever
		// its layout position says, or it would jump by the scroll.
		const moving = new Set(gliding.map((a) => (a.effect as KeyframeEffect | null)?.target));
		rows.forEach((row, j) => {
			const top = row.offsetTop - to;
			const off =
				Math.max(top, top + shift) + row.offsetHeight < 0 || Math.min(top, top + shift) > h;
			if (off && !moving.has(row)) return;
			const rel = j - i;
			const ahead = delta > 0 ? rel < 0 : rel > 0;
			gliding.push(
				row.animate([{ translate: `0 ${shift}px` }, { translate: '0 0' }], {
					duration: base + (scrollMs(rel, ahead) - base) * stagger,
					easing: SCROLL_EASE,
					composite: 'add'
				})
			);
		});
	}

	function seekTo(line: api.LyricLine) {
		if (line.time_ms === undefined) return;
		const secs = Math.max(0, (line.time_ms + offsetMs) / 1000);
		playback.position = secs; // optimistic — the mpv tick confirms
		userScrollUntil = 0; // jump the view along with the seek
		api.seek(secs);
	}

	// mpv's position arrives ~4x a second. Run a local clock forward from each one so the karaoke
	// sweep moves every frame instead of stepping four times a second.
	let interpolatedPosSecs = $state(playback.position);

	/** The rAF clock exists for the word sweep and nothing else. Unsynced lyrics have no cues, and
	 *  line-level-only lyrics move at most once a line, so both are served perfectly well by the
	 *  position tick they already get unless "also animate synced lyrics" estimates word timing for
	 *  them. Without this gate the loop ran at refresh rate for any mounted lyrics panel, on every
	 *  track, for the whole session, which meant the app never reached an idle frame. Lite mode
	 *  closes it too: the sweep then steps at the ~4 Hz position tick, which is the same update
	 *  line-only lyrics already live with — visible highlight still follows the song, without a
	 *  repaint per display frame. */
	const needsFrameClock = $derived(
		!!lyrics?.synced &&
			(lyrics.lines.some(
				(l) =>
					(l.words?.length ?? 0) > 0 ||
					(showRomanized && (l.romanized_words?.length ?? 0) > 0)
			) ||
				animateSynced) &&
			!lite()
	);

	$effect(() => {
		const pos = playback.position;
		if (playback.paused || !needsFrameClock) {
			interpolatedPosSecs = pos;
			return;
		}
		// Rebase on every run. Rebasing only when the value moved kept the base timestamp from
		// before a pause, so resuming after N seconds paused ran the clock N seconds fast until
		// the next tick corrected it.
		const base = pos;
		const baseAt = performance.now();
		// The song moves `speed` seconds a second. Without it the sweep ran ahead at a slowed tempo
		// and snapped back on every position tick (60 ms at 0.75x, 125 ms at 0.5x).
		const speed = playback.speed;
		interpolatedPosSecs = pos;
		let frameId = requestAnimationFrame(function tick() {
			interpolatedPosSecs = base + ((performance.now() - baseAt) / 1000) * speed;
			frameId = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(frameId);
	});

	const posMs = $derived(interpolatedPosSecs * 1000 - offsetMs);

	function getWordProgress(word: api.LyricWord, currentMs: number): number {
		if (currentMs <= word.start_ms) return 0;
		if (currentMs >= word.end_ms) return 1;
		const dur = word.end_ms - word.start_ms;
		if (dur <= 0) return 1;
		return (currentMs - word.start_ms) / dur;
	}

	function getWordTransform(word: api.LyricWord, currentMs: number): string {
		if (reducedMotion.current) return 'translate3d(0,0,0)';
		const anim = appearance.lyricsAnim;
		const strength = appearance.lyricsAnimStrength / 100;
		if (anim === 'none' || strength <= 0) return 'translate3d(0,0,0)';
		const elapsed = currentMs - word.start_ms;
		if (anim === 'slide') {
			if (elapsed <= 0) return `translate3d(${(4 * strength).toFixed(2)}px,0,0)`;
			if (elapsed >= 280) return 'translate3d(0,0,0)';
			const t = elapsed / 280;
			const ease = 1 - (1 - t) * (1 - t);
			const x = 4 * strength * (1 - ease);
			const y = -3 * strength * Math.sin(t * Math.PI);
			return `translate3d(${x.toFixed(2)}px,${y.toFixed(2)}px,0)`;
		}
		if (elapsed <= 0) return 'translate3d(0,0,0)';
		let y = 0;
		if (anim === 'bounce') {
			if (elapsed < 140) {
				const t = elapsed / 140;
				y = -6 * Math.sin(t * (Math.PI / 2));
			} else if (elapsed < 280) {
				const t = (elapsed - 140) / 140;
				y = -6 * (1 - t * t);
			} else if (elapsed < 430) {
				const t = (elapsed - 280) / 150;
				y = -2.6 * Math.sin(t * Math.PI);
			} else if (elapsed < 560) {
				const t = (elapsed - 430) / 130;
				y = -0.9 * Math.sin(t * Math.PI);
			}
		} else if (anim === 'float') {
			const dur = Math.max(220, word.end_ms - word.start_ms);
			if (elapsed < dur) {
				const t = Math.min(1, elapsed / 180);
				y = -4.5 * Math.sin(t * (Math.PI / 2));
			} else if (elapsed < dur + 260) {
				const t = (elapsed - dur) / 260;
				y = -4.5 + 3.3 * Math.sin(t * (Math.PI / 2));
			} else {
				y = -1.2;
			}
		} else if (anim === 'wave') {
			if (elapsed < 700) {
				y = -5.5 * Math.sin(elapsed / 75) * Math.exp(-elapsed / 380);
			}
		} else if (anim === 'pop') {
			if (elapsed < 320) {
				const t = elapsed / 320;
				y = -7 * Math.sin(t * Math.PI) * (1 - t * 0.35);
			}
		}
		return `translate3d(0,${(y * strength).toFixed(2)}px,0)`;
	}

	function getEnhancedWordStyle(word: api.LyricWord, progress: number, currentMs: number): string {
		const pct = Math.round(Math.min(1, Math.max(0, progress)) * 100);
		const isCurrent = progress > 0 && progress < 1;
		const isSung = progress >= 1;
		const style = appearance.lyricsStyle;
		const dim = 'color-mix(in oklab, var(--muted-foreground) 45%, transparent)';
		const lit =
			style === 'accent'
				? 'color-mix(in oklab, var(--primary) 78%, var(--foreground))'
				: 'var(--foreground)';
		let bg: string;
		if (isSung) {
			bg = `linear-gradient(90deg, ${lit} 0%, ${lit} 100%)`;
		} else if (progress <= 0) {
			bg = `linear-gradient(90deg, ${dim} 0%, ${dim} 100%)`;
		} else if (style === 'word') {
			const mix = `color-mix(in oklab, ${lit} ${pct}%, ${dim})`;
			bg = `linear-gradient(90deg, ${mix} 0%, ${mix} 100%)`;
		} else if (style === 'crisp') {
			bg = `linear-gradient(90deg, ${lit} ${pct}%, ${dim} ${pct}%)`;
		} else if (style === 'accent') {
			bg = `linear-gradient(90deg, ${lit} ${Math.max(0, pct - 12)}%, var(--primary) ${pct}%, ${dim} ${Math.min(100, pct + 12)}%)`;
		} else {
			bg = `linear-gradient(90deg, ${lit} ${Math.max(0, pct - 10)}%, ${dim} ${Math.min(100, pct + 10)}%)`;
		}
		const transform = getWordTransform(word, currentMs);
		const glowPct = appearance.lyricsGlow;
		let filter = 'none';
		if (glowPct > 0 && progress > 0) {
			const g = glowPct / 50;
			const glowColor = style === 'accent' ? 'var(--primary)' : 'var(--foreground)';
			const radius = Math.max(2, Math.round((isCurrent ? 8 : 6) * Math.max(0.5, g)));
			const alpha = Math.min(85, Math.round((isCurrent ? 20 + progress * 25 : 34) * g));
			filter = `drop-shadow(0 0 ${radius}px color-mix(in oklab, ${glowColor} ${alpha}%, transparent))`;
		}
		return `background-image:${bg};transform:${transform};filter:${filter}`;
	}

	function getLineWords(line: api.LyricLine, index: number, text?: string): api.LyricWord[] {
		if (!text && line.words && line.words.length > 0) return line.words;
		if (text && line.romanized_words && line.romanized_words.length > 0) return line.romanized_words;
		if (!animateSynced || !lyrics || line.time_ms === undefined) return [];
		const raw = (text ?? line.text).trim();
		if (!raw || raw === '♪') return [];
		const tokens = raw.split(/\s+/).filter(Boolean);
		if (!tokens.length) return [];
		const startMs = line.time_ms;
		let nextMs: number | undefined;
		for (let j = index + 1; j < lyrics.lines.length; j++) {
			const t = lyrics.lines[j].time_ms;
			if (t !== undefined && t > startMs) {
				nextMs = t;
				break;
			}
		}
		const endMs = line.end_time_ms;
		const cueEndMs =
			endMs !== undefined && endMs > startMs
				? nextMs !== undefined && nextMs > startMs
					? Math.min(endMs, nextMs)
					: endMs
				: nextMs !== undefined && nextMs > startMs
					? nextMs
					: startMs + 3500;
		const rawSpan = Math.max(100, cueEndMs - startMs);
		const vocalSpan = Math.min(rawSpan, Math.min(rawSpan * 0.88, Math.max(1000, tokens.length * 400)));
		const weights = tokens.map((w) => Math.max(2, w.length));
		const totalWeight = weights.reduce((a, b) => a + b, 0);
		let cursor = startMs;
		return tokens.map((token, idx) => {
			const slice = (weights[idx] / totalWeight) * vocalSpan;
			const wordStart = cursor;
			cursor += slice;
			return {
				text: idx < tokens.length - 1 ? `${token} ` : token,
				start_ms: wordStart,
				end_ms: cursor
			};
		});
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -- handlers only detect scroll intent -->
<div
	bind:this={scroller}
	style="font-size:{size.value}rem"
	onwheel={onUserScroll}
	ontouchmove={onUserScroll}
	onpointerdown={onUserScroll}
	class="relative min-h-0 flex-1 overflow-y-auto {compact
		? 'px-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden'
		: expanded
			? 'px-10 py-6'
			: 'px-5 py-6'}"
>
	{#if loading}
		<div class="space-y-3">
			{#each { length: 8 } as _, i (i)}
				<div class="h-5 animate-pulse rounded bg-muted" style="width:{55 + ((i * 17) % 40)}%"></div>
			{/each}
		</div>
	{:else if lyrics?.instrumental}
		<p class="py-8 text-center text-lg text-muted-foreground">{t('lyrics.instrumental')} ♪</p>
	{:else if lyrics && lyrics.synced}
		<!-- Bottom padding only, so the last lines can still center-scroll. A matching top padding
		     would put half a panel of void above line 1, which is all you see until the song has
		     played far enough to scroll past it (issue #201). Instead the opening lines sit at the
		     top and centering starts once there is room above, the way every other lyrics view
		     behaves. -->
		<div class="pb-[55vh] {expanded ? 'mx-auto max-w-3xl' : ''}">
			{#each lyrics.lines as line, i (i)}
				{@const isActive = i === activeIndex || i === lingering}
				{@const isPast = i < activeIndex}
				<!-- A held note's halo stays a line longer than the sweep, so the note it ends a
				     line on lets go of its glow after the next line has started. -->
				{@const halo = isActive || i === activeIndex - 1}
				{@const words = getLineWords(line, i)}
				{@const romWords =
					showRomanized && line.romanized ? getLineWords(line, i, line.romanized) : []}
				{@const v = voice?.[i]}
				<!-- Dimming is `opacity` on the line, never a translucent text colour. A colour with
				     alpha is composited glyph by glyph, so wherever two glyphs overlap the coverage
				     adds up and the overlap comes out brighter than the rest of the line: very
				     visible in scripts whose marks sit on top of the letters, Devanagari in issue
				     #191. `opacity` paints the line opaque first and fades it once, as a group.
				     Light themes dim less: the dark themes' 70% put dark grey on white at about 2.8:1,
				     90% matches the 4:1 the dark themes get.
				     The line being sung is full size and the rest a step smaller, from the side it
				     reads from, so the active line grows into place rather than past its neighbours.
				     With motion on `scale` is always set, so a line keeps its layer while its opacity
				     crosses 1 (the opacity rule in docs/UI-PERFORMANCE.md). -->
				<button
					data-line={i}
					onclick={() => seekTo(line)}
					class="block w-full cursor-pointer font-heading font-bold leading-snug transition-[color,opacity,scale] duration-500 ease-[cubic-bezier(0.2,0.7,0.3,1)] hover:text-foreground hover:opacity-100 dark:hover:opacity-100
						{v?.far ? 'origin-right text-right' : 'origin-left text-left'}
						{duet && expanded ? (v?.far ? 'pl-[14%]' : 'pr-[14%]') : ''}
						{expanded ? 'py-3 text-[1.875em]' : compact ? 'py-1 text-[0.875em]' : 'py-2 text-[1.25em]'}
						{isActive
						? 'text-foreground motion-safe:scale-100'
						: isPast
							? 'text-muted-foreground opacity-55 motion-safe:scale-[0.96] dark:opacity-40'
							: 'text-muted-foreground opacity-90 motion-safe:scale-[0.96] dark:opacity-70'}"
				>
					{#if turn?.[i]}
						<!-- Whose turn it is, part of the line: it sits on the line's side, dims and scales
						     with it, and `glide` centres the two together. -->
						<span class="mb-[0.15em] block text-[length:max(0.5em,10px)] font-semibold tracking-wide opacity-75">
							{turn[i]}
						</span>
					{/if}
					{#if v?.runs && words === line.words}
						<!-- Background vocals are a quieter line of their own, under the lead they
						     answer: smaller, and dimmed as a group (see the note on `opacity` above).
						     They sweep on their own timings, which overlap the lead's. -->
						{#each v.runs as run, r (r)}
							{#if run.bg}
								<span class="block py-[0.1em] text-[0.72em] font-semibold opacity-65">
									{@render sweep(run.words, isActive, halo, v.far)}
								</span>
							{:else}
								{@render sweep(run.words, isActive, halo, v.far)}
							{/if}
						{/each}
					{:else if words.length > 0}
						{@render sweep(words, isActive, halo, v?.far)}
					{:else if line.text}
						<span>{line.text}</span>
					{:else}
						<!-- An instrumental break: a timed line with no text. An icon, not the font's
						     ♪, which the heading face lacks and a fallback draws thin and small. It
						     pulses while the break plays, like the line under the cover. -->
						<HugeiconsIcon
							icon={MusicNote01Icon}
							class="inline-block size-[1.1em] {isActive
								? `motion-safe:animate-note-pulse ${playback.paused ? '[animation-play-state:paused]' : ''}`
								: ''}"
						/>
						<span class="sr-only">{t('lyrics.instrumental')}</span>
					{/if}

					{#if showRomanized && line.romanized}
						<!-- Relative size, so it follows the line from the mini player to theater mode.
						     A timed reading (Apple's, or ours on a word-synced line) sweeps with the line. -->
						<span class="mt-0.5 block text-[length:max(0.62em,11px)] font-semibold">
							{#if romWords.length > 0}
								{@render sweep(romWords, isActive, halo, v?.far)}
							{:else}
								{line.romanized}
							{/if}
						</span>
					{/if}

					{#if showTranslation && line.translation}
						<!-- `rem` times the size, never `em`: inside the line an `em` follows the line's own
						     size, so theater's translations came out twice their old 14px at 100%. -->
						<p
							class="mt-1 text-sm font-normal italic tracking-wide opacity-80 transition-opacity"
							style="font-size:{0.875 * size.value}rem"
						>
							{line.translation}
						</p>
					{/if}
				</button>
			{/each}
		</div>
	{:else if lyrics}
		<div
			class="space-y-2 leading-relaxed text-foreground opacity-90 {expanded
				? 'mx-auto max-w-3xl text-[1.25em]'
				: compact
					? 'text-[0.75em]'
					: 'text-[0.9375em]'}"
		>
			{#each lyrics.lines as line, i (i)}
				{#if line.text}
					<div>
						<p>{line.text}</p>
						{#if showRomanized && line.romanized}
							<p class="text-[0.85em] text-muted-foreground">{line.romanized}</p>
						{/if}
						{#if showTranslation && line.translation}
							<p class="text-xs italic text-muted-foreground" style="font-size:{0.75 * size.value}rem">
								{line.translation}
							</p>
						{/if}
					</div>
				{:else}
					<div class="h-4"></div>
				{/if}
			{/each}
		</div>
	{:else}
		<div class="flex flex-col items-center gap-1.5 py-8 text-center">
			<p class="text-sm text-muted-foreground">{t('lyrics.none_found')}</p>
			{#if !compact && track}
				<p class="max-w-64 text-xs text-muted-foreground/80">{t('lyrics.none_found_hint')}</p>
				<button
					onclick={() => (pickerOpen = true)}
					class="mt-2 flex cursor-pointer items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-medium transition-colors hover:bg-foreground/5"
				>
					<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5" />
					{t('lyrics.find')}
				</button>
			{/if}
		</div>
	{/if}
</div>
{#if track && !loading && !compact && !preview}
	<div class="mt-3 flex items-center gap-1 border-t border-border/60 px-4 pt-2 pb-1.5 text-xs text-muted-foreground">
		<!-- The source is the switch (#23): a popover with every provider's answer for this song. -->
		<div class="flex min-w-0 flex-1">
			<LyricsSourcePicker {lyrics} {track} bind:open={pickerOpen} onchange={onPicked} />
		</div>
		<!-- Each only on lyrics that have something for it, so neither sits there dead. The mini
		     player has no footer and follows whatever was chosen here. -->
		{#if canTranslate}
			{@render prefToggle(
				'translated',
				showTranslation,
				TranslateIcon,
				t('lyrics.translation'),
				t('lyrics.translation_hint')
			)}
		{/if}
		{#if canRomanize}
			{@render prefToggle(
				'romanized',
				showRomanized,
				CharacterPhoneticIcon,
				t('lyrics.romanize'),
				t('lyrics.romanize_hint')
			)}
		{/if}
		<!-- Karaoke's switch lives here, not in the player bar (KaraokeButton.svelte). It works on
		     this desktop's player, so it goes while the bar controls a linked device. -->
		{#if !link.remote}<KaraokeButton />{/if}
		<!-- One icon, because the footer is full (the side panel is 320px). The popover holds the
		     `− 115% +` stepper; the ends go dead rather than wrapping. -->
		{#if canResize}
			<Popover.Root>
				<Popover.Trigger
					class="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-2 py-0.5 transition-colors hover:bg-foreground/10 hover:text-foreground data-[state=open]:bg-foreground/10 data-[state=open]:text-foreground {sizeChanged
						? 'text-primary'
						: ''}"
					title={t('lyrics.text_size_hint')}
					aria-label={t('lyrics.text_size')}
				>
					<HugeiconsIcon icon={TextFontIcon} class="h-3.5 w-3.5" />
				</Popover.Trigger>
				<Popover.Content side="top" align="end" sideOffset={8} class="w-auto gap-0 p-0">
					<div class="flex items-center gap-3 px-4 py-3">
						<HugeiconsIcon icon={TextFontIcon} class="h-4 w-4 shrink-0 text-muted-foreground" />
						<div class="min-w-0 flex-1">
							<p class="text-sm font-medium">{t('lyrics.text_size')}</p>
							<p class="text-xs leading-snug text-muted-foreground">
								{t('lyrics.text_size_hint')}
							</p>
						</div>
						<div class="flex shrink-0 items-center rounded-full border bg-muted/40">
							<button
								class="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
								disabled={size.value === SIZES[0]}
								aria-label={t('lyrics.text_size_smaller')}
								title={t('lyrics.text_size_smaller')}
								onclick={() => nudgeSize(-1)}
							>
								<HugeiconsIcon icon={MinusSignIcon} class="h-3.5 w-3.5" />
							</button>
							<span
								class="w-14 text-center text-xs font-medium tabular-nums {sizeChanged
									? 'text-primary'
									: ''}"
								aria-live="polite"
							>
								{sizeLabel}
							</span>
							<button
								class="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
								disabled={size.value === SIZES[SIZES.length - 1]}
								aria-label={t('lyrics.text_size_larger')}
								title={t('lyrics.text_size_larger')}
								onclick={() => nudgeSize(1)}
							>
								<HugeiconsIcon icon={PlusSignIcon} class="h-3.5 w-3.5" />
							</button>
						</div>
					</div>
				</Popover.Content>
			</Popover.Root>
		{/if}
		<!-- Only on the user's own Limusic lyric, so most songs never show it. -->
		<LyricsFileButton {lyrics} {track} />
	</div>
{/if}

{#snippet prefToggle(pref: Pref, on: boolean, icon: typeof TranslateIcon, label: string, hint: string)}
	<button
		onclick={() => toggle(pref)}
		class="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-2 py-0.5 transition-colors hover:bg-foreground/10 {on
			? 'text-primary'
			: 'hover:text-foreground'}"
		aria-pressed={on}
		title={hint}
	>
		<HugeiconsIcon {icon} class="h-3.5 w-3.5" />
		{label}
	</button>
{/snippet}

<!-- Word-by-word karaoke sweep (Better Lyrics style), for the line and for the timed
     romanization under it. A word is the same element whether its line is being sung or not, so a
     change of line restyles words instead of rebuilding them. Off the active line its colour and
     dimming come from the line; on it the sweep paints it, and a held note lights, crests and
     glows letter by letter (`lyricmotion.ts`). Enhanced lyrics draw their own style and motion
     per frame instead. -->
{#snippet sweep(words: api.LyricWord[], active: boolean, halo: boolean, far = false)}
	<span class="inline-flex flex-wrap items-baseline {far ? 'justify-end' : ''}">
		{#each words as word, wIdx (wIdx)}
			<!-- No gap after the last word: on a far-side line it would hold the text off the edge. -->
			{@const gap = word.text.endsWith(' ') && wIdx < words.length - 1 ? 'mr-[0.26em]' : ''}
			{@const held = enhanced ? null : letters(word)}
			{#if held}
				<span class="relative isolate inline-block whitespace-nowrap {gap}"
					>{#if halo}{@render glow(held, word, posMs)}{/if}{#each held as l, k (k)}{@render glyph(
							l,
							active,
							'lyric-held'
						)}{/each}</span
				>
			{:else}
				{@render glyph(word, active, gap)}
			{/if}
		{/each}
	</span>
{/snippet}

<!-- A held note's halo (`.lyric-halo`): the same letters, clear, under a text shadow. -->
{#snippet glow(held: api.LyricWord[], note: api.LyricWord, ms: number)}<span
		class="lyric-halo"
		aria-hidden="true"
		>{#each held as l, k (k)}<span class={glowing(l, note, ms) ? 'lit' : ''}>{l.text}</span
			>{/each}</span
	>{/snippet}

<!-- The clip and the fill are classes, so per frame only the gradient's stops are re-serialised.
     Both colours are theme tokens: the sung half was hardcoded white once, invisible on every
     light theme. -->
{#snippet glyph(word: api.LyricWord, active: boolean, extra: string)}<span
		class="inline-block {extra} {active
			? `bg-clip-text text-transparent [-webkit-text-fill-color:transparent] ${extra === 'lyric-held' && posMs >= word.start_ms ? 'sung' : ''}`
			: ''}"
		style={!active
			? undefined
			: enhanced
				? getEnhancedWordStyle(word, getWordProgress(word, posMs), posMs)
				: sweepStyle(word, posMs)}>{word.text.trimEnd()}</span
	>{/snippet}
