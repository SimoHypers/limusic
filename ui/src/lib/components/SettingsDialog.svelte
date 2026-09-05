<script lang="ts">
	import { untrack, type Snippet } from 'svelte';
	import { open, save } from '@tauri-apps/plugin-dialog';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		Settings02Icon,
		PaintBoardIcon,
		PlayCircleIcon,
		Database02Icon,
		InformationCircleIcon,
		KeyboardIcon,
		Cancel01Icon as RemoveIcon,
		Copy01Icon,
		Coffee02Icon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import { Alert, AlertDescription } from '$lib/components/ui/alert';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as Select from '$lib/components/ui/select';
	import { MOD } from '$lib/shortcuts';
	import { copyText } from '$lib/clipboard';
	import * as api from '$lib/api';
	import { blocked, prefs, ui, toast, unblockArtist } from '$lib/player.svelte';
	import ColorPicker from '$lib/components/ColorPicker.svelte';
	import Changelog from '$lib/components/Changelog.svelte';
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
		type Custom,
		type ThemeId
	} from '$lib/theme.svelte';
	import {
		updateState,
		checkForUpdatesInteractive,
		installUpdate,
		openDownloadPage
	} from '$lib/updater.svelte';
	import { getVersion } from '@tauri-apps/api/app';
	import { t, setLocale, currentLocale, LOCALES, type LocaleId } from '$lib/i18n.svelte';
	import { appIcon, chooseAppIcon } from '$lib/appicon.svelte';

	type TabId = 'general' | 'themes' | 'playback' | 'data' | 'about';
	const TABS = $derived<{ id: TabId; label: string; hint: string; icon: typeof Settings02Icon }[]>([
		{ id: 'general', label: t('settings.tabs.general'), hint: t('settings.tabs.general_hint'), icon: Settings02Icon },
		{ id: 'themes', label: t('settings.tabs.themes'), hint: t('settings.tabs.themes_hint'), icon: PaintBoardIcon },
		{ id: 'playback', label: t('settings.tabs.playback'), hint: t('settings.tabs.playback_hint'), icon: PlayCircleIcon },
		{ id: 'data', label: t('settings.tabs.data'), hint: t('settings.tabs.data_hint'), icon: Database02Icon },
		{ id: 'about', label: t('settings.tabs.about'), hint: t('settings.tabs.about_hint'), icon: InformationCircleIcon }
	]);

	// Shared shapes for the settings rows. Kept as strings so the markup below stays readable and
	// every group looks identical without a wrapper component per row.
	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';

	const ACCENT_THEMES = THEMES.filter((t) => t.kind === 'accent');
	const PALETTE_THEMES = THEMES.filter((t) => t.kind === 'palette');
	const currentTheme = $derived(THEMES.find((t) => t.id === theme.id) ?? THEMES[0]);

	// --- Themes tab ---
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
	let pickerOpen = $state(false);
	// Whether each font row is on "Custom", and the family name typed into it. Kept locally because
	// the select can sit on Custom before anything has been typed.
	let isCustomFont = $state<Record<FontKey, boolean>>({ fontSans: false, fontHeading: false });
	let fontName = $state<Record<FontKey, string>>({ fontSans: '', fontHeading: '' });

	/** Which entry in the font dropdown a resolved stack corresponds to. */
	const fontOptions = $derived([...FONTS, ...fileFonts()]);
	const matchFont = (stack: string) =>
		fontOptions.find((f) => familyName(f.value) === familyName(stack))?.value ?? 'custom';

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

	let tab = $state<TabId>('general');
	const currentTab = $derived(TABS.find((tb) => tb.id === tab) ?? TABS[0]);
	const shortcutsHint = $derived(t('settings.general.shortcuts_hint').split('{key}'));
	const currentLocaleLabel = $derived(
		LOCALES.find((l) => l.id === currentLocale.id)?.nativeLabel ?? currentLocale.id
	);
	let settings = $state<Record<string, string>>({});
	let clients = $state<string[]>([]);
	let proxyInput = $state('');
	/// How many blocked artists the section shows before the "show all" toggle. The list is never
	/// truncated, only collapsed: a long one would otherwise push Lyrics and Advanced off the tab.
	const BLOCKED_PREVIEW = 5;
	let showAllBlocked = $state(false);
	/// Export: the stored value verbatim, so it can be pasted into another player or back into a
	/// fresh install. No file format for a list of a dozen names.
	async function copyBlocked() {
		try {
			await copyText(JSON.stringify(blocked.artists, null, 2));
			toast(t('toasts.blocked_copied', { count: blocked.artists.length }));
		} catch {
			toast(t('toasts.could_not_copy_link'));
		}
	}
	let loaded = $state(false);
	let clearing = $state(false);
	let version = $state('');
	getVersion().then((v) => (version = v));
	// Result of the last "Check for updates" click — shown inline (a toast renders behind the modal).
	let updateResult = $state<{ message: string; error: boolean } | null>(null);

	// (Re)load whenever the modal opens, so it reflects the current persisted values. Also clear the
	// stale update-check result so re-opening the modal doesn't show it until pressed again.
	// untrack: this reads and writes theme state, and `registerFontFiles` can rewrite it again when
	// it prunes a deleted font. Opening the modal is the only thing that should run it.
	$effect(() => {
		if (!ui.settingsOpen) return;
		untrack(() => {
			load();
			updateResult = null;
			pickerOpen = false;
			readBack();
			// Catches a font deleted while the app was running, not just between launches.
			registerFontFiles();
			for (const key of ['fontSans', 'fontHeading'] as FontKey[]) {
				isCustomFont[key] = matchFont(effective[key]) === 'custom';
				fontName[key] = isCustomFont[key] ? familyName(effective[key]) : '';
			}
		});
	});

	async function checkUpdates() {
		updateResult = await checkForUpdatesInteractive();
	}

	// Diagnostics. Toasts render behind this modal, so the buttons report on themselves.
	let diagState = $state<'idle' | 'busy' | 'copied' | 'saved'>('idle');
	let diagError = $state('');

	function flash(kind: 'copied' | 'saved') {
		diagState = kind;
		setTimeout(() => (diagState = 'idle'), 2500);
	}

	async function copyDiagnostics() {
		diagError = '';
		diagState = 'busy';
		try {
			await copyText(await api.diagnostics());
			flash('copied');
		} catch (e) {
			diagState = 'idle';
			diagError = String(e);
		}
	}

	async function saveDiagnostics() {
		diagError = '';
		try {
			const path = await save({
				defaultPath: `limusic-diagnostics-${new Date().toISOString().slice(0, 10)}.txt`,
				filters: [{ name: 'Text', extensions: ['txt'] }]
			});
			if (!path) return;
			diagState = 'busy';
			await api.saveDiagnostics(path);
			flash('saved');
		} catch (e) {
			diagState = 'idle';
			diagError = String(e);
		}
	}

	async function openBugForm() {
		diagError = '';
		try {
			// GitHub's prefill only reaches `input` and `textarea` fields, so the "Which system?"
			// dropdown stays the user's one click and everything the app knows goes in `system`.
			const system = await api.diagnosticsSummary();
			const q = new URLSearchParams({
				template: 'bug_report.yml',
				version,
				system
			});
			await api.openExternal(`https://github.com/SimoHypers/limusic/issues/new?${q}`);
		} catch (e) {
			diagError = String(e);
		}
	}

	async function load() {
		try {
			const [s, c] = await Promise.all([api.getSettings(), api.getStreamClients()]);
			settings = s;
			clients = c;
			proxyInput = s.proxy ?? '';
		} catch (e) {
			toast.error(String(e));
		}
		loaded = true;
	}

	const quality = $derived(settings.quality ?? 'HIGH');
	const historyOn = $derived(settings.enable_history !== 'false');
	const autoplayOn = $derived(settings.autoplay !== 'false');
	const hideVideosOn = $derived(settings.hide_videos === 'true');
	// Off until the setting is turned on: still experimental, so nobody gets video they didn't ask
	// for. Same test in `player.svelte.ts`, which hydrates `prefs` at launch.
	const musicVideosOn = $derived(settings.music_videos === 'true');
	const boiduOn = $derived(settings.lyrics_boidu !== 'false');
	const preventDuplicatesOn = $derived(settings.prevent_duplicates === 'true');
	// Off by default: shuffle applies to the queue it was turned on for (issue #117).
	const stickyShuffleOn = $derived(settings.sticky_shuffle === 'true');
	const updateBannerOn = $derived(settings.update_banner !== 'false');
	const discordOn = $derived(settings.discord_rpc === 'true');
	const trayOn = $derived(settings.close_to_tray !== 'false');
	const autostartOn = $derived(settings.autostart === 'true');
	const disabled = $derived(
		new Set(
			(settings.disabled_stream_clients ?? '')
				.split(',')
				.map((s) => s.trim())
				.filter(Boolean)
		)
	);

	const QUALITIES = [
		{ id: 'LOW', key: 'settings.playback.quality_low' },
		{ id: 'AUTO', key: 'settings.playback.quality_auto' },
		{ id: 'HIGH', key: 'settings.playback.quality_high' }
	] as const;

	async function setQuality(q: string) {
		settings.quality = q;
		await api.setSetting('quality', q);
		// Cached URLs are keyed by video only, so clear them to apply the new quality everywhere.
		await api.clearCaches();
		toast.success(t('toasts.quality_updated'));
	}

	async function setHistory(on: boolean) {
		settings.enable_history = on ? 'true' : 'false';
		await api.setSetting('enable_history', settings.enable_history);
	}

	async function setAutoplay(on: boolean) {
		settings.autoplay = on ? 'true' : 'false';
		await api.setSetting('autoplay', settings.autoplay);
	}

	// Also lands in `prefs`, which is where the player view reads it: the switch has to take effect
	// on the track that's already playing, not on the next launch.
	async function setMusicVideos(on: boolean) {
		settings.music_videos = on ? 'true' : 'false';
		prefs.musicVideos = on;
		await api.setSetting('music_videos', settings.music_videos);
	}

	async function setHideVideos(on: boolean) {
		settings.hide_videos = on ? 'true' : 'false';
		await api.setSetting('hide_videos', settings.hide_videos);
	}

	async function setBoidu(on: boolean) {
		settings.lyrics_boidu = on ? 'true' : 'false';
		await api.setSetting('lyrics_boidu', settings.lyrics_boidu);
	}

	async function setPreventDuplicates(on: boolean) {
		settings.prevent_duplicates = on ? 'true' : 'false';
		await api.setSetting('prevent_duplicates', settings.prevent_duplicates);
	}

	async function setStickyShuffle(on: boolean) {
		settings.sticky_shuffle = on ? 'true' : 'false';
		await api.setSetting('sticky_shuffle', settings.sticky_shuffle);
	}

	async function setUpdateBanner(on: boolean) {
		settings.update_banner = on ? 'true' : 'false';
		await api.setSetting('update_banner', settings.update_banner);
	}

	async function setDiscord(on: boolean) {
		settings.discord_rpc = on ? 'true' : 'false';
		await api.setSetting('discord_rpc', settings.discord_rpc);
	}

	async function setTray(on: boolean) {
		settings.close_to_tray = on ? 'true' : 'false';
		await api.setSetting('close_to_tray', settings.close_to_tray);
	}

	async function setAutostart(on: boolean) {
		settings.autostart = on ? 'true' : 'false';
		try {
			await api.setSetting('autostart', settings.autostart);
		} catch (e) {
			settings.autostart = on ? 'false' : 'true'; // registration failed — revert the switch
			toast.error(String(e));
		}
	}

	async function toggleClient(name: string) {
		const set = new Set(disabled);
		if (set.has(name)) set.delete(name);
		else set.add(name);
		settings.disabled_stream_clients = [...set].join(',');
		await api.setSetting('disabled_stream_clients', settings.disabled_stream_clients);
	}

	async function saveProxy() {
		settings.proxy = proxyInput.trim();
		await api.setSetting('proxy', settings.proxy);
		toast.success(t('toasts.proxy_saved'));
	}

	async function doClearCaches() {
		clearing = true;
		try {
			await api.clearCaches();
			toast.success(t('toasts.caches_cleared'));
		} finally {
			clearing = false;
		}
	}
</script>

<!-- One row shape for the whole modal: label and description on the left, the control on the right,
     and an optional block underneath for the things that expand (color picker, font input, lists). -->
{#snippet row(o: {
	title: string;
	desc?: string;
	badge?: string;
	control?: Snippet;
	below?: Snippet;
	tall?: boolean;
})}
	<div class="px-4 py-3.5">
		<div class="flex {o.tall ? 'items-start' : 'items-center'} justify-between gap-6">
			<div class="min-w-0">
				<div class="flex items-center gap-2">
					<span class="text-sm font-medium">{o.title}</span>
					{#if o.badge}
						<span
							class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary"
						>
							{o.badge}
						</span>
					{/if}
				</div>
				{#if o.desc}
					<p class="mt-1 max-w-prose text-xs leading-relaxed text-muted-foreground">{o.desc}</p>
				{/if}
			</div>
			{#if o.control}
				<div class="shrink-0">{@render o.control()}</div>
			{/if}
		</div>
		{#if o.below}
			<div class="mt-3">{@render o.below()}</div>
		{/if}
	</div>
{/snippet}

<Dialog.Root bind:open={ui.settingsOpen}>
	<Dialog.Content class="gap-0 overflow-hidden p-0 sm:max-w-3xl">
		<Dialog.Description class="sr-only">{t('settings.title')}</Dialog.Description>

		<div class="flex h-[min(34rem,72vh)]">
			<!-- Tab rail -->
			<nav class="flex w-52 shrink-0 flex-col border-r bg-muted/40 p-3">
				<Dialog.Title class="px-3 pt-1 pb-4 font-heading text-base font-semibold">
					{t('settings.title')}
				</Dialog.Title>
				<div class="flex flex-col gap-0.5">
					{#each TABS as tb (tb.id)}
						<button
							onclick={() => (tab = tb.id)}
							aria-current={tab === tb.id}
							class="flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors {tab ===
							tb.id
								? 'bg-background text-foreground shadow-sm ring-1 ring-border/70'
								: 'text-muted-foreground hover:bg-foreground/5 hover:text-foreground'}"
						>
							<HugeiconsIcon
								icon={tb.icon}
								size={17}
								strokeWidth={2}
								class={tab === tb.id ? 'text-primary' : ''}
							/>
							<span class="truncate">{tb.label}</span>
						</button>
					{/each}
				</div>
				{#if version}
					<span class="mt-auto px-3 pb-1 text-[11px] text-muted-foreground">v{version}</span>
				{/if}
			</nav>

			<!-- Content pane. min-w-0: a flex child's min-width is auto, so without it one wide row
			     (a long font name, a long path) widens the pane and pushes every tab off the modal. -->
			<div class="flex min-w-0 flex-1 flex-col">
				<!-- h-14 also keeps the dialog's close button clear of the first row. -->
				<header class="flex h-14 shrink-0 flex-col justify-center border-b px-6 pr-14">
					<h2 class="text-sm font-semibold">{currentTab.label}</h2>
					<p class="truncate text-xs text-muted-foreground">{currentTab.hint}</p>
				</header>

				<div class="min-w-0 flex-1 overflow-y-auto px-6 py-5">
					{#if !loaded}
						<p class="text-sm text-muted-foreground">{t('common.loading')}</p>
					{:else if tab === 'general'}
						<!-- The shortcuts list has no other entry point in the chrome. Closing settings
						     first: two stacked dialogs would trap focus in the wrong one. -->
						<button
							type="button"
							class="mb-5 inline-flex items-center gap-2 rounded-full border bg-muted/50 px-3 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
							onclick={() => {
								ui.settingsOpen = false;
								ui.shortcutsOpen = true;
							}}
						>
							<HugeiconsIcon icon={KeyboardIcon} class="h-3.5 w-3.5" />
							<span
								>{shortcutsHint[0]}<kbd class="font-mono font-medium">{MOD}H</kbd>{shortcutsHint[1] ??
									''}</span
							>
						</button>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.language')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.general.language'),
									desc: t('settings.general.language_hint'),
									control: languagePicker
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.activity')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('player.history'),
									desc: t('settings.playback.play_history_hint'),
									control: historySwitch
								})}
								{@render row({
									title: t('settings.general.discord_rpc'),
									desc: t('settings.general.discord_rpc_hint'),
									control: discordSwitch
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.system')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.general.close_to_tray'),
									desc: t('settings.general.close_to_tray_hint'),
									control: traySwitch
								})}
								{@render row({
									title: t('settings.general.autostart'),
									desc: t('settings.general.autostart_hint'),
									control: autostartSwitch
								})}
							</div>
						</section>
					{:else if tab === 'themes'}
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.theme')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.tabs.themes'),
									desc: t('settings.tabs.themes_hint'),
									control: presetSelect
								})}
								{@render row({
									title: t('settings.themes.primary_color'),
									desc: t('settings.themes.custom_colors'),
									control: accentSwatch,
									below: pickerOpen ? accentPicker : undefined
								})}
								{@render row({
									title: t('settings.themes.background_color'),
									desc:
										currentTheme.kind === 'palette'
											? t('settings.themes.tint_palette_hint', { theme: currentTheme.label })
											: t('settings.themes.tint_hint'),
									control: tintSlider
								})}
								{@render row({
									title: t('settings.themes.roundness'),
									desc: t('settings.themes.roundness_hint'),
									control: radiusSlider
								})}
								{@render row({
									title: t('settings.themes.app_icon'),
									desc: t('settings.themes.app_icon_hint'),
									control: appIconButtons
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
									title: t('settings.themes.open_player'),
									desc: t('settings.themes.open_player_hint'),
									control: openPlayerSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.themes.tabbed_player'),
									desc: t('settings.themes.tabbed_player_hint'),
									control: tabbedSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.themes.artwork_background'),
									desc: t('settings.themes.artwork_background_hint'),
									control: artworkBgSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.themes.artwork_accent'),
									badge: t('settings.themes.experimental'),
									desc: t('settings.themes.artwork_accent_hint'),
									control: artworkAccentSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.themes.reset_theme'),
									desc: t('settings.themes.reset_theme_hint'),
									control: resetButton
								})}
							</div>
						</section>
					{:else if tab === 'playback'}
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.audio')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.playback.audio_quality'),
									desc: t('settings.playback.audio_quality_hint'),
									control: qualityPicker
								})}
								{@render row({
									title: t('settings.playback.autoplay'),
									desc: t('settings.playback.autoplay_hint'),
									control: autoplaySwitch
								})}
								{@render row({
									title: t('settings.playback.prevent_duplicates'),
									desc: t('settings.playback.prevent_duplicates_hint'),
									control: dupSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.playback.sticky_shuffle'),
									desc: t('settings.playback.sticky_shuffle_hint'),
									control: stickyShuffleSwitch,
									tall: true
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.video')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.playback.music_videos'),
									badge: t('settings.themes.experimental'),
									desc: t('settings.playback.music_videos_hint'),
									control: musicVideoSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.playback.hide_videos'),
									desc: t('settings.playback.hide_videos_hint'),
									control: hideVideoSwitch,
									tall: true
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.blocked')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.playback.blocked_artists'),
									desc: t('settings.playback.blocked_artists_hint'),
									below: blockedList
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.lyrics')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.playback.lyrics_provider'),
									desc: t('settings.playback.lyrics_provider_hint'),
									control: boiduSwitch,
									tall: true
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.advanced')}</h3>
							<div class={CARD}>
								{@render row({ title: t('settings.general.stream_clients'), below: clientList })}
							</div>
						</section>
					{:else if tab === 'data'}
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.network')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.general.proxy'),
									desc: t('settings.general.proxy_hint'),
									below: proxyForm
								})}
							</div>
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.storage')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.data.clear_cache'),
									desc: t('settings.data.clear_cache_hint'),
									control: clearButton
								})}
							</div>
						</section>
					{:else if tab === 'about'}
						<div
							class="mb-7 rounded-xl border bg-gradient-to-br from-primary/8 to-transparent px-4 py-4"
						>
							<div class="flex items-center gap-2">
								<span class="font-heading text-lg font-bold">Limusic</span>
								{#if version}
									<span
										class="rounded-full bg-primary/12 px-2 py-0.5 text-[11px] font-semibold text-primary"
									>
										v{version}
									</span>
								{/if}
							</div>
							<p class="mt-1.5 max-w-prose text-xs leading-relaxed text-muted-foreground">
								{t('settings.about.description')}
							</p>
						</div>

						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.support')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.about.kofi'),
									desc: t('settings.about.kofi_hint'),
									control: kofiButton,
									tall: true
								})}
							</div>
						</section>

						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.updates')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.about.check_updates'),
									desc: updateState.available && !updateState.canInstall
										? `${t('settings.about.update_available', { version: updateState.available.version })} ${t('settings.about.update_packaged')}`
										: updateState.available
											? t('settings.about.update_available', { version: updateState.available.version })
											: t('settings.about.up_to_date'),
									control: updateButton,
									below: updateResult && !updateState.available ? updateAlert : undefined
								})}
								{@render row({
									title: t('settings.general.update_banner'),
									desc: t('settings.general.update_banner_hint'),
									control: bannerSwitch,
									tall: true
								})}
							</div>
						</section>

						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.report')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.about.diagnostics'),
									desc: t('settings.about.diagnostics_hint'),
									control: copyDiagButton,
									tall: true,
									below: diagError ? diagAlert : undefined
								})}
								{@render row({
									title: t('settings.about.diagnostics_save'),
									desc: t('settings.about.diagnostics_save_hint'),
									control: saveDiagButton
								})}
								{@render row({
									title: t('settings.about.report_issue'),
									desc: t('settings.about.report_issue_hint'),
									control: reportButton
								})}
							</div>
						</section>

						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.whats_new')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.about.changelog'),
									desc: t('settings.about.version').replace('{version}', version),
									below: changelog
								})}
							</div>
						</section>
					{/if}
				</div>
			</div>
		</div>
	</Dialog.Content>
</Dialog.Root>

<!-- Controls. Split out so the rows above read as a list of settings rather than a wall of markup. -->
{#snippet languagePicker()}
	<Select.Root
		type="single"
		value={currentLocale.id}
		onValueChange={(v) => setLocale(v as LocaleId)}
	>
		<Select.Trigger class="w-44 shrink-0" aria-label={t('settings.general.language')}>
			<span class="flex-1 truncate text-left">{currentLocaleLabel}</span>
		</Select.Trigger>
		<Select.Content>
			{#each LOCALES as locale (locale.id)}
				<Select.Item value={locale.id} label={locale.nativeLabel}>
					{locale.nativeLabel}
				</Select.Item>
			{/each}
		</Select.Content>
	</Select.Root>
{/snippet}

{#snippet historySwitch()}<Switch checked={historyOn} onCheckedChange={setHistory} />{/snippet}
{#snippet discordSwitch()}<Switch checked={discordOn} onCheckedChange={setDiscord} />{/snippet}
{#snippet traySwitch()}<Switch checked={trayOn} onCheckedChange={setTray} />{/snippet}
{#snippet autostartSwitch()}<Switch checked={autostartOn} onCheckedChange={setAutostart} />{/snippet}
{#snippet autoplaySwitch()}<Switch checked={autoplayOn} onCheckedChange={setAutoplay} />{/snippet}
{#snippet dupSwitch()}<Switch
		checked={preventDuplicatesOn}
		onCheckedChange={setPreventDuplicates}
	/>{/snippet}
{#snippet stickyShuffleSwitch()}<Switch
		checked={stickyShuffleOn}
		onCheckedChange={setStickyShuffle}
	/>{/snippet}
{#snippet musicVideoSwitch()}<Switch checked={musicVideosOn} onCheckedChange={setMusicVideos} />{/snippet}
{#snippet hideVideoSwitch()}<Switch checked={hideVideosOn} onCheckedChange={setHideVideos} />{/snippet}
{#snippet boiduSwitch()}<Switch checked={boiduOn} onCheckedChange={setBoidu} />{/snippet}
{#snippet bannerSwitch()}<Switch checked={updateBannerOn} onCheckedChange={setUpdateBanner} />{/snippet}
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
{#snippet artworkAccentSwitch()}<Switch
		checked={appearance.artworkAccent}
		onCheckedChange={(on) => setAppearance({ artworkAccent: on })}
	/>{/snippet}

{#snippet presetSelect()}
	<Select.Root type="single" value={theme.id} onValueChange={(v) => applyTheme(v as ThemeId)}>
		<Select.Trigger class="w-44 shrink-0" aria-label={t('a11y.theme')}>
			<span
				class="size-4 shrink-0 rounded-full ring-1 ring-black/10"
				style="background:{currentTheme.color}"
			></span>
			<span class="flex-1 truncate text-left">{currentTheme.label}</span>
		</Select.Trigger>
		<Select.Content>
			<Select.Group>
				<Select.GroupHeading>{t('settings.themes.accent_colors')}</Select.GroupHeading>
				{#each ACCENT_THEMES as th (th.id)}
					<Select.Item value={th.id} label={th.label}>
						<span
							class="size-4 shrink-0 rounded-full ring-1 ring-black/10"
							style="background:{th.color}"
						></span>
						{th.label}
					</Select.Item>
				{/each}
			</Select.Group>
			<Select.Group>
				<Select.GroupHeading>{t('settings.themes.palettes')}</Select.GroupHeading>
				{#each PALETTE_THEMES as th (th.id)}
					<Select.Item value={th.id} label={th.label}>
						<span
							class="size-4 shrink-0 rounded-full ring-1 ring-black/10"
							style="background:{th.color}"
						></span>
						{th.label}
					</Select.Item>
				{/each}
			</Select.Group>
		</Select.Content>
	</Select.Root>
{/snippet}

{#snippet accentSwatch()}
	<button
		type="button"
		onclick={() => (pickerOpen = !pickerOpen)}
		aria-label={t('a11y.choose_accent')}
		aria-expanded={pickerOpen}
		class="size-8 cursor-pointer rounded-lg ring-1 ring-black/10 transition-transform hover:scale-105 {pickerOpen
			? 'ring-2 ring-primary/60'
			: ''}"
		style="background:{effective.accent}"
	></button>
{/snippet}

{#snippet accentPicker()}
	<ColorPicker value={effective.accent} onchange={(hex) => setCustom({ accent: hex })} />
{/snippet}

{#snippet tintSlider()}
	<Slider
		type="single"
		aria-label={t('a11y.background_tint')}
		max={360}
		step={1}
		disabled={currentTheme.kind === 'palette'}
		value={effective.hue}
		onValueChange={(hue) => setCustom({ hue })}
		class="w-44 shrink-0 [&_[data-slot=slider-range]]:bg-transparent [&_[data-slot=slider-track]]:bg-[linear-gradient(to_right,#f00,#ff0,#0f0,#0ff,#00f,#f0f,#f00)]"
	/>
{/snippet}

{#snippet radiusSlider()}
	<div class="flex w-44 shrink-0 items-center gap-3">
		<Slider
			type="single"
			aria-label={t('a11y.roundness')}
			max={1.5}
			step={0.05}
			value={effective.radius}
			onValueChange={(radius) => setCustom({ radius })}
		/>
		<span class="w-10 shrink-0 text-right font-mono text-xs text-muted-foreground">
			{effective.radius.toFixed(2)}
		</span>
	</div>
{/snippet}

{#snippet fontSelect(key: FontKey, label: string)}
	<Select.Root
		type="single"
		value={isCustomFont[key] ? 'custom' : matchFont(effective[key])}
		onValueChange={(v) => chooseFont(key, v)}
	>
		<Select.Trigger class="w-44 shrink-0" aria-label={label}>
			<span class="min-w-0 flex-1 truncate text-left" style="font-family:{effective[key]}">
				{isCustomFont[key] ? 'Custom' : familyName(effective[key])}
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

{#snippet appIconButtons()}
	<div class="flex shrink-0 items-center gap-2">
		<img src={appIcon.src} alt="" class="size-7 rounded" />
		<Button variant="outline" size="sm" onclick={pickAppIcon}>{t('settings.themes.app_icon_pick')}</Button>
		<Button variant="ghost" size="sm" onclick={resetAppIcon}>{t('common.reset')}</Button>
	</div>
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

{#snippet resetButton()}
	<Button
		variant="outline"
		size="sm"
		disabled={isDefaultCustom()}
		onclick={() => {
			resetCustom();
			isCustomFont = { fontSans: false, fontHeading: false };
			fontName = { fontSans: '', fontHeading: '' };
		}}
	>
		{t('common.reset')}
	</Button>
{/snippet}

<!-- Segmented, not three buttons: the options are one exclusive choice and should look like it. -->
{#snippet qualityPicker()}
	<div class="flex rounded-lg bg-muted p-0.5">
		{#each QUALITIES as q (q.id)}
			<button
				type="button"
				onclick={() => setQuality(q.id)}
				aria-pressed={quality === q.id}
				class="cursor-pointer rounded-md px-3.5 py-1.5 text-xs font-medium transition-colors {quality ===
				q.id
					? 'bg-background text-foreground shadow-sm'
					: 'text-muted-foreground hover:text-foreground'}"
			>
				{t(q.key)}
			</button>
		{/each}
	</div>
{/snippet}

{#snippet clientList()}
	<p class="mb-3 max-w-prose text-xs leading-relaxed text-muted-foreground">
		{t('settings.general.stream_clients_hint', { var: 'LIMUSIC_DISABLED_CLIENTS' })}
	</p>
	<div class="flex flex-col gap-2">
		{#each clients as name (name)}
			<div class="flex items-center justify-between rounded-lg bg-muted/60 py-1.5 pr-2 pl-3">
				<span class="font-mono text-xs">{name}</span>
				<Switch checked={!disabled.has(name)} onCheckedChange={() => toggleClient(name)} />
			</div>
		{/each}
	</div>
{/snippet}

{#snippet blockedList()}
	{#if !blocked.artists.length}
		<p class="text-xs leading-relaxed text-muted-foreground">
			{t('settings.playback.blocked_artists_empty')}
		</p>
	{:else}
		<div class="flex flex-col gap-2">
			{#each showAllBlocked ? blocked.artists : blocked.artists.slice(0, BLOCKED_PREVIEW) as entry (entry.id ?? entry.name)}
				<div class="flex items-center justify-between gap-2 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-3">
					<span class="truncate text-xs">{entry.name}</span>
					<Button
						variant="ghost"
						size="icon"
						class="h-7 w-7 shrink-0"
						aria-label={t('settings.playback.blocked_artists_remove', { name: entry.name })}
						onclick={() => unblockArtist(entry)}
					>
						<HugeiconsIcon icon={RemoveIcon} class="h-3.5 w-3.5" />
					</Button>
				</div>
			{/each}
		</div>
		<div class="mt-2 flex items-center gap-1">
			{#if blocked.artists.length > BLOCKED_PREVIEW}
				<Button
					variant="ghost"
					size="sm"
					class="h-7 px-2 text-xs"
					onclick={() => (showAllBlocked = !showAllBlocked)}
				>
					{showAllBlocked
						? t('settings.playback.blocked_artists_show_less')
						: t('settings.playback.blocked_artists_show_all', { count: blocked.artists.length })}
				</Button>
			{/if}
			<Button variant="ghost" size="sm" class="ml-auto h-7 gap-1.5 px-2 text-xs" onclick={copyBlocked}>
				<HugeiconsIcon icon={Copy01Icon} class="h-3.5 w-3.5" />
				{t('settings.playback.blocked_artists_copy')}
			</Button>
		</div>
	{/if}
{/snippet}

{#snippet proxyForm()}
	<form
		class="flex gap-2"
		onsubmit={(e) => {
			e.preventDefault();
			saveProxy();
		}}
	>
		<Input bind:value={proxyInput} placeholder={t('settings.general.proxy_placeholder')} />
		<Button type="submit" variant="outline">{t('common.save')}</Button>
	</form>
{/snippet}

{#snippet clearButton()}
	<Button variant="destructive" size="sm" onclick={doClearCaches} disabled={clearing}>
		{clearing ? t('common.loading') : t('settings.data.clear_cache_button')}
	</Button>
{/snippet}

{#snippet copyDiagButton()}
	<Button variant="secondary" size="sm" onclick={copyDiagnostics} disabled={diagState === 'busy'}>
		{diagState === 'copied' ? t('settings.about.diagnostics_copied') : t('settings.about.copy')}
	</Button>
{/snippet}

{#snippet saveDiagButton()}
	<Button variant="secondary" size="sm" onclick={saveDiagnostics} disabled={diagState === 'busy'}>
		{diagState === 'saved' ? t('settings.about.diagnostics_saved') : t('common.save')}
	</Button>
{/snippet}

{#snippet reportButton()}
	<Button size="sm" onclick={openBugForm}>{t('settings.about.report_issue_button')}</Button>
{/snippet}

{#snippet kofiButton()}
	<Button variant="secondary" size="sm" onclick={() => api.openExternal('https://ko-fi.com/simohypers')}>
		<HugeiconsIcon icon={Coffee02Icon} size={15} strokeWidth={1.8} />
		{t('settings.about.kofi_button')}
	</Button>
{/snippet}

{#snippet diagAlert()}
	<Alert variant="destructive" class="mt-3">
		<AlertDescription>{diagError}</AlertDescription>
	</Alert>
{/snippet}

{#snippet updateButton()}
	{#if updateState.available && !updateState.canInstall}
		<Button size="sm" onclick={openDownloadPage}>{t('settings.about.download_page')}</Button>
	{:else if updateState.available}
		<Button size="sm" onclick={installUpdate} disabled={updateState.installing}>
			{updateState.installing ? t('common.loading') : t('settings.about.install_update')}
		</Button>
	{:else}
		<Button variant="outline" size="sm" onclick={checkUpdates} disabled={updateState.checking}>
			{updateState.checking ? t('settings.about.checking_updates') : t('settings.about.check_updates')}
		</Button>
	{/if}
{/snippet}

{#snippet updateAlert()}
	<Alert variant={updateResult?.error ? 'destructive' : 'default'}>
		<AlertDescription>{updateResult?.message}</AlertDescription>
	</Alert>
{/snippet}

{#snippet changelog()}
	<Changelog current={version} />
{/snippet}
