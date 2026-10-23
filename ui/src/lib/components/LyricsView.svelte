<script module lang="ts">
	// Romanization (#202) and translations (#329) are one switch each: on stays on across every song
	// until it is turned off. Shared by every lyrics view, and the `storage` event carries a change
	// over to the mini player window. Translations start on, the way they always showed.
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

	const prefs = $state({ romanized: load('romanized'), translated: load('translated') });
	window.addEventListener('storage', (e) => {
		for (const pref of Object.keys(KEYS) as Pref[]) if (e.key === KEYS[pref]) prefs[pref] = load(pref);
		if (e.key === SIZE_KEY) size.value = loadSize();
	});

	function toggle(pref: Pref) {
		prefs[pref] = !prefs[pref];
		try {
			localStorage.setItem(KEYS[pref], prefs[pref] ? '1' : '0');
		} catch {
			// Private storage: the toggle still works for this session.
		}
	}
</script>

<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CharacterPhoneticIcon,
		MinusSignIcon,
		PlusSignIcon,
		Search01Icon,
		TextFontIcon,
		TranslateIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { playback } from '$lib/player.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
	import * as Popover from '$lib/components/ui/popover';
	import LyricsSourcePicker from './LyricsSourcePicker.svelte';

	// `expanded` only sizes the type and centres the column. The owner of the extra room (the side
	// panel, or the now-playing view) decides how much there is. Toggling it must not remount this
	// component, or the lyrics refetch and the scroll position is lost.
	// `compact` is the mini-player: a ~220px column with no room for the source footer or a
	// scrollbar. It only shrinks the type and chrome; the sync/auto-scroll logic is identical.
	// `missing` says the playing song has nothing to read (none found, or instrumental). It changes
	// only when a fetch answers, never while the next one loads, so theater can lay out on it.
	let {
		expanded = false,
		compact = false,
		missing = $bindable(false)
	}: { expanded?: boolean; compact?: boolean; missing?: boolean } = $props();

	/** "3:21" / "1:02:03" → seconds. */
	function durationSecs(d?: string): number | undefined {
		if (!d) return undefined;
		const parts = d.split(':').map(Number);
		if (!parts.length || parts.some(Number.isNaN)) return undefined;
		return parts.reduce((a, b) => a * 60 + b, 0);
	}

	let lyrics = $state<api.Lyrics | null>(null);
	let loading = $state(true);
	let scroller: HTMLElement | undefined = $state();

	const canRomanize = $derived(!!lyrics?.lines.some((l) => l.romanized));
	const showRomanized = $derived(canRomanize && prefs.romanized);
	const canTranslate = $derived(!!lyrics?.lines.some((l) => l.translation));
	const showTranslation = $derived(canTranslate && prefs.translated);
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
	}

	// videoId of the fetch whose result is (or will be) shown — guards stale responses.
	let requested = '';

	$effect(() => {
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
		api.getLyrics({
			videoId: id,
			title: now.title,
			artists: now.artists,
			// From now-playing, not the queue row: on a gapless advance the queue event lands after
			// this one, and the previous song's album makes Boidu and LRCLIB miss.
			album: now.album ?? undefined,
			// The track's own length — NOT playback.duration, which still holds the previous
			// track's value for a moment after a track change.
			duration: durationSecs(now.duration)
		})
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

	// Last synced line whose cue has passed (lines arrive sorted by time).
	const activeIndex = $derived.by(() => {
		if (!lyrics?.synced) return -1;
		const currentMs = posMs;
		let i = -1;
		for (let j = 0; j < lyrics.lines.length; j++) {
			const t = lyrics.lines[j].time_ms;
			if (t === undefined) continue;
			if (t > currentMs) break;
			i = j;
		}
		return i;
	});

	// Auto-scroll pauses while the user is scrolling (wheel/touch/scrollbar), resumes after 3s.
	// Tracked via input events, not `scroll`, so our own smooth scrolls don't trip it.
	let userScrollUntil = 0;
	let hasScrolled = false;
	function onUserScroll() {
		userScrollUntil = Date.now() + 3000;
	}

	let wasLayout: string | undefined;

	$effect(() => {
		const i = activeIndex;
		// Re-centre after the layout width/font changes or the romanized lines come and go, and
		// jump rather than glide across it. (Also fires on the first run.)
		const layout = `${expanded} ${showRomanized} ${size.value}`;
		if (layout !== wasLayout) {
			wasLayout = layout;
			hasScrolled = false;
			userScrollUntil = 0;
		}
		if (i < 0 || !scroller || Date.now() < userScrollUntil) return;
		const line = scroller.querySelector(`[data-line="${i}"]`);
		if (!line) return;
		// Scroll the scroller itself, never `scrollIntoView`: that walks up and scrolls every
		// scrollable ancestor too, including theater mode's `overflow-hidden` grid, which then has
		// no scrollbar to put it back (issue #168).
		const lineRect = line.getBoundingClientRect();
		const boxRect = scroller.getBoundingClientRect();
		scroller.scrollTo({
			top:
				scroller.scrollTop + (lineRect.top - boxRect.top) - (boxRect.height - lineRect.height) / 2,
			// Opening mid-song jumps straight to the line; after that, glide.
			behavior: hasScrolled ? 'smooth' : 'instant'
		});
		hasScrolled = true;
	});

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
	 *  position tick they already get. Without this gate the loop ran at refresh rate for any
	 *  mounted lyrics panel, on every track, for the whole session, which meant the app never
	 *  reached an idle frame. */
	const needsFrameClock = $derived(
		!!lyrics?.synced && lyrics.lines.some((l) => (l.words?.length ?? 0) > 0)
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
		interpolatedPosSecs = pos;
		let frameId = requestAnimationFrame(function tick() {
			interpolatedPosSecs = base + (performance.now() - baseAt) / 1000;
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
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -- handlers only detect scroll intent -->
<div
	bind:this={scroller}
	style="font-size:{size.value}rem"
	onwheel={onUserScroll}
	ontouchmove={onUserScroll}
	onpointerdown={onUserScroll}
	class="min-h-0 flex-1 overflow-y-auto {compact
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
				{@const isActive = i === activeIndex}
				{@const isPast = i < activeIndex}
				<!-- Dimming is `opacity` on the line, never a translucent text colour. A colour with
				     alpha is composited glyph by glyph, so wherever two glyphs overlap the coverage
				     adds up and the overlap comes out brighter than the rest of the line: very
				     visible in scripts whose marks sit on top of the letters, Devanagari in issue
				     #191. `opacity` paints the line opaque first and fades it once, as a group.
				     Light themes dim less: the dark themes' 70% put dark grey on white at about 2.8:1,
				     90% matches the 4:1 the dark themes get. -->
				<button
					data-line={i}
					onclick={() => seekTo(line)}
					class="block w-full origin-left cursor-pointer text-left font-heading font-bold leading-snug transition-[color,opacity,transform] duration-300 ease-out hover:text-foreground hover:opacity-100 dark:hover:opacity-100
						{expanded ? 'py-3 text-[1.875em]' : compact ? 'py-1 text-[0.875em]' : 'py-2 text-[1.25em]'}
						{isActive
						? 'scale-[1.04] text-foreground'
						: isPast
							? 'text-muted-foreground opacity-55 dark:opacity-40'
							: 'text-muted-foreground opacity-90 dark:opacity-70'}"
				>
					{#if line.words && line.words.length > 0}
						{@render sweep(line.words, isActive)}
					{:else}
						<span>{line.text || '♪'}</span>
					{/if}

					{#if showRomanized && line.romanized}
						<!-- Relative size, so it follows the line from the mini player to theater mode.
						     Apple's reading is timed to the same syllables and sweeps with the line. -->
						<span class="mt-0.5 block text-[length:max(0.62em,11px)] font-semibold">
							{#if line.romanized_words && line.romanized_words.length > 0}
								{@render sweep(line.romanized_words, isActive)}
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
{#if track && !loading && !compact}
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

<!-- Word-by-word karaoke sweep (Better Lyrics style), for the line and for Apple's timed
     romanization under it. -->
{#snippet sweep(words: api.LyricWord[], active: boolean)}
	<span class="inline-flex flex-wrap items-baseline">
		{#each words as word, wIdx (wIdx)}
			{@const isWordEnd = word.text.endsWith(' ')}
			{@const cleanText = word.text.trimEnd()}
			{#if active}
				{@const progress = getWordProgress(word, posMs)}
				{@const pct = Math.round(Math.min(1, Math.max(0, progress)) * 100)}
				{@const isCurrentWord = progress > 0 && progress < 1}
				<!-- Only the gradient stop moves per frame; the clip/fill are static, so they
				     live in the class and aren't re-serialised 60 times a second. Both
				     colours are theme tokens: the sung half was hardcoded white, which is
				     invisible on every light theme. -->
				<span
					class="inline-block bg-clip-text text-transparent [-webkit-text-fill-color:transparent] transition-transform duration-100 ease-out {isWordEnd
						? 'mr-[0.26em]'
						: ''} {isCurrentWord ? 'scale-[1.03]' : ''}"
					style="background-image: linear-gradient(90deg, var(--foreground) {pct}%, var(--muted-foreground) {pct}%)"
				>
					{cleanText}
				</span>
			{:else}
				<!-- Colour and dimming both come from the line. -->
				<span class="inline-block {isWordEnd ? 'mr-[0.26em]' : ''}">
					{cleanText}
				</span>
			{/if}
		{/each}
	</span>
{/snippet}

