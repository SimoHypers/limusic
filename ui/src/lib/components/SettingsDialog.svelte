<script lang="ts">
	import { untrack } from 'svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Cancel01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import { Alert, AlertDescription } from '$lib/components/ui/alert';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as Select from '$lib/components/ui/select';
	import * as api from '$lib/api';
	import { prefs, ui, toast } from '$lib/player.svelte';
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

	type TabId = 'general' | 'themes' | 'playback' | 'data' | 'about';
	const TABS = $derived<{ id: TabId; label: string }[]>([
		{ id: 'general', label: t('settings.tabs.general') },
		{ id: 'themes', label: t('settings.tabs.themes') },
		{ id: 'playback', label: t('settings.tabs.playback') },
		{ id: 'data', label: t('settings.tabs.data') },
		{ id: 'about', label: t('settings.tabs.about') }
	]);

	const ACCENT_THEMES = THEMES.filter((t) => t.kind === 'accent');
	const PALETTE_THEMES = THEMES.filter((t) => t.kind === 'palette');
	const currentTheme = $derived(THEMES.find((t) => t.id === theme.id) ?? THEMES[0]);

	// --- Themes tab ---
	type FontKey = 'fontSans' | 'fontHeading';
	const FONT_ROWS: { key: FontKey; label: string; hint: string }[] = [
		{ key: 'fontSans', label: 'Interface font', hint: 'Everything except headings.' },
		{ key: 'fontHeading', label: 'Heading font', hint: 'Page and section titles.' }
	];
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
			title: 'Load a font',
			filters: [{ name: 'Fonts', extensions: ['ttf', 'otf', 'woff', 'woff2'] }]
		});
		for (const path of picked ?? []) {
			try {
				toast.success(`${await addFontFile(path)} loaded — pick it above`);
			} catch (e) {
				toast.error(String(e));
			}
		}
	}

	function chooseFont(key: FontKey, value: string) {
		isCustomFont[key] = value === 'custom';
		if (value === 'custom') fontName[key] = familyName(effective[key]);
		else setCustom({ [key]: value } as Partial<Custom>);
	}

	function typeFont(key: FontKey, name: string) {
		fontName[key] = name;
		// Blank clears the override, so the preset's font comes back.
		setCustom({ [key]: name.trim() ? `'${name.trim()}', sans-serif` : null } as Partial<Custom>);
	}

	let tab = $state<TabId>('general');
	let settings = $state<Record<string, string>>({});
	let clients = $state<string[]>([]);
	let proxyInput = $state('');
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
		{ id: 'LOW', label: 'Low' },
		{ id: 'AUTO', label: 'Auto' },
		{ id: 'HIGH', label: 'High' }
	];

	async function setQuality(q: string) {
		settings.quality = q;
		await api.setSetting('quality', q);
		// Cached URLs are keyed by video only, so clear them to apply the new quality everywhere.
		await api.clearCaches();
		toast.success('Audio quality updated');
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
		toast.success('Proxy saved — restart to apply');
	}

	async function doClearCaches() {
		clearing = true;
		try {
			await api.clearCaches();
			toast.success('Caches cleared');
		} finally {
			clearing = false;
		}
	}
</script>

<Dialog.Root bind:open={ui.settingsOpen}>
	<Dialog.Content class="gap-0 overflow-hidden p-0 sm:max-w-3xl">
		<div class="flex items-center border-b px-6 py-4">
			<Dialog.Title class="text-lg font-semibold">{t('settings.title')}</Dialog.Title>
			<Dialog.Description class="sr-only">{t('settings.general.title')}</Dialog.Description>
		</div>

		<div class="flex h-[28rem]">
			<!-- Tab rail -->
			<nav class="w-48 shrink-0 border-r p-2">
				{#each TABS as tTab (tTab.id)}
					<button
						onclick={() => (tab = tTab.id)}
						class="w-full rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors {tab ===
						tTab.id
							? 'bg-accent text-accent-foreground'
							: 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'}"
					>
						{tTab.label}
					</button>
				{/each}
			</nav>

			<!-- Content pane. min-w-0: a flex child's min-width is auto, so without it one wide row
			     (a long font name, a long path) widens the pane and pushes every tab off the modal. -->
			<div class="min-w-0 flex-1 overflow-y-auto px-6 py-4">
				{#if !loaded}
					<p class="text-sm text-muted-foreground">{t('common.loading')}</p>
				{:else if tab === 'general'}
					<div class="flex items-center justify-between gap-8 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">{t('settings.general.language')}</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{t('settings.general.language_hint')}
							</p>
						</div>
						<Select.Root
							type="single"
							value={currentLocale.id}
							onValueChange={(v) => setLocale(v as LocaleId)}
						>
							<Select.Trigger class="w-44 shrink-0" aria-label={t('settings.general.language')}>
								<span class="flex-1 text-left">
									{LOCALES.find((l) => l.id === currentLocale.id)?.nativeLabel ?? currentLocale.id}
								</span>
							</Select.Trigger>
							<Select.Content>
								{#each LOCALES as loc (loc.id)}
									<Select.Item value={loc.id} label={loc.nativeLabel}>
										{loc.nativeLabel} ({loc.label})
									</Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">{t('settings.playback.play_history')}</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{t('settings.playback.play_history_hint')}
							</p>
						</div>
						<Switch checked={historyOn} onCheckedChange={setHistory} />
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">{t('settings.general.discord_rpc')}</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{t('settings.general.discord_rpc_hint')}
							</p>
						</div>
						<Switch checked={discordOn} onCheckedChange={setDiscord} />
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">{t('settings.general.close_to_tray')}</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{t('settings.general.close_to_tray_hint')}
							</p>
						</div>
						<Switch checked={trayOn} onCheckedChange={setTray} />
					</div>
					<div class="flex items-start justify-between gap-4 py-3">
						<div class="min-w-0">
							<div class="font-medium">{t('settings.general.autostart')}</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{t('settings.general.autostart_hint')}
							</p>
						</div>
						<Switch checked={autostartOn} onCheckedChange={setAutostart} />
					</div>
				{:else if tab === 'themes'}
					<div class="flex items-center justify-between gap-8 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Preset</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Accent colors tint the default look; palettes swap every color.
							</p>
						</div>
						<Select.Root
							type="single"
							value={theme.id}
							onValueChange={(v) => applyTheme(v as ThemeId)}
						>
							<Select.Trigger class="w-44 shrink-0" aria-label="Theme">
								<span class="size-4 shrink-0 rounded-full ring-1 ring-black/10" style="background:{currentTheme.color}"></span>
								<span class="flex-1 text-left">{currentTheme.label}</span>
							</Select.Trigger>
							<Select.Content>
								<Select.Group>
									<Select.GroupHeading>Accent colors</Select.GroupHeading>
									{#each ACCENT_THEMES as t (t.id)}
										<Select.Item value={t.id} label={t.label}>
											<span class="size-4 shrink-0 rounded-full ring-1 ring-black/10" style="background:{t.color}"></span>
											{t.label}
										</Select.Item>
									{/each}
								</Select.Group>
								<Select.Group>
									<Select.GroupHeading>Palettes</Select.GroupHeading>
									{#each PALETTE_THEMES as t (t.id)}
										<Select.Item value={t.id} label={t.label}>
											<span class="size-4 shrink-0 rounded-full ring-1 ring-black/10" style="background:{t.color}"></span>
											{t.label}
										</Select.Item>
									{/each}
								</Select.Group>
							</Select.Content>
						</Select.Root>
					</div>

					<div class="border-b py-3">
						<div class="flex items-center justify-between gap-8">
							<div class="min-w-0">
								<div class="font-medium">Accent color</div>
								<p class="mt-0.5 text-sm text-muted-foreground">
									Buttons, highlights and the progress bar. Applies over any preset.
								</p>
							</div>
							<button
								type="button"
								onclick={() => (pickerOpen = !pickerOpen)}
								aria-label="Choose accent color"
								aria-expanded={pickerOpen}
								class="size-8 shrink-0 rounded-md ring-1 ring-black/10 transition-transform hover:scale-105"
								style="background:{effective.accent}"
							></button>
						</div>
						{#if pickerOpen}
							<div class="mt-3">
								<ColorPicker
									value={effective.accent}
									onchange={(hex) => setCustom({ accent: hex })}
								/>
							</div>
						{/if}
					</div>

					<div class="flex items-center justify-between gap-8 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Background tint</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{#if currentTheme.kind === 'palette'}
									Only shades the default palette — {currentTheme.label} brings its own colors.
								{:else}
									Shades the greys: surfaces, borders and secondary text.
								{/if}
							</p>
						</div>
						<Slider
							type="single"
							aria-label="Background tint"
							max={360}
							step={1}
							disabled={currentTheme.kind === 'palette'}
							value={effective.hue}
							onValueChange={(hue) => setCustom({ hue })}
							class="w-44 shrink-0 [&_[data-slot=slider-range]]:bg-transparent [&_[data-slot=slider-track]]:bg-[linear-gradient(to_right,#f00,#ff0,#0f0,#0ff,#00f,#f0f,#f00)]"
						/>
					</div>

					<div class="flex items-center justify-between gap-8 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Roundness</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Corner radius of cards, buttons and artwork.
							</p>
						</div>
						<div class="flex w-44 shrink-0 items-center gap-3">
							<Slider
								type="single"
								aria-label="Roundness"
								max={1.5}
								step={0.05}
								value={effective.radius}
								onValueChange={(radius) => setCustom({ radius })}
							/>
							<span class="w-10 shrink-0 text-right font-mono text-xs text-muted-foreground">
								{effective.radius.toFixed(2)}
							</span>
						</div>
					</div>

					{#each FONT_ROWS as row (row.key)}
						<div class="border-b py-3">
							<div class="flex items-center justify-between gap-8">
								<div class="min-w-0">
									<div class="font-medium">{row.label}</div>
									<p class="mt-0.5 text-sm text-muted-foreground">{row.hint}</p>
								</div>
								<Select.Root
									type="single"
									value={isCustomFont[row.key] ? 'custom' : matchFont(effective[row.key])}
									onValueChange={(v) => chooseFont(row.key, v)}
								>
									<Select.Trigger class="w-44 shrink-0" aria-label={row.label}>
										<span
											class="min-w-0 flex-1 truncate text-left"
											style="font-family:{effective[row.key]}"
										>
											{isCustomFont[row.key] ? 'Custom' : familyName(effective[row.key])}
										</span>
									</Select.Trigger>
									<!-- max-w: a loaded font's name is whatever the file was called, and the
									     dropdown grows to its widest item. -->
									<Select.Content class="max-w-64">
										{#each FONTS as f (f.value)}
											<Select.Item value={f.value} label={f.label}>
												<span class="block truncate" style="font-family:{f.value}">{f.label}</span>
											</Select.Item>
										{/each}
										{#if custom.fontFiles.length}
											<Select.Group>
												<Select.GroupHeading>Your fonts</Select.GroupHeading>
												{#each fileFonts() as f (f.value)}
													<Select.Item value={f.value} label={f.label}>
														<span class="block truncate" style="font-family:{f.value}">
															{f.label}
														</span>
													</Select.Item>
												{/each}
											</Select.Group>
										{/if}
										<Select.Item value="custom" label="Custom">Custom…</Select.Item>
									</Select.Content>
								</Select.Root>
							</div>
							{#if isCustomFont[row.key]}
								<div class="mt-3">
									<Input
										value={fontName[row.key]}
										oninput={(e) => typeFont(row.key, e.currentTarget.value)}
										placeholder="Font installed on this computer, e.g. Inter"
										aria-label="{row.label} family name"
										spellcheck={false}
										style="font-family:{effective[row.key]}"
									/>
									{#if fontName[row.key].trim() && !fontAvailable(fontName[row.key])}
										<p class="mt-1.5 text-sm text-muted-foreground">
											Not installed — install the font, then reopen settings.
										</p>
									{/if}
								</div>
							{/if}
						</div>
					{/each}

					<div class="border-b py-3">
						<div class="flex items-center justify-between gap-8">
							<div class="min-w-0">
								<div class="font-medium">Font files</div>
								<p class="mt-0.5 text-sm text-muted-foreground">
									Load a .ttf, .otf or .woff from anywhere on this computer. It joins both dropdowns
									above.
								</p>
							</div>
							<Button variant="outline" size="sm" class="shrink-0" onclick={pickFontFiles}>
								Add font…
							</Button>
						</div>
						{#if custom.fontFiles.length}
							<div class="mt-3 flex flex-col gap-1.5">
								{#each custom.fontFiles as path (path)}
									<div class="flex items-center gap-3 rounded-md bg-secondary/50 py-1.5 pr-1.5 pl-3">
										<!-- The name is the identity; the path only earns a tooltip. A font called
										     BigBlueTerm437NerdFontMono-Regular is wider than the modal. -->
										<span
											class="min-w-0 flex-1 truncate"
											style="font-family:'{fileFamily(path)}'"
											title={path}
										>
											{fileFamily(path)}
										</span>
										<button
											type="button"
											onclick={() => removeFontFile(path)}
											aria-label="Remove {fileFamily(path)}"
											class="flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
										>
											<HugeiconsIcon icon={Cancel01Icon} size={14} />
										</button>
									</div>
								{/each}
							</div>
						{/if}
					</div>

					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Open the player when you press play</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								On, playing a song, album or playlist brings up the full player view. Off, it starts
								playing and leaves you on the page you were browsing.
							</p>
						</div>
						<Switch
							checked={appearance.openPlayerOnPlay}
							onCheckedChange={(on) => setAppearance({ openPlayerOnPlay: on })}
						/>
					</div>

					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Queue and lyrics in the player view</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								On, the player view carries them as tabs and the bar's two buttons switch between
								them. Off, those buttons only ever open the side panels, which stay open over the
								player view so you can see both at once.
							</p>
						</div>
						<Switch
							checked={appearance.tabbedPlayer}
							onCheckedChange={(on) => setAppearance({ tabbedPlayer: on })}
						/>
					</div>

					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Artwork background</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Tint the player view with the playing track's cover, blurred. Off leaves it plain.
							</p>
						</div>
						<Switch
							checked={appearance.artworkBackground}
							onCheckedChange={(on) => setAppearance({ artworkBackground: on })}
						/>
					</div>

					<div class="flex items-center justify-between gap-4 py-3">
						<div class="min-w-0">
							<div class="font-medium">Reset customization</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Drop the color, roundness and font overrides. Keeps the preset.
							</p>
						</div>
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
							Reset
						</Button>
					</div>
				{:else if tab === 'playback'}
					<div class="border-b py-3">
						<div class="font-medium">Audio quality</div>
						<p class="mt-0.5 mb-3 text-sm text-muted-foreground">
							Preferred stream quality when resolving a track.
						</p>
						<div class="flex gap-2">
							{#each QUALITIES as q (q.id)}
								<Button
									variant={quality === q.id ? 'default' : 'outline'}
									size="sm"
									onclick={() => setQuality(q.id)}
								>
									{q.label}
								</Button>
							{/each}
						</div>
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Autoplay</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Keep the music going with similar songs when your queue ends.
							</p>
						</div>
						<Switch checked={autoplayOn} onCheckedChange={setAutoplay} />
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Prevent duplicate tracks in queue</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Adding a track that's already in the queue moves it from its old position instead of
								adding a second copy.
							</p>
						</div>
						<Switch checked={preventDuplicatesOn} onCheckedChange={setPreventDuplicates} />
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="flex items-center gap-2">
								<span class="font-medium">Play music videos</span>
								<span
									class="rounded-full bg-primary/15 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary"
								>
									Experimental
								</span>
							</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								When a track is a music video, the player shows the video instead of the artwork.
								Uses noticeably more data and battery than audio alone.
							</p>
						</div>
						<Switch checked={musicVideosOn} onCheckedChange={setMusicVideos} />
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Hide music videos</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Keep only the audio version of a track, so the official video doesn't turn up
								beside it. Applies to newly loaded content.
							</p>
						</div>
						<Switch checked={hideVideosOn} onCheckedChange={setHideVideos} />
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Word-by-word lyrics</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Asks lyrics-api.boidu.dev first, the only source here with per-word timings, so
								lyrics can highlight as they're sung. It's checked for every track, so turning
								this off keeps your listening off that service. Other sources still provide
								line-by-line lyrics.
							</p>
						</div>
						<Switch checked={boiduOn} onCheckedChange={setBoidu} />
					</div>
					<div class="py-3">
						<div class="font-medium">Stream clients</div>
						<p class="mt-0.5 mb-2 text-sm text-muted-foreground">
							Advanced — turn a client off to skip it when resolving streams. Overridden by the
							<span class="font-mono text-xs">LIMUSIC_DISABLED_CLIENTS</span> env var.
						</p>
						<div class="flex flex-col gap-2">
							{#each clients as name (name)}
								<div class="flex items-center justify-between">
									<span class="font-mono text-sm">{name}</span>
									<Switch
										checked={!disabled.has(name)}
										onCheckedChange={() => toggleClient(name)}
									/>
								</div>
							{/each}
						</div>
					</div>
				{:else if tab === 'data'}
					<div class="border-b py-3">
						<div class="font-medium">Proxy</div>
						<p class="mt-0.5 mb-3 text-sm text-muted-foreground">
							HTTP/SOCKS proxy for all YouTube traffic. Takes effect on restart.
						</p>
						<form
							class="flex gap-2"
							onsubmit={(e) => {
								e.preventDefault();
								saveProxy();
							}}
						>
							<Input bind:value={proxyInput} placeholder="http://host:port (blank = none)" />
							<Button type="submit" variant="outline">Save</Button>
						</form>
					</div>
					<div class="py-3">
						<div class="font-medium">Cache</div>
						<p class="mt-0.5 mb-3 text-sm text-muted-foreground">
							Clear cached stream URLs and downloaded audio bytes.
						</p>
						<Button variant="destructive" size="sm" onclick={doClearCaches} disabled={clearing}>
							{clearing ? 'Clearing…' : 'Clear caches'}
						</Button>
					</div>
				{:else if tab === 'about'}
					<div class="border-b py-3">
						<div class="font-heading text-lg font-bold">Limusic</div>
						<p class="mt-1 text-sm text-muted-foreground">
							A cross-platform desktop YouTube Music client — ad-free playback straight from
							YouTube's private API, with your real library and OS media keys.
						</p>
						{#if version}<p class="mt-2 text-sm text-muted-foreground">Version {version}</p>{/if}
					</div>
					<div class="flex items-center justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Updates</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{#if updateState.available && !updateState.canInstall}
									Version {updateState.available.version} is available. This build was installed by a
									package manager, so update it the same way.
								{:else if updateState.available}
									Version {updateState.available.version} is available.
								{:else}
									Check GitHub for a newer release.
								{/if}
							</p>
						</div>
						{#if updateState.available && !updateState.canInstall}
							<Button size="sm" onclick={openDownloadPage}>Download</Button>
						{:else if updateState.available}
							<Button size="sm" onclick={installUpdate} disabled={updateState.installing}>
								{updateState.installing ? 'Updating…' : 'Update now'}
							</Button>
						{:else}
							<Button
								variant="outline"
								size="sm"
								onclick={checkUpdates}
								disabled={updateState.checking}
							>
								{updateState.checking ? 'Checking…' : 'Check for updates'}
							</Button>
						{/if}
					</div>
					{#if updateResult && !updateState.available}
						<Alert variant={updateResult.error ? 'destructive' : 'default'}>
							<AlertDescription>{updateResult.message}</AlertDescription>
						</Alert>
					{/if}
					<div class="flex items-start justify-between gap-4 py-3">
						<div class="min-w-0">
							<div class="font-medium">Tell me about new versions</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Check on launch and show a banner when a newer version is out. Off means no check
								and no banner, so use the button above to look.
							</p>
						</div>
						<Switch checked={updateBannerOn} onCheckedChange={setUpdateBanner} />
					</div>
					<div class="border-t py-3">
						<div class="font-medium">What's new</div>
						<p class="mt-0.5 mb-1 text-sm text-muted-foreground">
							What changed in this version and the ones before it.
						</p>
						<Changelog current={version} />
					</div>
				{/if}
			</div>
		</div>
	</Dialog.Content>
</Dialog.Root>
