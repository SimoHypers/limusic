<script lang="ts">
	// The Scrobbling tab (#327, #404): when a play counts, what it is sent as, rules that rewrite
	// every track, and fixes for single tracks. Built like the Discord tab, options on the left and a
	// live preview on the right, because "find (?i)vevo$ in the artist" means nothing until you see
	// the track it changes.
	//
	// The preview is computed by Rust (`lastfm_preview`) from this tab's unsaved state, with the same
	// function the scrobbler sends from. Saving is debounced like the Discord tab, and the scrobbler
	// picks the new config up at once (`set_setting`), so a rule saved mid-track already applies to
	// that track's scrobble.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		ArrowDown02Icon,
		Cancel01Icon,
		Delete02Icon,
		FileImportIcon,
		LinkSquare02Icon,
		Loading03Icon,
		MusicNote01Icon,
		PencilEdit02Icon
	} from '@hugeicons/core-free-icons';
	import { untrack, type Snippet } from 'svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import * as Select from '$lib/components/ui/select';
	import * as Popover from '$lib/components/ui/popover';
	import * as api from '$lib/api';
	import type { ScrobblePreview, ScrobbleTrack } from '$lib/api';
	import { playback, prefs, toast, ui } from '$lib/player.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
	import { thumb } from '$lib/thumb';
	import LastFmIcon from '$lib/components/LastFmIcon.svelte';
	import { connectLastfm, disconnectLastfm, lastfm } from '$lib/lastfm.svelte';
	import {
		connectListenBrainz,
		disconnectListenBrainz,
		listenbrainz,
		watchListenBrainz
	} from '$lib/listenbrainz.svelte';
	import { onMount } from 'svelte';
	import {
		PRESETS,
		blankEdit,
		importScrobbleFile,
		mergeImport,
		parseScrobbleConfig,
		scrobbleAt,
		type Field,
		type ScrobbleConfig,
		type ScrobbleEdit
	} from '$lib/scrobble';

	let { settings }: { settings: Record<string, string> } = $props();

	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';
	const HINT = 'mb-2 px-1 max-w-prose text-xs leading-relaxed text-muted-foreground';

	// Seeded once, then owned here (see DiscordSettings for why not a $derived).
	let cfg = $state<ScrobbleConfig>(untrack(() => parseScrobbleConfig(settings.lastfm_config)));

	let saveTimer: ReturnType<typeof setTimeout> | undefined;
	function save() {
		const json = JSON.stringify(cfg);
		settings.lastfm_config = json;
		clearTimeout(saveTimer);
		saveTimer = setTimeout(() => {
			api.setSetting('lastfm_config', json).catch((e) => toast.error(String(e)));
		}, 250);
	}

	function set(patch: Partial<ScrobbleConfig>) {
		Object.assign(cfg, patch);
		save();
	}

	// The two switches from #231 keep their own settings rows (and their translations), so they
	// save straight through instead of riding in the blob.
	const primaryOn = $derived(settings.lastfm_primary_artist === 'true');
	const strictOn = $derived(settings.lastfm_primary_strict === 'true');
	async function setRow(key: 'lastfm_primary_artist' | 'lastfm_primary_strict', on: boolean) {
		settings[key] = on ? 'true' : 'false';
		await api.setSetting(key, settings[key]).catch((e) => toast.error(String(e)));
	}

	// --- The account card: avatar and counts off the user's public profile ---
	let profile = $state<api.LastfmProfile | null>(null);
	$effect(() => {
		const name = lastfm.connected ? lastfm.username : null;
		profile = null;
		if (!name) return;
		api.lastfmProfile()
			.then((p) => lastfm.username === name && (profile = p))
			.catch(() => {});
	});
	// --- ListenBrainz: token auth, listen count off the public API ---
	let lbProfile = $state<api.ListenBrainzProfile | null>(null);
	let lbToken = $state('');
	onMount(watchListenBrainz);
	$effect(() => {
		const name = listenbrainz.connected ? listenbrainz.username : null;
		lbProfile = null;
		if (!name) return;
		api.listenbrainzProfile()
			.then((p) => listenbrainz.username === name && (lbProfile = p))
			.catch(() => {});
	});
	function connectLb() {
		const token = lbToken.trim();
		if (!token) return;
		lbToken = '';
		connectListenBrainz(token);
	}
	const num = (n: number) => n.toLocaleString(currentLocale.id);
	const since = $derived(
		profile?.since
			? new Date(profile.since * 1000).toLocaleDateString(currentLocale.id, {
					month: 'long',
					year: 'numeric'
				})
			: null
	);
	const STATS = $derived(
		profile
			? [
					{ label: t('settings.scrobbling.stat_scrobbles'), value: profile.scrobbles },
					{ label: t('settings.scrobbling.stat_artists'), value: profile.artists },
					{ label: t('settings.scrobbling.stat_tracks'), value: profile.tracks }
				]
			: []
	);

	// --- The track the preview shows ---
	// "Edit scrobble" in a track menu picks one; otherwise the one playing; otherwise a stand-in that
	// shows off what the options do (a video upload with the artist in its title).
	type Track = ScrobbleTrack & { thumbnail?: string };
	let picked = $state<Track | null>(null);
	// The editor opens once the preview has answered for the picked track, so it starts from what
	// that track scrobbles as rather than from the previous one's.
	let editWhenReady = false;
	$effect(() => {
		const s = ui.scrobbleTrack;
		if (!s) return;
		ui.scrobbleTrack = null;
		picked = s;
		editing = false;
		editWhenReady = true;
	});
	const SAMPLE: Track = {
		video_id: '',
		title: 'Nova Sky - Higher Ground (Official Video)',
		artists: 'NovaSkyVEVO',
		album: null,
		is_video: true
	};
	const row = $derived(playback.queue.items[playback.queue.currentIndex]);
	const live = $derived<Track | null>(
		playback.now
			? {
					video_id: playback.now.videoId,
					title: playback.now.title,
					artists: playback.now.artists,
					album:
						playback.now.album ??
						(row?.video_id === playback.now.videoId ? row.album : undefined) ??
						null,
					is_video: !!playback.now.isVideo,
					thumbnail: playback.now.thumbnail
				}
			: null
	);
	const track = $derived(picked ?? live ?? SAMPLE);
	const source = $derived(picked ? 'picked' : live ? 'live' : 'sample');

	// --- The preview itself, from Rust. Out-of-order answers are dropped by sequence number. ---
	let preview = $state<ScrobblePreview | null>(null);
	let seq = 0;
	$effect(() => {
		const config = JSON.stringify({ ...cfg, primary_artist: primaryOn, primary_strict: strictOn });
		const { video_id, title, artists, album, is_video } = track;
		const n = ++seq;
		api.lastfmPreview(config, { video_id, title, artists, album, is_video })
			.then((p) => {
				if (n !== seq) return;
				preview = p;
				if (editWhenReady) {
					editWhenReady = false;
					openEditor();
				}
			})
			.catch(() => {});
	});
	const errors = $derived(new Map(preview?.errors ?? []));

	const at = $derived(source === 'live' ? scrobbleAt(playback.duration, cfg) : null);
	const fmt = (s: number) => {
		s = Math.max(0, Math.round(s));
		return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
	};
	// One line under the mock saying whether this play counts and when. The first reason that
	// applies wins: nothing else matters if neither account is connected.
	const status = $derived.by(() => {
		if (!lastfm.connected && !listenbrainz.connected)
			return t('settings.scrobbling.status_disconnected');
		if (!cfg.enabled) return t('settings.scrobbling.status_paused');
		if (preview?.skip === 'edit') return t('settings.scrobbling.status_skip_edit');
		if (preview?.skip === 'incomplete') return t('settings.scrobbling.status_incomplete');
		if (source !== 'live') return null;
		if (at === null) {
			return playback.duration > 0 ? t('settings.scrobbling.status_short') : null;
		}
		return playback.position >= at
			? t('settings.scrobbling.status_done', { time: fmt(at) })
			: t('settings.scrobbling.status_at', { time: fmt(at) });
	});
	const sending = $derived(
		(lastfm.connected || listenbrainz.connected) && cfg.enabled && !preview?.skip
	);

	// --- Editing one track (#404) ---
	let editing = $state(false);
	let draft = $state({ artist: '', title: '', album: '', skip: false });
	// The in-app edit for this video, if any. A keyless edit (an import) can match it too, which is
	// what `preview.edit` reports; saving here adds a video edit in front of it, so this one wins.
	const ownEdit = $derived(track.video_id ? cfg.edits.findIndex((e) => e.key === track.video_id) : -1);

	function openEditor() {
		// Pinned: the next song starting must not move the edit onto a track the user never opened.
		picked ??= { ...track };
		// What it scrobbles as now, so a fix starts from the rules' output rather than from scratch.
		draft = {
			artist: preview?.artist ?? track.artists,
			title: preview?.title ?? track.title,
			album: preview?.album ?? track.album ?? '',
			skip: preview?.skip === 'edit'
		};
		editing = true;
	}

	function saveEdit() {
		const e: ScrobbleEdit = {
			...blankEdit(),
			key: track.video_id,
			from_artist: track.artists,
			from_title: track.title,
			from_album: track.album ?? '',
			artist: draft.artist.trim(),
			title: draft.title.trim(),
			album: draft.album.trim(),
			skip: draft.skip
		};
		if (ownEdit >= 0) cfg.edits[ownEdit] = e;
		else cfg.edits.unshift(e);
		save();
		closeEditor();
	}

	function removeEdit(i: number) {
		cfg.edits.splice(i, 1);
		save();
		closeEditor();
	}

	// Pinned only for the edit: if it was the song playing, the preview goes back to following it.
	function closeEditor() {
		editing = false;
		if (picked && picked.video_id === live?.video_id) picked = null;
	}

	function backToLive() {
		picked = null;
		editing = false;
	}

	// --- Rules ---
	const FIELDS: Field[] = ['title', 'artist', 'album'];
	const EDIT_ORDER: Field[] = ['artist', 'title', 'album'];
	const FIELD_LABELS = $derived<Record<Field, string>>({
		title: t('settings.scrobbling.field_title'),
		artist: t('settings.scrobbling.field_artist'),
		album: t('settings.scrobbling.field_album')
	});
	const PRESET_LABELS = $derived<Record<string, string>>({
		video_tags: t('settings.scrobbling.preset_video_tags'),
		remastered: t('settings.scrobbling.preset_remastered'),
		explicit: t('settings.scrobbling.preset_explicit'),
		topic: t('settings.scrobbling.preset_topic'),
		vevo: t('settings.scrobbling.preset_vevo')
	});
	let presetsOpen = $state(false);
	function addRule(preset?: (typeof PRESETS)[number]) {
		presetsOpen = false;
		cfg.rules.push({
			enabled: true,
			field: preset?.field ?? 'title',
			find: preset?.find ?? '',
			replace: preset?.replace ?? ''
		});
		save();
	}

	// --- Import (#327) ---
	let fileInput = $state<HTMLInputElement>();
	let importNote = $state<{ text: string; error: boolean } | null>(null);
	async function onImport() {
		const file = fileInput?.files?.[0];
		if (!fileInput || !file) return;
		fileInput.value = '';
		const imp = importScrobbleFile(await file.text());
		if (!imp) {
			importNote = { text: t('settings.scrobbling.import_unknown'), error: true };
			return;
		}
		const n = mergeImport(cfg, imp);
		save();
		const skipped = imp.skipped ? ` ${t('settings.scrobbling.import_skipped', { count: imp.skipped })}` : '';
		importNote = { text: t('settings.scrobbling.imported', { rules: n.rules, edits: n.edits }) + skipped, error: false };
	}

	// --- Edit list. Collapsed past a few rows: an import can bring hundreds. ---
	const EDITS_PREVIEW = 5;
	let showAllEdits = $state(false);
	const shownEdits = $derived(showAllEdits ? cfg.edits : cfg.edits.slice(0, EDITS_PREVIEW));
	const join = (...parts: string[]) => parts.filter((p) => p.trim()).join(' · ');
	function editFrom(e: ScrobbleEdit) {
		return join(e.from_artist, e.from_title, e.key ? '' : e.from_album) || e.key;
	}
	function editTo(e: ScrobbleEdit) {
		if (e.skip) return t('settings.scrobbling.not_scrobbled');
		return join(e.artist || e.from_artist, e.title || e.from_title, e.album);
	}
</script>

{#snippet row_(o: { title: string; desc?: string; control?: Snippet; below?: Snippet })}
	<div class="px-4 py-3.5">
		<div class="flex items-center justify-between gap-4">
			<span class="min-w-0 text-sm font-medium">{o.title}</span>
			{#if o.control}
				<div class="shrink-0">{@render o.control()}</div>
			{/if}
		</div>
		{#if o.desc}
			<p class="mt-1.5 text-xs leading-relaxed text-muted-foreground">{o.desc}</p>
		{/if}
		{#if o.below}
			<div class="mt-3">{@render o.below()}</div>
		{/if}
	</div>
{/snippet}

<div class="flex min-h-0 flex-1 max-lg:flex-col">
	<div class="min-w-0 flex-1 overflow-y-auto px-6 py-5">
		<section class={GROUP}>
			<!-- The account, the way Last.fm's own profile header shows it: avatar, name, and the
			     three counts under it. -->
			<div class="mb-2.5 overflow-hidden rounded-xl border bg-card">
				<div class="flex items-center gap-3.5 px-4 py-4">
					<div
						class="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted ring-1 ring-border"
					>
						{#if lastfm.connected && profile?.image}
							<img src={profile.image} alt="" class="size-full object-cover" draggable="false" />
						{:else if lastfm.connected && lastfm.username}
							<span class="text-base font-semibold text-muted-foreground uppercase">
								{lastfm.username[0]}
							</span>
						{:else}
							<LastFmIcon class="h-5 w-5 text-muted-foreground" />
						{/if}
					</div>
					<div class="min-w-0 flex-1">
						{#if lastfm.connected}
							<button
								type="button"
								class="flex max-w-full cursor-pointer items-center gap-1.5 text-left hover:underline disabled:cursor-default disabled:no-underline"
								disabled={!profile?.url}
								onclick={() => profile?.url && api.openExternal(profile.url)}
								title={t('settings.scrobbling.open_profile')}
							>
								<span class="truncate text-sm font-semibold">{lastfm.username}</span>
								{#if profile?.url}
									<HugeiconsIcon icon={LinkSquare02Icon} size={13} class="shrink-0 text-muted-foreground" />
								{/if}
							</button>
							<p class="truncate text-xs text-muted-foreground">
								{since
									? t('settings.scrobbling.since', { date: since })
									: t('integrations.lastfm_scrobbling_as', { user: lastfm.username ?? '' })}
							</p>
						{:else}
							<p class="text-sm font-semibold">Last.fm</p>
							<p class="text-xs text-muted-foreground">
								{lastfm.connecting
									? t('settings.scrobbling.account_connecting')
									: t('settings.scrobbling.account_disconnected')}
							</p>
						{/if}
					</div>
					{@render accountButton()}
				</div>
				{#if lastfm.connected && STATS.length}
					<div class="grid grid-cols-3 divide-x divide-border/60 border-t border-border/60">
						{#each STATS as s (s.label)}
							<div class="min-w-0 px-4 py-2.5">
								<div
									class="truncate text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase"
								>
									{s.label}
								</div>
								<div class="truncate text-base font-semibold tabular-nums">{num(s.value)}</div>
							</div>
						{/each}
					</div>
				{/if}
			</div>
			<!-- ListenBrainz: same plays, second service. Token auth instead of the browser flow. -->
			<div class="mb-2.5 overflow-hidden rounded-xl border bg-card">
				<div class="flex items-center gap-3.5 px-4 py-4">
					<div
						class="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted ring-1 ring-border"
					>
						{#if listenbrainz.connected && listenbrainz.username}
							<span class="text-base font-semibold text-muted-foreground uppercase">
								{listenbrainz.username[0]}
							</span>
						{:else}
							<HugeiconsIcon icon={MusicNote01Icon} size={20} class="text-muted-foreground" />
						{/if}
					</div>
					<div class="min-w-0 flex-1">
						{#if listenbrainz.connected}
							<button
								type="button"
								class="flex max-w-full cursor-pointer items-center gap-1.5 text-left hover:underline disabled:cursor-default disabled:no-underline"
								disabled={!lbProfile?.url}
								onclick={() => lbProfile?.url && api.openExternal(lbProfile.url)}
								title={t('settings.scrobbling.lb_open_profile')}
							>
								<span class="truncate text-sm font-semibold">{listenbrainz.username}</span>
								{#if lbProfile?.url}
									<HugeiconsIcon icon={LinkSquare02Icon} size={13} class="shrink-0 text-muted-foreground" />
								{/if}
							</button>
							<p class="truncate text-xs text-muted-foreground">
								{t('integrations.listenbrainz_connected_as', { user: listenbrainz.username ?? '' })}
							</p>
						{:else}
							<p class="text-sm font-semibold">ListenBrainz</p>
							<p class="text-xs text-muted-foreground">
								{listenbrainz.connecting
									? t('settings.scrobbling.lb_validating')
									: t('settings.scrobbling.lb_disconnected')}
							</p>
						{/if}
					</div>
					{@render lbAccountButton()}
				</div>
				{#if listenbrainz.connected && lbProfile}
					<div class="grid grid-cols-1 divide-x divide-border/60 border-t border-border/60">
						<div class="min-w-0 px-4 py-2.5">
							<div
								class="truncate text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase"
							>
								{t('settings.scrobbling.stat_listens')}
							</div>
							<div class="truncate text-base font-semibold tabular-nums">{num(lbProfile.listens)}</div>
						</div>
					</div>
				{/if}
				{#if !listenbrainz.connected}
					<div class="border-t border-border/60 px-4 py-3">
						<p class="mb-2 text-xs leading-relaxed text-muted-foreground">
							{t('settings.scrobbling.lb_token_hint')}
							<button
								type="button"
								class="cursor-pointer underline hover:text-foreground"
								onclick={() => api.openExternal('https://listenbrainz.org/settings/')}
							>
								listenbrainz.org
							</button>
						</p>
						<div class="flex gap-2">
							<Input
								class="h-8 font-mono text-xs"
								type="password"
								bind:value={lbToken}
								placeholder={t('settings.scrobbling.lb_token_placeholder')}
								aria-label={t('settings.scrobbling.lb_token_placeholder')}
								spellcheck={false}
								onkeydown={(e) => e.key === 'Enter' && connectLb()}
							/>
							<Button
								size="sm"
								disabled={!lbToken.trim() || listenbrainz.connecting}
								onclick={connectLb}
							>
								{#if listenbrainz.connecting}
									<HugeiconsIcon icon={Loading03Icon} size={15} class="animate-spin" />
								{:else}
									{t('settings.scrobbling.connect')}
								{/if}
							</Button>
						</div>
					</div>
				{/if}
			</div>
			<div class={CARD}>
				{@render row_({
					title: t('settings.scrobbling.enable'),
					desc: t('settings.scrobbling.enable_hint'),
					control: enabledSwitch
				})}
				{@render row_({
					title: t('settings.scrobbling.now_playing'),
					desc: t('settings.scrobbling.now_playing_hint'),
					control: nowPlayingSwitch
				})}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.scrobbling.section_timing')}</h3>
			<div class={CARD}>
				{@render row_({
					title: t('settings.scrobbling.percent'),
					desc: t('settings.scrobbling.percent_hint'),
					control: percentSlider
				})}
				{@render row_({
					title: t('settings.scrobbling.minutes'),
					desc: t('settings.scrobbling.minutes_hint'),
					control: minutesSlider
				})}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.scrobbling.section_metadata')}</h3>
			<div class={CARD}>
				{@render row_({
					title: t('settings.scrobbling.split_titles'),
					desc: t('settings.scrobbling.split_titles_hint'),
					control: splitSwitch
				})}
				{@render row_({
					title: t('settings.playback.lastfm_primary_artist'),
					desc: t('settings.playback.lastfm_primary_artist_hint'),
					control: primarySwitch
				})}
				{#if primaryOn}
					{@render row_({
						title: t('settings.playback.lastfm_primary_strict'),
						desc: t('settings.playback.lastfm_primary_strict_hint'),
						control: strictSwitch
					})}
				{/if}
			</div>
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.scrobbling.section_rules')}</h3>
			<p class={HINT}>{t('settings.scrobbling.rules_hint')}</p>
			{#if cfg.rules.length}
				<div class="{CARD} mb-2.5">
					{#each cfg.rules as rule, i (i)}
						<div class="px-4 py-3 {rule.enabled ? '' : 'opacity-60'}">
							<div class="flex items-center gap-2.5">
								<Switch
									checked={rule.enabled}
									onCheckedChange={(v) => {
										rule.enabled = v;
										save();
									}}
									aria-label={t('settings.scrobbling.rule_enabled')}
								/>
								<Select.Root
									type="single"
									value={rule.field}
									onValueChange={(v) => {
										rule.field = v as Field;
										save();
									}}
								>
									<Select.Trigger class="h-8 w-28" aria-label={t('settings.scrobbling.rule_field')}>
										<span class="flex-1 truncate text-left">{FIELD_LABELS[rule.field]}</span>
									</Select.Trigger>
									<Select.Content>
										{#each FIELDS as f (f)}
											<Select.Item value={f} label={FIELD_LABELS[f]}>{FIELD_LABELS[f]}</Select.Item>
										{/each}
									</Select.Content>
								</Select.Root>
								{#if preview?.rules.includes(i)}
									<span
										class="truncate rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary"
									>
										{t('settings.scrobbling.rule_applied')}
									</span>
								{/if}
								<button
									type="button"
									class="ml-auto shrink-0 cursor-pointer rounded-md p-1 text-muted-foreground transition-colors hover:text-destructive"
									onclick={() => {
										cfg.rules.splice(i, 1);
										save();
									}}
									aria-label={t('settings.scrobbling.rule_remove')}
									title={t('settings.scrobbling.rule_remove')}
								>
									<HugeiconsIcon icon={Delete02Icon} size={15} />
								</button>
							</div>
							<div class="mt-2.5 grid grid-cols-2 gap-2">
								<Input
									class="h-8 font-mono text-xs {errors.has(i) ? 'border-destructive' : ''}"
									value={rule.find}
									oninput={(e) => {
										rule.find = e.currentTarget.value;
										save();
									}}
									placeholder={t('settings.scrobbling.find_placeholder')}
									aria-label={t('settings.scrobbling.find_placeholder')}
									title={t('settings.scrobbling.find_hint')}
									spellcheck={false}
								/>
								<Input
									class="h-8 font-mono text-xs"
									value={rule.replace}
									oninput={(e) => {
										rule.replace = e.currentTarget.value;
										save();
									}}
									placeholder={t('settings.scrobbling.replace_placeholder')}
									aria-label={t('settings.scrobbling.replace_placeholder')}
									spellcheck={false}
								/>
							</div>
							{#if errors.has(i)}
								<!-- The regex crate's message is multi-line with a caret under the
								     column, which only lines up in a monospace face. -->
								<pre
									class="mt-2 overflow-x-auto font-mono text-[11px] leading-snug whitespace-pre text-destructive">{errors.get(i)}</pre>
							{/if}
						</div>
					{/each}
				</div>
			{/if}
			<div class="flex flex-wrap items-center gap-2">
				<Popover.Root bind:open={presetsOpen}>
					<Popover.Trigger>
						{#snippet child({ props })}
							<Button {...props} variant="secondary" size="sm" class="gap-1.5">
								<HugeiconsIcon icon={Add01Icon} size={15} />
								{t('settings.scrobbling.add_rule')}
							</Button>
						{/snippet}
					</Popover.Trigger>
					<Popover.Content align="start" class="w-80 gap-0 p-1">
						<button
							type="button"
							class="flex w-full cursor-pointer rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
							onclick={() => addRule()}
						>
							{t('settings.scrobbling.blank_rule')}
						</button>
						<div class="mx-1 my-1 h-px bg-border"></div>
						{#each PRESETS as p (p.id)}
							<button
								type="button"
								class="flex w-full cursor-pointer rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
								onclick={() => addRule(p)}
							>
								{PRESET_LABELS[p.id]}
							</button>
						{/each}
					</Popover.Content>
				</Popover.Root>
				<Button
					variant="ghost"
					size="sm"
					class="gap-1.5"
					title={t('settings.scrobbling.import_hint')}
					onclick={() => fileInput?.click()}
				>
					<HugeiconsIcon icon={FileImportIcon} size={15} />
					{t('settings.scrobbling.import')}
				</Button>
				<!-- The webview's own file picker: reading one JSON file needs no plugin. -->
				<input
					bind:this={fileInput}
					type="file"
					accept=".json,application/json"
					class="hidden"
					onchange={onImport}
				/>
			</div>
			{#if importNote}
				<p
					class="mt-2 px-1 text-xs leading-relaxed {importNote.error
						? 'text-destructive'
						: 'text-foreground'}"
				>
					{importNote.text}
				</p>
			{/if}
		</section>

		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.scrobbling.section_edits')}</h3>
			<p class={HINT}>{t('settings.scrobbling.edits_hint')}</p>
			<div class={CARD}>
				{#each shownEdits as e, i (i)}
					<div class="flex items-center gap-3 px-4 py-2.5">
						<div class="min-w-0 flex-1">
							<div class="truncate text-xs text-muted-foreground">{editFrom(e)}</div>
							<div class="truncate text-sm {e.skip ? 'text-muted-foreground italic' : ''}">
								{editTo(e)}
							</div>
						</div>
						<button
							type="button"
							class="shrink-0 cursor-pointer rounded-md p-1 text-muted-foreground transition-colors hover:text-destructive"
							onclick={() => removeEdit(i)}
							aria-label={t('settings.scrobbling.edit_remove')}
							title={t('settings.scrobbling.edit_remove')}
						>
							<HugeiconsIcon icon={Cancel01Icon} size={15} />
						</button>
					</div>
				{:else}
					<p class="px-4 py-3.5 text-xs leading-relaxed text-muted-foreground">
						{t('settings.scrobbling.edits_empty')}
					</p>
				{/each}
			</div>
			{#if cfg.edits.length > EDITS_PREVIEW}
				<Button
					variant="ghost"
					size="sm"
					class="mt-1.5 h-7 px-2 text-xs"
					onclick={() => (showAllEdits = !showAllEdits)}
				>
					{showAllEdits
						? t('settings.scrobbling.edits_show_less')
						: t('settings.scrobbling.edits_show_all', { count: cfg.edits.length })}
				</Button>
			{/if}
		</section>
	</div>

	<aside
		class="flex shrink-0 flex-col overflow-y-auto bg-muted/30 px-5 py-5 lg:w-[22rem] lg:border-l max-lg:border-t"
	>
		<div class="mb-2.5 flex items-center gap-2">
			<h3 class="text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
				{t('settings.discord.preview')}
			</h3>
			<span class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
				{source === 'picked'
					? t('settings.scrobbling.preview_picked')
					: source === 'live'
						? t('settings.discord.preview_live')
						: t('settings.discord.preview_sample')}
			</span>
			{#if source === 'picked'}
				<button
					type="button"
					class="ml-auto cursor-pointer text-[11px] text-muted-foreground hover:text-foreground"
					onclick={backToLive}
				>
					{t('settings.scrobbling.preview_back')}
				</button>
			{/if}
		</div>

		<!-- What YouTube Music calls it, and what changed it on the way. -->
		<div class="rounded-xl border border-dashed p-3">
			<p class="mb-1.5 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
				{t('settings.scrobbling.from_youtube')}
			</p>
			<div class="truncate text-sm">{track.title}</div>
			<div class="truncate text-xs text-muted-foreground">
				{join(track.artists, track.album ?? '')}
			</div>
			{#if preview && (preview.edit !== null || preview.split || preview.rules.length)}
				<div class="mt-2 flex flex-wrap gap-1">
					{#if preview.edit !== null}
						<span class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
							{t('settings.scrobbling.applied_edit')}
						</span>
					{/if}
					{#if preview.split}
						<span class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
							{t('settings.scrobbling.applied_split')}
						</span>
					{/if}
					{#each preview.rules as r (r)}
						<span class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
							{t('settings.scrobbling.applied_rule', { n: r + 1 })}
						</span>
					{/each}
				</div>
			{/if}
		</div>

		<HugeiconsIcon icon={ArrowDown02Icon} size={14} class="mx-auto my-2.5 shrink-0 text-muted-foreground" />

		{#if editing}
			<div class="flex flex-col gap-2 rounded-xl border bg-card p-3">
				<p class="text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
					{t('settings.scrobbling.edit_track')}
				</p>
				{#each EDIT_ORDER as k (k)}
					<label class="flex flex-col gap-1">
						<span class="text-xs text-muted-foreground">{FIELD_LABELS[k]}</span>
						<Input class="h-8" bind:value={draft[k]} disabled={draft.skip} spellcheck={false} />
					</label>
				{/each}
				<div class="mt-1 flex items-center justify-between gap-3">
					<span class="text-sm">{t('settings.scrobbling.skip_track')}</span>
					<Switch bind:checked={draft.skip} />
				</div>
				<p class="text-xs leading-relaxed text-muted-foreground">{t('settings.scrobbling.edit_hint')}</p>
				<div class="mt-1 flex items-center gap-2">
					<Button size="sm" onclick={saveEdit}>{t('common.save')}</Button>
					<Button size="sm" variant="ghost" onclick={closeEditor}>{t('common.cancel')}</Button>
					{#if ownEdit >= 0}
						<Button
							size="sm"
							variant="ghost"
							class="ml-auto text-destructive hover:text-destructive"
							onclick={() => removeEdit(ownEdit)}
						>
							{t('settings.scrobbling.edit_remove')}
						</Button>
					{/if}
				</div>
			</div>
		{:else}
			<!-- A row of Last.fm's Recent Tracks list, in the app's theme: what the profile will show. -->
			<div class="rounded-xl border bg-card p-3 {sending ? '' : 'opacity-60'}">
				<div class="flex items-center gap-3">
					<div class="flex size-12 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted">
						{#if track.thumbnail}
							<img src={thumb(track.thumbnail, 96)} alt="" class="size-full object-cover" draggable="false" />
						{:else}
							<HugeiconsIcon icon={MusicNote01Icon} size={20} class="text-muted-foreground" />
						{/if}
					</div>
					<div class="min-w-0 flex-1">
						{#if preview?.skip}
							<div class="truncate text-sm font-semibold text-muted-foreground italic">
								{t('settings.scrobbling.not_scrobbled')}
							</div>
							<div class="truncate text-xs text-muted-foreground">{track.title}</div>
						{:else if preview}
							<div class="truncate text-sm font-semibold">{preview.title}</div>
							<div class="truncate text-xs">{preview.artist}</div>
							{#if preview.album}
								<div class="truncate text-xs text-muted-foreground">{preview.album}</div>
							{/if}
						{:else}
							<HugeiconsIcon icon={Loading03Icon} size={16} class="animate-spin text-muted-foreground" />
						{/if}
					</div>
				</div>
				{#if source === 'live' && at !== null && playback.duration > 0}
					<!-- Where the scrobble fires, against where playback is. -->
					<div class="relative mt-3 h-1 rounded-full bg-muted">
						<div
							class="h-full rounded-full bg-primary"
							style="width: {Math.min(100, (playback.position / playback.duration) * 100)}%"
						></div>
						<div
							class="absolute -top-1 h-3 w-0.5 rounded-full bg-foreground"
							style="left: {Math.min(100, (at / playback.duration) * 100)}%"
						></div>
					</div>
				{/if}
			</div>
			{#if status}
				<p class="mt-2 px-1 text-xs leading-relaxed text-muted-foreground">{status}</p>
			{/if}
			{#if source !== 'sample'}
				<Button variant="secondary" size="sm" class="mt-3 w-fit gap-1.5" onclick={openEditor}>
					<HugeiconsIcon icon={PencilEdit02Icon} size={15} />
					{t('settings.scrobbling.edit_track')}
				</Button>
			{/if}
		{/if}
	</aside>
</div>

{#snippet accountButton()}
	{#if lastfm.connected}
		<Button variant="ghost" size="sm" class="text-destructive hover:text-destructive" onclick={disconnectLastfm}>
			{t('integrations.disconnect')}
		</Button>
	{:else if lastfm.connecting}
		<!-- Cancels the browser authorization, same as a second click on the titlebar button. -->
		<Button variant="ghost" size="sm" class="gap-1.5" onclick={disconnectLastfm}>
			<HugeiconsIcon icon={Loading03Icon} size={15} class="animate-spin" />
			{t('common.cancel')}
		</Button>
	{:else}
		<Button size="sm" onclick={connectLastfm}>{t('settings.scrobbling.connect')}</Button>
	{/if}
{/snippet}
{#snippet lbAccountButton()}
	{#if listenbrainz.connected}
		<Button variant="ghost" size="sm" class="text-destructive hover:text-destructive" onclick={disconnectListenBrainz}>
			{t('integrations.disconnect')}
		</Button>
	{:else if listenbrainz.connecting}
		<Button variant="ghost" size="sm" class="gap-1.5" disabled>
			<HugeiconsIcon icon={Loading03Icon} size={15} class="animate-spin" />
			{t('common.cancel')}
		</Button>
	{/if}
{/snippet}
{#snippet enabledSwitch()}<Switch
		checked={cfg.enabled}
		onCheckedChange={(v) => {
			set({ enabled: v });
			prefs.scrobbling = v;
		}}
	/>{/snippet}
{#snippet nowPlayingSwitch()}<Switch
		checked={cfg.now_playing}
		onCheckedChange={(v) => set({ now_playing: v })}
	/>{/snippet}
{#snippet splitSwitch()}<Switch
		checked={cfg.split_video_titles}
		onCheckedChange={(v) => set({ split_video_titles: v })}
	/>{/snippet}
{#snippet primarySwitch()}<Switch
		checked={primaryOn}
		onCheckedChange={(v) => setRow('lastfm_primary_artist', v)}
	/>{/snippet}
{#snippet strictSwitch()}<Switch
		checked={strictOn}
		onCheckedChange={(v) => setRow('lastfm_primary_strict', v)}
	/>{/snippet}
{#snippet percentSlider()}
	<div class="flex w-44 shrink-0 items-center gap-3">
		<Slider
			type="single"
			aria-label={t('settings.scrobbling.percent')}
			min={10}
			max={100}
			step={5}
			value={cfg.percent}
			onValueChange={(v) => set({ percent: v })}
		/>
		<span class="w-10 shrink-0 text-right font-mono text-xs text-muted-foreground">
			{t('settings.scrobbling.percent_value', { n: cfg.percent })}
		</span>
	</div>
{/snippet}
{#snippet minutesSlider()}
	<div class="flex w-44 shrink-0 items-center gap-3">
		<Slider
			type="single"
			aria-label={t('settings.scrobbling.minutes')}
			min={0}
			max={10}
			step={1}
			value={cfg.minutes}
			onValueChange={(v) => set({ minutes: v })}
		/>
		<span class="w-10 shrink-0 text-right font-mono text-xs text-muted-foreground">
			{cfg.minutes ? t('settings.scrobbling.minutes_value', { n: cfg.minutes }) : t('settings.scrobbling.minutes_off')}
		</span>
	</div>
{/snippet}
