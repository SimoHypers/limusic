<script lang="ts">
	// The Appearance tab. A miniature of the app beside the controls shows the result before the app
	// takes it.
	//
	// Performance is the reason for the shape. Every theme token lives on <html>, and a custom
	// property written there restyles the whole document: 160-200 ms on WebKitGTK (#217). The old
	// tint and roundness sliders and the colour picker wrote on every pointer move, so a drag queued
	// dozens of those and the slider itself stuttered. Now a drag only writes `draft`, which the
	// miniatures pick up as inline vars on themselves, and the app gets one write when the pointer
	// lets go (`onValueCommit`, ColorPicker's `onchange`). A click on a swatch or a palette is one
	// write too.
	import { untrack, type Snippet } from 'svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { mode, setMode, userPrefersMode } from 'mode-watcher';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		Tick02Icon,
		Sun01Icon,
		Moon02Icon,
		ComputerIcon,
		PlusSignIcon,
		MinusSignIcon,
		ArrowTurnBackwardIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import * as Select from '$lib/components/ui/select';
	import * as Popover from '$lib/components/ui/popover';
	import ColorPicker from '$lib/components/ColorPicker.svelte';
	import ThemeMock from '$lib/components/ThemeMock.svelte';
	import { LEVELS as ZOOM_LEVELS, setZoom, zoom } from '$lib/zoom.svelte';
	import { playback, toast } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { appIcon, chooseAppIcon } from '$lib/appicon.svelte';
	import { t, type TranslationKey } from '$lib/i18n.svelte';
	import {
		THEMES,
		FONTS,
		theme,
		appearance,
		setAppearance,
		custom,
		effective,
		applyTheme,
		setCustom,
		syncMatugen,
		resetCustom,
		isDefaultCustom,
		readBack,
		familyName,
		fontAvailable,
		fileFonts,
		fileFamily,
		addFontFile,
		removeFontFile,
		registerFontFiles,
		onAccent,
		type Custom
	} from '$lib/theme.svelte';

	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';

	const isDark = $derived(mode.current === 'dark');

	// --- Live preview -----------------------------------------------------------------------------
	// What a drag in progress would set. Empty between drags; the miniatures read it over `custom`.
	let draft = $state<{ accent?: string; hue?: number; radius?: number }>({});
	const live = $derived({
		accent: draft.accent ?? custom.accent,
		hue: draft.hue ?? custom.hue,
		radius: draft.radius ?? custom.radius,
		fontSans: custom.fontSans,
		fontHeading: custom.fontHeading
	});

	function commit(patch: Partial<Custom>) {
		clearTimeout(commitTimer); // a click (reset, a swatch) must not be undone by a pending slider commit
		setCustom(patch);
		for (const k of Object.keys(patch)) delete draft[k as keyof typeof draft];
	}

	// Sliders commit through a short debounce. A pointer drag commits once anyway, but bits-ui
	// commits on every arrow key, and a held key repeats at ~30 Hz.
	let commitTimer: ReturnType<typeof setTimeout> | undefined;
	function commitSoon(patch: Partial<Custom>) {
		clearTimeout(commitTimer);
		commitTimer = setTimeout(() => commit(patch), 150);
	}

	// The playing track when there is one, so the preview is your app and not a sample of it. Same
	// stand-in as the Discord tab's preview otherwise. The 120px cover is the one the player bar has
	// already loaded.
	const hour = new Date().getHours();
	const daypart: TranslationKey =
		hour < 5
			? 'home.good_night'
			: hour < 12
				? 'home.good_morning'
				: hour < 18
					? 'home.good_afternoon'
					: 'home.good_evening';
	const detail = $derived({
		heading: t(daypart),
		title: playback.now?.title ?? 'Higher Ground',
		artist: playback.now?.artists ?? 'Nova Sky',
		play: t('common.play_all'),
		cover: thumb(playback.now?.thumbnail, 120)
	});

	// --- Colours ----------------------------------------------------------------------------------
	// Picked to hold up as a fill under white text in both modes (all of them get the light
	// foreground from `onAccent`). Anything else is one click away in the custom picker.
	const ACCENTS = [
		'#e11d48',
		'#ea580c',
		'#d97706',
		'#16a34a',
		'#0d9488',
		'#2563eb',
		'#7c3aed',
		'#db2777'
	];
	const accentNow = $derived(live.accent?.toLowerCase() ?? null);
	const customAccent = $derived(accentNow && !ACCENTS.includes(accentNow) ? accentNow : null);
	let pickerOpen = $state(false);

	// Muted on purpose: the tint is a shade of grey, and a full-saturation rainbow under it promised
	// colours the setting never paints. Stops every 30 degrees, since oklch gradient interpolation
	// is too new to rely on in every webview this ships on.
	const TINT_TRACK = `linear-gradient(to right, ${Array.from(
		{ length: 13 },
		(_, i) => `oklch(0.72 0.1 ${i * 30})`
	).join(', ')})`;

	// --- Light / dark -----------------------------------------------------------------------------
	const MODES = $derived([
		{ id: 'light', label: t('settings.themes.mode_light'), icon: Sun01Icon },
		{ id: 'dark', label: t('settings.themes.mode_dark'), icon: Moon02Icon },
		{ id: 'system', label: t('settings.themes.mode_system'), icon: ComputerIcon }
	] as const);

	// --- Scale ------------------------------------------------------------------------------------
	const zoomIndex = $derived(Math.max(0, ZOOM_LEVELS.indexOf(zoom.level)));
	const pct = (level: number) => `${Math.round(level * 100)}%`;

	// --- Fonts ------------------------------------------------------------------------------------
	type FontKey = 'fontSans' | 'fontHeading';
	const FONT_ROWS: { key: FontKey; label: string; hint: string }[] = $derived([
		{
			key: 'fontSans',
			label: t('settings.themes.interface_font_label'),
			hint: t('settings.themes.interface_font_short_hint')
		},
		{
			key: 'fontHeading',
			label: t('settings.themes.heading_font_label'),
			hint: t('settings.themes.heading_font_short_hint')
		}
	]);
	/** Which entry in the font dropdown a resolved stack corresponds to. */
	const fontOptions = $derived([...FONTS, ...fileFonts()]);
	const matchFont = (stack: string) =>
		fontOptions.find((f) => familyName(f.value) === familyName(stack))?.value ?? 'custom';

	// Whether each font row is on "Custom", and the family name typed into it. Kept locally because
	// the select can sit on Custom before anything has been typed.
	let isCustomFont = $state<Record<FontKey, boolean>>({ fontSans: false, fontHeading: false });
	let fontName = $state<Record<FontKey, string>>({ fontSans: '', fontHeading: '' });

	// Once per mount, which is once per visit to the tab: the dialog unmounts its content on close.
	untrack(() => {
		readBack();
		// Catches a font deleted while the app was running, not just between launches.
		registerFontFiles();
		for (const key of ['fontSans', 'fontHeading'] as FontKey[]) {
			isCustomFont[key] = matchFont(effective[key]) === 'custom';
			fontName[key] = isCustomFont[key] ? familyName(effective[key]) : '';
		}
	});

	function chooseFont(key: FontKey, value: string) {
		isCustomFont[key] = value === 'custom';
		if (value === 'custom') fontName[key] = familyName(effective[key]);
		else setCustom({ [key]: value } as Partial<Custom>);
	}

	// Applying a font family rewrites --font-sans/--font-heading on <html>, which restyles and
	// reflows the whole app (and `apply` then re-reads the computed tokens). Doing that per
	// keystroke is what made typing a font name lag (#97), so the input updates immediately and the
	// theme follows once typing pauses. Half-typed names are meaningless anyway.
	const fontTimers: Record<FontKey, ReturnType<typeof setTimeout> | undefined> = {
		fontSans: undefined,
		fontHeading: undefined
	};

	function typeFont(key: FontKey, name: string) {
		fontName[key] = name;
		clearTimeout(fontTimers[key]);
		fontTimers[key] = setTimeout(() => {
			// Blank clears the override, so the preset's font comes back.
			setCustom({ [key]: name.trim() ? `'${name.trim()}', sans-serif` : null } as Partial<Custom>);
		}, 300);
	}

	async function pickFontFiles() {
		const picked = await open({
			multiple: true,
			title: t('settings.themes.load_font_dialog'),
			filters: [{ name: t('settings.themes.font_filter'), extensions: ['ttf', 'otf', 'woff', 'woff2'] }]
		});
		for (const path of picked ?? []) {
			try {
				toast.success(t('toasts.font_loaded', { name: await addFontFile(path) }));
			} catch (e) {
				toast.error(String(e));
			}
		}
	}

	// --- App icon ---------------------------------------------------------------------------------
	async function pickAppIcon() {
		try {
			const picked = await open({
				title: t('settings.themes.app_icon_dialog'),
				filters: [{ name: t('settings.themes.app_icon_filter'), extensions: ['png'] }]
			});
			if (typeof picked !== 'string') return;
			await chooseAppIcon(picked);
			toast.success(t('toasts.app_icon_set'));
		} catch (e) {
			toast.error(String(e));
		}
	}

	async function resetAppIcon() {
		try {
			await chooseAppIcon(null);
		} catch (e) {
			toast.error(String(e));
		}
	}

	function resetAll() {
		clearTimeout(commitTimer);
		resetCustom();
		draft = {};
		isCustomFont = { fontSans: false, fontHeading: false };
		fontName = { fontSans: '', fontHeading: '' };
	}
</script>

<!-- Same row as the Discord tab: the description runs the full width under the title and control,
     because this column shares the dialog with the preview. -->
{#snippet row(o: {
	title: string;
	desc?: string;
	badge?: string;
	control?: Snippet;
	below?: Snippet;
})}
	<div class="px-4 py-3.5">
		<div class="flex min-h-8 items-center justify-between gap-4">
			<div class="flex min-w-0 items-center gap-2">
				<span class="text-sm font-medium">{o.title}</span>
				{#if o.badge}
					<span
						class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary"
					>
						{o.badge}
					</span>
				{/if}
			</div>
			{#if o.control}
				<div class="shrink-0">{@render o.control()}</div>
			{/if}
		</div>
		{#if o.desc}
			<p class="mt-1 max-w-prose text-xs leading-relaxed text-muted-foreground">{o.desc}</p>
		{/if}
		{#if o.below}
			<div class="mt-3">{@render o.below()}</div>
		{/if}
	</div>
{/snippet}

<!-- A small reset beside a control that has been moved off the theme's value. Kept in the layout
     (invisible) when there is nothing to reset, so the row does not shift when it appears. -->
{#snippet resetIcon(on: boolean, onclick: () => void)}
	<Button
		variant="ghost"
		size="icon-sm"
		class="text-muted-foreground {on ? '' : 'invisible'}"
		aria-label={t('common.reset')}
		title={t('common.reset')}
		{onclick}
	>
		<HugeiconsIcon icon={ArrowTurnBackwardIcon} size={15} strokeWidth={2} />
	</Button>
{/snippet}

<!-- Options scroll, the preview does not. Side by side from `lg` up; below it the preview becomes a
     strip above the options, since under the options it would be out of sight while dragging. -->
<div class="flex min-h-0 flex-1 max-lg:flex-col-reverse">
	<!-- relative: see the content pane in SettingsDialog (#406). -->
	<div class="relative min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-6 py-5">
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.sections.theme')}</h3>
			<div class={CARD}>
				<!-- Chips, not miniatures: the preview beside this column already shows the chosen
				     palette in full. Each swatch carries the palette's own classes, so it is the
				     preset's background and accent whatever is applied on top. -->
				<div class="flex flex-wrap gap-2 p-4" role="group" aria-label={t('a11y.theme')}>
					{#each THEMES as th (th.id)}
						<button
							type="button"
							onclick={() => applyTheme(th.id)}
							aria-pressed={theme.id === th.id}
							class="flex cursor-pointer items-center gap-2 rounded-full border py-1.5 pr-3.5 pl-2 text-sm transition-colors {theme.id ===
							th.id
								? 'border-primary bg-primary/10 font-medium'
								: 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
						>
							<span
								class="theme-{th.id} {isDark
									? 'dark'
									: ''} size-4 shrink-0 rounded-full bg-[linear-gradient(135deg,var(--background)_50%,var(--primary)_50%)] ring-1 ring-foreground/20"
							></span>
							{th.label}
						</button>
					{/each}
				</div>
				{@render row({
					title: t('settings.themes.mode'),
					desc: t('settings.themes.mode_hint'),
					below: modePicker
				})}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.sections.colors')}</h3>
			<div class={CARD}>
				{@render row({
					title: t('settings.themes.primary_color'),
					desc: t('settings.themes.accent_hint'),
					below: accentSwatches
				})}
				{#if theme.id === 'default'}
					{@render row({
						title: t('settings.themes.background_color'),
						desc: t('settings.themes.tint_hint'),
						control: tintReset,
						below: tintSlider
					})}
				{:else}
					{@render row({
						title: t('settings.themes.background_color'),
						desc: t('settings.themes.tint_palette_hint', {
							theme: THEMES.find((x) => x.id === theme.id)?.label ?? ''
						})
					})}
				{/if}
				{@render row({
					title: t('settings.themes.artwork_accent'),
					badge: t('settings.themes.experimental'),
					desc: t('settings.themes.artwork_accent_hint'),
					control: artworkAccentSwitch
				})}
				{@render row({
					title: t('settings.themes.matugen'),
					desc: t('settings.themes.matugen_hint'),
					control: matugenSwitch
				})}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.sections.interface')}</h3>
			<div class={CARD}>
				{@render row({
					title: t('settings.themes.roundness'),
					desc: t('settings.themes.roundness_hint'),
					control: radiusValue,
					below: radiusSlider
				})}
				{@render row({
					title: t('settings.themes.zoom'),
					desc: t('settings.themes.zoom_hint'),
					control: zoomStepper
				})}
				{@render row({
					title: t('settings.themes.app_icon'),
					desc: t('settings.themes.app_icon_hint'),
					control: appIconButtons
				})}
				{@render row({
					title: t('settings.themes.blur_playlist_header'),
					desc: t('settings.themes.blur_playlist_header_hint'),
					control: blurHeaderSwitch
				})}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.sections.typography')}</h3>
			<div class={CARD}>
				{#each FONT_ROWS as fr (fr.key)}
					<!-- Zero-arg wrappers: a snippet passed as a value can't carry arguments. -->
					{#snippet pick()}{@render fontSelect(fr.key, fr.label)}{/snippet}
					{#snippet type()}{@render fontInput(fr.key, fr.label)}{/snippet}
					{@render row({
						title: fr.label,
						desc: fr.hint,
						control: pick,
						below: isCustomFont[fr.key] ? type : undefined
					})}
				{/each}
				{@render row({
					title: t('settings.themes.load_font_file'),
					desc: t('settings.themes.load_font_file_hint'),
					control: addFontButton,
					below: custom.fontFiles.length ? fontFileList : undefined
				})}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.sections.player_view')}</h3>
			<div class={CARD}>
				{@render row({
					title: t('settings.themes.classic_player'),
					desc: t('settings.themes.classic_player_hint'),
					control: classicPlayerSwitch
				})}
				{@render row({
					title: t('settings.themes.open_player'),
					desc: t('settings.themes.open_player_hint'),
					control: openPlayerSwitch
				})}
				{@render row({
					title: t('settings.themes.tabbed_player'),
					desc: t('settings.themes.tabbed_player_hint'),
					control: tabbedSwitch
				})}
				{@render row({
					title: t('settings.themes.artwork_background'),
					desc: t('settings.themes.artwork_background_hint'),
					control: artworkBgSwitch
				})}
			</div>
		</section>
	</div>

	<aside
		class="flex shrink-0 gap-3 bg-muted/30 px-5 py-5 lg:w-[22rem] lg:flex-col lg:border-l max-lg:items-center max-lg:border-b max-lg:py-3"
	>
		<h3 class="{LABEL} max-lg:hidden">{t('settings.themes.preview')}</h3>
		<div class="max-lg:w-52 max-lg:shrink-0">
			<ThemeMock id={theme.id} dark={isDark} overrides={live} {detail} />
		</div>
		<div class="flex min-w-0 flex-col gap-3 lg:flex-1">
			<p class="text-xs leading-relaxed text-muted-foreground">{t('settings.themes.preview_hint')}</p>
			<div class="flex flex-col items-start gap-1.5 lg:mt-auto">
				<Button variant="outline" size="sm" disabled={isDefaultCustom()} onclick={resetAll}>
					<HugeiconsIcon icon={ArrowTurnBackwardIcon} size={15} strokeWidth={2} />
					{t('common.reset')}
				</Button>
				<p class="text-xs leading-relaxed text-muted-foreground max-lg:hidden">
					{t('settings.themes.reset_theme_hint')}
				</p>
			</div>
		</div>
	</aside>
</div>

<!-- Segmented, like audio quality: one exclusive choice. Under the description rather than beside
     the title, so a wide font or a long translation doesn't squeeze the title onto two lines. -->
{#snippet modePicker()}
	<div class="inline-flex rounded-lg bg-muted p-0.5">
		{#each MODES as m (m.id)}
			<button
				type="button"
				onclick={() => setMode(m.id)}
				aria-pressed={userPrefersMode.current === m.id}
				class="flex cursor-pointer items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors {userPrefersMode.current ===
				m.id
					? 'bg-background text-foreground shadow-sm'
					: 'text-muted-foreground hover:text-foreground'}"
			>
				<HugeiconsIcon icon={m.icon} size={14} strokeWidth={2} />
				{m.label}
			</button>
		{/each}
	</div>
{/snippet}

{#snippet accentSwatches()}
	<div class="flex flex-wrap items-center gap-2">
		<!-- The palette's own accent: the swatch carries the palette's classes and reads --primary
		     there, so it shows the preset's colour even while a custom one is applied. -->
		<button
			type="button"
			disabled={appearance.matugen}
			onclick={() => commit({ accent: null })}
			aria-pressed={!accentNow}
			aria-label={t('settings.themes.accent_theme')}
			title={t('settings.themes.accent_theme')}
			class="theme-{theme.id} {isDark
				? 'dark'
				: ''} flex size-7 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground ring-offset-2 ring-offset-card {accentNow
				? ''
				: 'ring-2 ring-foreground/60'}"
		>
			{#if !accentNow}
				<HugeiconsIcon icon={Tick02Icon} size={14} strokeWidth={2.6} />
			{/if}
		</button>
		<span class="mx-0.5 h-5 w-px bg-border"></span>
		{#each ACCENTS as hex (hex)}
			<button
				type="button"
				disabled={appearance.matugen}
				onclick={() => commit({ accent: hex })}
				aria-pressed={accentNow === hex}
				aria-label="{t('a11y.choose_accent')}: {hex}"
				title={hex}
				class="flex size-7 cursor-pointer items-center justify-center rounded-full ring-offset-2 ring-offset-card {accentNow ===
				hex
					? 'ring-2 ring-foreground/60'
					: ''}"
				style="background:{hex}; color:{onAccent(hex)}"
			>
				{#if accentNow === hex}
					<HugeiconsIcon icon={Tick02Icon} size={14} strokeWidth={2.6} />
				{/if}
			</button>
		{/each}
		<Popover.Root
			bind:open={pickerOpen}
			onOpenChange={(o) => {
				if (o) readBack();
				else draft.accent = undefined;
			}}
		>
			<!-- A hue wheel until a colour of your own is picked, then that colour. -->
			<Popover.Trigger
				disabled={appearance.matugen}
				aria-label={t('a11y.choose_accent')}
				title={t('a11y.choose_accent')}
				class="flex size-7 cursor-pointer items-center justify-center rounded-full ring-offset-2 ring-offset-card {customAccent ||
				pickerOpen
					? 'ring-2 ring-foreground/60'
					: ''}"
				style={customAccent
					? `background:${customAccent}; color:${onAccent(customAccent)}`
					: 'background:conic-gradient(#f00,#ff0,#0f0,#0ff,#00f,#f0f,#f00); color:#fff'}
			>
				{#if customAccent}
					<HugeiconsIcon icon={Tick02Icon} size={14} strokeWidth={2.6} />
				{:else}
					<HugeiconsIcon icon={PlusSignIcon} size={14} strokeWidth={2.6} class="drop-shadow-sm" />
				{/if}
			</Popover.Trigger>
			<Popover.Content side="bottom" align="end" class="w-auto p-3">
				<ColorPicker
					value={custom.accent ?? effective.accent}
					oninput={(hex) => (draft.accent = hex)}
					onchange={(hex) => commit({ accent: hex })}
				/>
			</Popover.Content>
		</Popover.Root>
	</div>
	{#if appearance.artworkAccent && !appearance.matugen && playback.now}
		<p class="mt-3 max-w-prose text-xs leading-relaxed text-muted-foreground">
			{t('settings.themes.accent_artwork_note')}
		</p>
	{/if}
{/snippet}

{#snippet tintReset()}{@render resetIcon(custom.hue !== null, () => commit({ hue: null }))}{/snippet}

{#snippet tintSlider()}
	<Slider
		type="single"
		aria-label={t('a11y.background_tint')}
		max={360}
		step={1}
		value={draft.hue ?? effective.hue}
		onValueChange={(hue) => (draft.hue = hue)}
		onValueCommit={(hue) => commitSoon({ hue })}
		class="[&_[data-slot=slider-range]]:bg-transparent [&_[data-slot=slider-track]]:bg-(image:--tint-track)"
		style="--tint-track:{TINT_TRACK}"
	/>
{/snippet}

{#snippet radiusValue()}
	<div class="flex items-center gap-1">
		<span class="font-mono text-xs text-muted-foreground tabular-nums">
			{Math.round((draft.radius ?? effective.radius) * 16)}px
		</span>
		{@render resetIcon(custom.radius !== null, () => commit({ radius: null }))}
	</div>
{/snippet}

{#snippet radiusSlider()}
	<Slider
		type="single"
		aria-label={t('a11y.roundness')}
		max={1.5}
		step={0.05}
		value={draft.radius ?? effective.radius}
		onValueChange={(radius) => (draft.radius = radius)}
		onValueCommit={(radius) => commitSoon({ radius })}
	/>
{/snippet}

<!-- A stepper rather than a dropdown: the next size up or down is the usual change, and the middle
     shows where you are. Clicking the number goes back to 100%. -->
{#snippet zoomStepper()}
	<div class="flex items-center rounded-lg bg-muted p-0.5">
		<button
			type="button"
			onclick={() => setZoom(ZOOM_LEVELS[zoomIndex - 1])}
			disabled={zoomIndex === 0}
			aria-label={t('dialogs.shortcuts.zoom_out')}
			class="flex size-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
		>
			<HugeiconsIcon icon={MinusSignIcon} size={14} strokeWidth={2.2} />
		</button>
		<button
			type="button"
			onclick={() => setZoom(1)}
			aria-label="{t('a11y.zoom')}: {pct(zoom.level)}"
			title={t('dialogs.shortcuts.reset_zoom')}
			class="h-7 w-14 cursor-pointer rounded-md text-xs font-medium tabular-nums transition-colors hover:bg-background"
		>
			{pct(zoom.level)}
		</button>
		<button
			type="button"
			onclick={() => setZoom(ZOOM_LEVELS[zoomIndex + 1])}
			disabled={zoomIndex === ZOOM_LEVELS.length - 1}
			aria-label={t('dialogs.shortcuts.zoom_in')}
			class="flex size-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
		>
			<HugeiconsIcon icon={PlusSignIcon} size={14} strokeWidth={2.2} />
		</button>
	</div>
{/snippet}

{#snippet artworkAccentSwitch()}<Switch
		checked={appearance.artworkAccent}
		disabled={appearance.matugen}
		onCheckedChange={(on) => setAppearance({ artworkAccent: on })}
	/>{/snippet}
{#snippet matugenSwitch()}<Switch
		checked={appearance.matugen}
		onCheckedChange={async (on) => {
			pickerOpen = false;
			setAppearance({ matugen: on });
			await syncMatugen();
		}}
	/>{/snippet}
{#snippet blurHeaderSwitch()}<Switch
		checked={appearance.blurPlaylistHeader}
		onCheckedChange={(on) => setAppearance({ blurPlaylistHeader: on })}
	/>{/snippet}
{#snippet classicPlayerSwitch()}<Switch
		checked={appearance.classicPlayer}
		onCheckedChange={(on) => setAppearance({ classicPlayer: on })}
	/>{/snippet}
{#snippet openPlayerSwitch()}<Switch
		checked={appearance.openPlayerOnPlay}
		onCheckedChange={(on) => setAppearance({ openPlayerOnPlay: on })}
	/>{/snippet}
{#snippet tabbedSwitch()}<Switch
		checked={appearance.tabbedPlayer}
		onCheckedChange={(on) => setAppearance({ tabbedPlayer: on })}
	/>{/snippet}
{#snippet artworkBgSwitch()}<Switch
		checked={appearance.artworkBackground}
		onCheckedChange={(on) => setAppearance({ artworkBackground: on })}
	/>{/snippet}

{#snippet fontSelect(key: FontKey, label: string)}
	<Select.Root
		type="single"
		value={isCustomFont[key] ? 'custom' : matchFont(effective[key])}
		onValueChange={(v) => chooseFont(key, v)}
	>
		<Select.Trigger class="w-44 shrink-0" aria-label={label}>
			<span class="min-w-0 flex-1 truncate text-left" style="font-family:{effective[key]}">
				{isCustomFont[key] ? t('common.custom') : familyName(effective[key])}
			</span>
		</Select.Trigger>
		<!-- max-w: a loaded font's name is whatever the file was called, and the dropdown grows to
		     its widest item. -->
		<Select.Content class="max-w-64">
			{#each FONTS as f (f.value)}
				<Select.Item value={f.value} label={f.label}>
					<span class="block truncate" style="font-family:{f.value}">{f.label}</span>
				</Select.Item>
			{/each}
			{#if custom.fontFiles.length}
				<Select.Group>
					<Select.GroupHeading>{t('settings.themes.your_fonts')}</Select.GroupHeading>
					{#each fileFonts() as f (f.value)}
						<Select.Item value={f.value} label={f.label}>
							<span class="block truncate" style="font-family:{f.value}">{f.label}</span>
						</Select.Item>
					{/each}
				</Select.Group>
			{/if}
			<Select.Item value="custom" label={t('common.custom')}>{t('settings.themes.custom_font')}</Select.Item>
		</Select.Content>
	</Select.Root>
{/snippet}

{#snippet fontInput(key: FontKey, label: string)}
	<Input
		value={fontName[key]}
		oninput={(e) => typeFont(key, e.currentTarget.value)}
		placeholder={t('settings.themes.font_placeholder')}
		aria-label={t('settings.themes.font_aria', { label })}
		spellcheck={false}
		style="font-family:{effective[key]}"
	/>
	<!-- Probes the *applied* family, not the half-typed one: measuring a font on every keystroke is
	     the other half of #97, and a name mid-typing is never installed anyway. -->
	{#if fontName[key].trim() && !fontAvailable(familyName(effective[key]))}
		<p class="mt-1.5 text-xs text-muted-foreground">
			{t('settings.themes.font_not_installed')}
		</p>
	{/if}
{/snippet}

{#snippet addFontButton()}
	<Button variant="outline" size="sm" class="shrink-0" onclick={pickFontFiles}>{t('settings.themes.add_font')}</Button>
{/snippet}

{#snippet fontFileList()}
	<div class="flex flex-col gap-1.5">
		{#each custom.fontFiles as path (path)}
			<div class="flex items-center gap-3 rounded-lg bg-secondary/60 py-1.5 pr-1.5 pl-3 text-sm">
				<!-- The name is the identity; the path only earns a tooltip. A font called
				     BigBlueTerm437NerdFontMono-Regular is wider than the modal. -->
				<span class="min-w-0 flex-1 truncate" style="font-family:'{fileFamily(path)}'" title={path}>
					{fileFamily(path)}
				</span>
				<button
					type="button"
					onclick={() => removeFontFile(path)}
					aria-label={t('a11y.remove_font', { name: fileFamily(path) })}
					class="flex size-6 shrink-0 cursor-pointer items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
				>
					<HugeiconsIcon icon={Cancel01Icon} size={14} />
				</button>
			</div>
		{/each}
	</div>
{/snippet}

{#snippet appIconButtons()}
	<div class="flex shrink-0 items-center gap-2">
		<img src={appIcon.src} alt="" class="size-7 rounded" />
		<Button variant="outline" size="sm" onclick={pickAppIcon}>{t('settings.themes.app_icon_pick')}</Button>
		<Button variant="ghost" size="sm" onclick={resetAppIcon}>{t('common.reset')}</Button>
	</div>
{/snippet}
