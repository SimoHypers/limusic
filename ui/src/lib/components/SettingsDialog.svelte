<script lang="ts">
	import { tick, untrack, type Snippet } from 'svelte';
	import { save } from '@tauri-apps/plugin-dialog';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Settings02Icon,
		PaintBoardIcon,
		PlayCircleIcon,
		Database02Icon,
		InformationCircleIcon,
		KeyboardIcon,
		Cancel01Icon as RemoveIcon,
		Copy01Icon,
		Coffee02Icon,
		DiscordIcon,
		LastFmIcon,
		Globe02Icon,
		ArrowDown01Icon,
		Alert02Icon,
		LinkSquare02Icon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import { Alert, AlertDescription } from '$lib/components/ui/alert';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as Popover from '$lib/components/ui/popover';
	import { HELP_COMBO } from '$lib/shortcuts';
	import { copyText } from '$lib/clipboard';
	import * as api from '$lib/api';
	import { blocked, prefs, refreshLanguage, setAutoplay, ui, toast, unblockArtist } from '$lib/player.svelte';
	import { win } from '$lib/win.svelte';
	import { lt } from '$lib/lt.svelte';
	import AppearanceSettings from '$lib/components/AppearanceSettings.svelte';
	import Changelog from '$lib/components/Changelog.svelte';
	import DiscordSettings from '$lib/components/DiscordSettings.svelte';
	import ScrobbleSettings from '$lib/components/ScrobbleSettings.svelte';
	import {
		updateState,
		availableMessage,
		checkForUpdatesInteractive,
		installUpdate,
		openDownloadPage,
		recheckForUpdates
	} from '$lib/updater.svelte';
	import { getVersion } from '@tauri-apps/api/app';
	import { t, setLocale, currentLocale, LOCALES } from '$lib/i18n.svelte';
	import GlobalHotkeysSettings from '$lib/components/GlobalHotkeysSettings.svelte';
	import LyricsSourcesSettings from '$lib/components/LyricsSourcesSettings.svelte';
	import LanguagePicker from '$lib/components/LanguagePicker.svelte';

	type TabId =
		| 'general'
		| 'themes'
		| 'playback'
		| 'hotkeys'
		| 'discord'
		| 'scrobbling'
		| 'data'
		| 'about';
	const TABS = $derived<{ id: TabId; label: string; hint: string; icon: typeof Settings02Icon }[]>([
		{ id: 'general', label: t('settings.tabs.general'), hint: t('settings.tabs.general_hint'), icon: Settings02Icon },
		{ id: 'themes', label: t('settings.tabs.themes'), hint: t('settings.tabs.themes_hint'), icon: PaintBoardIcon },
		{ id: 'playback', label: t('settings.tabs.playback'), hint: t('settings.tabs.playback_hint'), icon: PlayCircleIcon },
		{ id: 'hotkeys', label: t('settings.tabs.hotkeys'), hint: t('settings.tabs.hotkeys_hint'), icon: KeyboardIcon },
		{ id: 'discord', label: t('settings.tabs.discord'), hint: t('settings.tabs.discord_hint'), icon: DiscordIcon },
		{ id: 'scrobbling', label: t('settings.tabs.scrobbling'), hint: t('settings.tabs.scrobbling_hint'), icon: LastFmIcon },
		{ id: 'data', label: t('settings.tabs.data'), hint: t('settings.tabs.data_hint'), icon: Database02Icon },
		{ id: 'about', label: t('settings.tabs.about'), hint: t('settings.tabs.about_hint'), icon: InformationCircleIcon }
	]);

	// Shared shapes for the settings rows. Kept as strings so the markup below stays readable and
	// every group looks identical without a wrapper component per row.
	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';

	let tab = $state<TabId>('general');
	const currentTab = $derived(TABS.find((tb) => tb.id === tab) ?? TABS[0]);
	const shortcutsHint = $derived(t('settings.general.shortcuts_hint').split('{key}'));
	const currentLocaleLabel = $derived(
		LOCALES.find((l) => l.id === currentLocale.id)?.nativeLabel ?? currentLocale.id
	);
	let langOpen = $state(false);
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
	// Release candidates ship only what the updater installs, so an .rpm, .deb or AUR install has
	// nothing to take from the beta channel. Shown in dev, which is never the AppImage either.
	let betaAvailable = $state(import.meta.env.DEV);
	api.canSelfUpdate().then((v) => (betaAvailable ||= v)).catch(() => {});
	// Result of the last "Check for updates" click — shown inline (a toast renders behind the modal).
	let updateResult = $state<{ message: string; error: boolean } | null>(null);

	// (Re)load whenever the modal opens, so it reflects the current persisted values. Also clear the
	// stale update-check result so re-opening the modal doesn't show it until pressed again.
	// untrack: opening the modal is the only thing that should run it.
	$effect(() => {
		if (!ui.settingsOpen) return;
		untrack(() => {
			// Opened on a section from elsewhere (the lyrics source picker): its tab, scrolled to it
			// once the tab has rendered. The Last.fm menu and "Edit scrobble" open a whole tab.
			if (ui.settingsFocus === 'scrobbling') {
				tab = 'scrobbling';
				ui.settingsFocus = null;
				load();
			} else if (ui.settingsFocus) {
				tab = 'playback';
				const id = `settings-${ui.settingsFocus}`;
				ui.settingsFocus = null;
				load().then(tick).then(() => {
					document.getElementById(id)?.scrollIntoView({ block: 'start', behavior: 'smooth' });
				});
			} else {
				load();
			}
			updateResult = null;
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
	// On unless turned off: loudness matching is what YTM does, and it's what most people want.
	// Off gives the untouched master, limiter included (#298, #300).
	const normalizeOn = $derived(settings.normalize_volume !== 'false');
	// Off by default: experimental, and it runs a second decoder while tracks overlap.
	const crossfadeOn = $derived(settings.crossfade === 'true');
	// A room carries one track and one position, so an overlap cannot be synced: the backend
	// suspends the fade for as long as we are in one (`AppState::apply_crossfade`). Say so here,
	// or it reads as crossfade quietly breaking.
	const crossfadeSuspended = $derived(lt.role !== 'none');
	// Clamped like the player clamps it (`set_crossfade`), so a stored value from anywhere but this
	// slider cannot show a number the audio will not use.
	const crossfadeSecs = $derived.by(() => {
		const secs = Number(settings.crossfade_secs ?? '5');
		return Number.isFinite(secs) ? Math.min(10, Math.max(1, secs)) : 5;
	});
	const hideVideosOn = $derived(settings.hide_videos === 'true');
	// Off until the setting is turned on: still experimental, so nobody gets video they didn't ask
	// for. Same test in `player.svelte.ts`, which hydrates `prefs` at launch.
	const musicVideosOn = $derived(settings.music_videos === 'true');
	// On by default, and only offered with music videos on: album songs that have a video start as
	// it, with the song on the Song/Video switch (#442, #464). Rust reads it in `get_album`.
	const videosFirstOn = $derived(settings.videos_first !== 'false');
	// Off by default, and only offered with music videos on: it costs real GPU time on every frame.
	const ambientOn = $derived(settings.ambient_light === 'true');
	const preventDuplicatesOn = $derived(settings.prevent_duplicates === 'true');
	// Off by default: shuffle applies to the queue it was turned on for (issue #117).
	const stickyShuffleOn = $derived(settings.sticky_shuffle === 'true');
	// Off by default: shuffle keeps what was added with Add to queue behind the playlist (#369).
	const shuffleWholeOn = $derived(settings.shuffle_whole_queue === 'true');
	const updateBannerOn = $derived(settings.update_banner !== 'false');
	const betaOn = $derived(settings.update_channel === 'beta');
	const trayOn = $derived(settings.close_to_tray !== 'false');
	const trackNotificationsOn = $derived(settings.track_notifications === 'true');
	const trackNotificationsFocusedOn = $derived(settings.track_notifications_focused === 'true');
	const playOnStartOn = $derived(settings.play_on_start === 'true');
	const autostartOn = $derived(settings.autostart === 'true');
	const startMinimizedOn = $derived(settings.start_minimized === 'true');
	// `native_chrome` is read-only and platform-derived (commands.rs). `overlay` is macOS, where the
	// traffic lights are fixed at window creation and there is nothing to offer the user (#65).
	const systemTitlebarOn = $derived(settings.native_chrome !== 'off');
	const systemTitlebarFixed = $derived(settings.native_chrome === 'overlay');
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

	// Rust retunes the track that's already playing, so the difference is audible immediately.
	async function setNormalize(on: boolean) {
		settings.normalize_volume = on ? 'true' : 'false';
		await api.setSetting('normalize_volume', settings.normalize_volume);
	}

	async function setCrossfade(on: boolean) {
		settings.crossfade = on ? 'true' : 'false';
		await api.setSetting('crossfade', settings.crossfade);
	}

	async function setCrossfadeSecs(secs: number) {
		settings.crossfade_secs = String(secs);
		await api.setSetting('crossfade_secs', settings.crossfade_secs);
	}

	// Also lands in `prefs`, which is where the player view reads it: the switch has to take effect
	// on the track that's already playing, not on the next launch.
	async function setMusicVideos(on: boolean) {
		settings.music_videos = on ? 'true' : 'false';
		prefs.musicVideos = on;
		await api.setSetting('music_videos', settings.music_videos);
	}

	async function setVideosFirst(on: boolean) {
		settings.videos_first = on ? 'true' : 'false';
		await api.setSetting('videos_first', settings.videos_first);
	}

	// `prefs` after the write: on Linux the write is also what turns WebGL on for the glow.
	async function setAmbient(on: boolean) {
		settings.ambient_light = on ? 'true' : 'false';
		await api.setSetting('ambient_light', settings.ambient_light);
		prefs.ambient = on;
	}

	async function setHideVideos(on: boolean) {
		settings.hide_videos = on ? 'true' : 'false';
		await api.setSetting('hide_videos', settings.hide_videos);
	}

	async function setPreventDuplicates(on: boolean) {
		settings.prevent_duplicates = on ? 'true' : 'false';
		await api.setSetting('prevent_duplicates', settings.prevent_duplicates);
	}

	async function setStickyShuffle(on: boolean) {
		settings.sticky_shuffle = on ? 'true' : 'false';
		await api.setSetting('sticky_shuffle', settings.sticky_shuffle);
	}

	async function setShuffleWhole(on: boolean) {
		settings.shuffle_whole_queue = on ? 'true' : 'false';
		await api.setSetting('shuffle_whole_queue', settings.shuffle_whole_queue);
	}

	async function setUpdateBanner(on: boolean) {
		settings.update_banner = on ? 'true' : 'false';
		await api.setSetting('update_banner', settings.update_banner);
	}

	async function setBeta(on: boolean) {
		settings.update_channel = on ? 'beta' : 'stable';
		await api.setSetting('update_channel', settings.update_channel);
		await recheckForUpdates();
	}

	async function setTray(on: boolean) {
		settings.close_to_tray = on ? 'true' : 'false';
		await api.setSetting('close_to_tray', settings.close_to_tray);
	}

	async function setTrackNotifications(on: boolean) {
		settings.track_notifications = on ? 'true' : 'false';
		await api.setSetting('track_notifications', settings.track_notifications);
	}

	async function setTrackNotificationsFocused(on: boolean) {
		settings.track_notifications_focused = on ? 'true' : 'false';
		await api.setSetting('track_notifications_focused', settings.track_notifications_focused);
	}

	// The backend flips the real window decorations; `win.chrome` is what the SPA keys its own
	// corner rounding, resize borders and window buttons off, so it has to move with it.
	async function setSystemTitlebar(on: boolean) {
		const prev = win.chrome;
		settings.native_chrome = on ? 'on' : 'off';
		win.chrome = on ? 'on' : 'off';
		try {
			await api.setSetting('system_titlebar', on ? 'true' : 'false');
		} catch (e) {
			settings.native_chrome = prev;
			win.chrome = prev;
			toast.error(String(e));
		}
	}

	async function setPlayOnStart(on: boolean) {
		settings.play_on_start = on ? 'true' : 'false';
		await api.setSetting('play_on_start', settings.play_on_start);
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

	async function setStartMinimized(on: boolean) {
		settings.start_minimized = on ? 'true' : 'false';
		try {
			await api.setSetting('start_minimized', settings.start_minimized);
		} catch (e) {
			settings.start_minimized = on ? 'false' : 'true';
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
		try {
			await api.setSetting('proxy', proxyInput.trim());
			settings.proxy = proxyInput.trim();
			toast.success(t('toasts.proxy_saved'));
		} catch (e) {
			toast.error(String(e));
		}
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
     and an optional block underneath for the things that expand (lists, inputs, the changelog). -->
{#snippet row(o: {
	title: string;
	desc?: string;
	badge?: string;
	/** After the title and badge, for a small info affordance that belongs to the title. */
	extra?: Snippet;
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
					{#if o.extra}{@render o.extra()}{/if}
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
	<!-- The Discord, Scrobbling and Appearance tabs put their live preview *beside* the controls rather than
	     under them, so they need the extra width; every other tab reads better narrow. Deliberately not animated:
	     transitioning the width relayouts the whole modal every frame, and WebKitGTK is the webview
	     that would pay for it. -->
	<Dialog.Content
		class="gap-0 overflow-hidden p-0 {tab === 'discord' || tab === 'scrobbling' || tab === 'themes' ? 'sm:max-w-5xl' : tab === 'hotkeys' ? 'sm:max-w-4xl' : 'sm:max-w-3xl'}"
	>
		<Dialog.Description class="sr-only">{t('settings.title')}</Dialog.Description>

		<div class="flex min-w-0 h-[min(38rem,80vh)]">
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

				{#if loaded && tab === 'discord'}
					<DiscordSettings {settings} />
				{:else if loaded && tab === 'scrobbling'}
					<ScrobbleSettings {settings} />
				{:else if tab === 'themes'}
					<AppearanceSettings />
				{:else}
				<!-- relative: an absolute child (the lyrics sources' sr-only live region) would otherwise
				     be laid out against the dialog, outside this pane, and make the overflow-hidden dialog
				     scrollable. scrollIntoView then scrolls the dialog too and clips its rail (#406). -->
				<div class="relative min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-6 py-5">
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
								>{shortcutsHint[0]}<kbd class="font-mono font-medium">{HELP_COMBO}</kbd>{shortcutsHint[1] ??
									''}</span
							>
						</button>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.language')}</h3>
							<div class={CARD}>
								{@render row({
									title: t('settings.general.language'),
									desc: t('settings.general.language_hint'),
									control: languageTrigger,
									below: langOpen ? languageList : undefined
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
									title: t('settings.general.track_notifications'),
									desc: t('settings.general.track_notifications_hint'),
									control: trackNotificationsSwitch
								})}
								{#if trackNotificationsOn}
									{@render row({
										title: t('settings.general.track_notifications_focused'),
										desc: t('settings.general.track_notifications_focused_hint'),
										control: trackNotificationsFocusedSwitch
									})}
								{/if}
								{@render row({
									title: t('settings.general.play_on_start'),
									desc: t('settings.general.play_on_start_hint'),
									control: playOnStartSwitch
								})}
								{@render row({
									title: t('settings.general.autostart'),
									desc: t('settings.general.autostart_hint'),
									control: autostartSwitch
								})}
								{#if autostartOn}
									{@render row({
										title: t('settings.general.start_minimized'),
										desc: t('settings.general.start_minimized_hint'),
										control: startMinimizedSwitch
									})}
								{/if}
								{#if !systemTitlebarFixed}
									{@render row({
										title: t('settings.general.system_titlebar'),
										desc: t('settings.general.system_titlebar_hint'),
										control: systemTitlebarSwitch
									})}
								{/if}
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
									title: t('settings.playback.normalize_volume'),
									desc: t('settings.playback.normalize_volume_hint'),
									control: normalizeSwitch,
									tall: true
								})}
								{@render row({
									title: t('settings.playback.autoplay'),
									desc: t('settings.playback.autoplay_hint'),
									control: autoplaySwitch
								})}
								{@render row({
									title: t('settings.playback.crossfade'),
									badge: t('settings.themes.experimental'),
									desc: crossfadeSuspended
										? t('settings.playback.crossfade_lt_paused')
										: t('settings.playback.crossfade_hint'),
									control: crossfadeSwitch,
									tall: true
								})}
								{#if crossfadeOn}
									{@render row({
										title: t('settings.playback.crossfade_duration'),
										desc: t('settings.playback.crossfade_duration_hint'),
										control: crossfadeSlider,
										tall: true
									})}
								{/if}
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
								{@render row({
									title: t('settings.playback.shuffle_whole_queue'),
									desc: t('settings.playback.shuffle_whole_queue_hint'),
									control: shuffleWholeSwitch,
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
								{#if musicVideosOn}
									{@render row({
										title: t('settings.playback.videos_first'),
										desc: t('settings.playback.videos_first_hint'),
										control: videosFirstSwitch,
										tall: true
									})}
									{@render row({
										title: t('settings.playback.ambient_light'),
										badge: t('settings.themes.experimental'),
										desc: t('settings.playback.ambient_light_hint'),
										extra: ambientGpu,
										control: ambientSwitch,
										tall: true
									})}
								{/if}
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
						<section class={GROUP} id="settings-lyrics">
							<h3 class={LABEL}>{t('settings.sections.lyrics')}</h3>
							<LyricsSourcesSettings {settings} />
						</section>
						<section class={GROUP}>
							<h3 class={LABEL}>{t('settings.sections.advanced')}</h3>
							<div class={CARD}>
								{@render row({ title: t('settings.general.stream_clients'), below: clientList })}
							</div>
						</section>
					{:else if tab === 'hotkeys'}
						<GlobalHotkeysSettings />
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
										? `${availableMessage(updateState.available)} ${t('settings.about.update_packaged')}`
										: updateState.available
											? availableMessage(updateState.available)
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
								{#if betaAvailable}
									{@render row({
										title: t('settings.about.beta'),
										desc: t('settings.about.beta_hint'),
										control: betaSwitch,
										tall: true
									})}
								{/if}
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
				{/if}
			</div>
		</div>
	</Dialog.Content>
</Dialog.Root>

<!-- Controls. Split out so the rows above read as a list of settings rather than a wall of markup. -->
<!-- Picking refreshes the page behind the dialog once Rust has the new language: half of what is on
     screen is YouTube's own text (#274), and that half only changes on the next fetch. -->
{#snippet languageTrigger()}
	<button
		type="button"
		onclick={() => (langOpen = !langOpen)}
		aria-expanded={langOpen}
		aria-label="{t('settings.general.language')}: {currentLocaleLabel}"
		class="flex h-9 w-44 shrink-0 cursor-pointer items-center gap-2 rounded-4xl border border-input bg-input/30 px-3 text-sm transition-colors hover:bg-input/50"
	>
		<HugeiconsIcon icon={Globe02Icon} strokeWidth={2} class="size-4 shrink-0 text-muted-foreground" />
		<span class="flex-1 truncate text-left">{currentLocaleLabel}</span>
		<HugeiconsIcon
			icon={ArrowDown01Icon}
			strokeWidth={2}
			class="size-4 shrink-0 text-muted-foreground transition-transform {langOpen ? 'rotate-180' : ''}"
		/>
	</button>
{/snippet}

{#snippet languageList()}
	<LanguagePicker
		onclose={() => (langOpen = false)}
		onpick={(id) => {
			langOpen = false;
			setLocale(id).then(refreshLanguage);
		}}
	/>
{/snippet}

{#snippet historySwitch()}<Switch checked={historyOn} onCheckedChange={setHistory} />{/snippet}
{#snippet traySwitch()}<Switch checked={trayOn} onCheckedChange={setTray} />{/snippet}
{#snippet trackNotificationsSwitch()}<Switch
		checked={trackNotificationsOn}
		onCheckedChange={setTrackNotifications}
	/>{/snippet}
{#snippet trackNotificationsFocusedSwitch()}<Switch
		checked={trackNotificationsFocusedOn}
		onCheckedChange={setTrackNotificationsFocused}
	/>{/snippet}
{#snippet playOnStartSwitch()}<Switch checked={playOnStartOn} onCheckedChange={setPlayOnStart} />{/snippet}
{#snippet autostartSwitch()}<Switch checked={autostartOn} onCheckedChange={setAutostart} />{/snippet}
{#snippet startMinimizedSwitch()}<Switch checked={startMinimizedOn} onCheckedChange={setStartMinimized} />{/snippet}
{#snippet systemTitlebarSwitch()}<Switch
		checked={systemTitlebarOn}
		onCheckedChange={setSystemTitlebar}
	/>{/snippet}
{#snippet autoplaySwitch()}<Switch checked={prefs.autoplay} onCheckedChange={setAutoplay} />{/snippet}

{#snippet crossfadeSwitch()}<Switch checked={crossfadeOn} onCheckedChange={setCrossfade} />{/snippet}

{#snippet crossfadeSlider()}
	<div class="flex w-44 shrink-0 items-center gap-3">
		<Slider
			type="single"
			aria-label={t('settings.playback.crossfade_duration')}
			min={1}
			max={10}
			step={1}
			value={crossfadeSecs}
			onValueChange={setCrossfadeSecs}
		/>
		<span class="w-8 shrink-0 text-right font-mono text-xs text-muted-foreground">
			{t('settings.playback.crossfade_seconds', { secs: crossfadeSecs })}
		</span>
	</div>
{/snippet}
{#snippet dupSwitch()}<Switch
		checked={preventDuplicatesOn}
		onCheckedChange={setPreventDuplicates}
	/>{/snippet}
{#snippet stickyShuffleSwitch()}<Switch
		checked={stickyShuffleOn}
		onCheckedChange={setStickyShuffle}
	/>{/snippet}
{#snippet shuffleWholeSwitch()}<Switch
		checked={shuffleWholeOn}
		onCheckedChange={setShuffleWhole}
	/>{/snippet}
{#snippet normalizeSwitch()}<Switch checked={normalizeOn} onCheckedChange={setNormalize} />{/snippet}
{#snippet musicVideoSwitch()}<Switch checked={musicVideosOn} onCheckedChange={setMusicVideos} />{/snippet}
{#snippet videosFirstSwitch()}<Switch checked={videosFirstOn} onCheckedChange={setVideosFirst} />{/snippet}
{#snippet ambientSwitch()}<Switch checked={ambientOn} onCheckedChange={setAmbient} />{/snippet}
<!-- The GPU note, behind a warning glyph by the title: it matters to the few whose card is weak,
     and a paragraph under the switch read as a reason not to try it. A popover rather than a
     tooltip, so it opens on a click or a key and can hold the link. -->
{#snippet ambientGpu()}
	<Popover.Root>
		<Popover.Trigger
			class="-m-1 cursor-pointer rounded-md p-1 text-muted-foreground transition-colors hover:text-foreground data-[state=open]:text-foreground"
			aria-label={t('settings.playback.ambient_light_gpu_title')}
			title={t('settings.playback.ambient_light_gpu_title')}
		>
			<HugeiconsIcon icon={Alert02Icon} size={14} strokeWidth={1.8} />
		</Popover.Trigger>
		<Popover.Content side="top" align="start" class="w-80 gap-3">
			<div class="flex items-start gap-2.5">
				<HugeiconsIcon icon={Alert02Icon} size={16} strokeWidth={1.8} class="mt-0.5 shrink-0" />
				<div class="min-w-0">
					<p class="text-sm font-semibold">{t('settings.playback.ambient_light_gpu_title')}</p>
					<p class="mt-1 text-xs leading-relaxed text-muted-foreground">
						{t('settings.playback.ambient_light_gpu')}
					</p>
				</div>
			</div>
			<Button
				variant="secondary"
				size="sm"
				class="self-start"
				onclick={() => api.openExternal('https://www.videocardbenchmark.net/gpu_list.php')}
			>
				<HugeiconsIcon icon={LinkSquare02Icon} size={15} strokeWidth={1.8} />
				{t('settings.playback.ambient_light_gpu_check')}
			</Button>
		</Popover.Content>
	</Popover.Root>
{/snippet}
{#snippet hideVideoSwitch()}<Switch checked={hideVideosOn} onCheckedChange={setHideVideos} />{/snippet}
{#snippet bannerSwitch()}<Switch checked={updateBannerOn} onCheckedChange={setUpdateBanner} />{/snippet}
{#snippet betaSwitch()}<Switch checked={betaOn} onCheckedChange={setBeta} />{/snippet}

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
