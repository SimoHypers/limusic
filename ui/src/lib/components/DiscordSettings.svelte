<script lang="ts">
	// The Discord tab: every Rich Presence control, with a live mock of the card beside them. The
	// options are abstract on their own, so the preview is not decoration — it is how you tell what
	// "line 2 shows the artist and album" actually does to your profile.
	//
	// Saving is optimistic and debounced: the preview follows the switch instantly, the blob reaches
	// SQLite once typing stops, and the presence thread re-pushes the card at once from there
	// (`set_discord_config`). Discord drops updates that arrive too close together, which is the
	// other reason not to write on every keystroke.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowTurnBackwardIcon } from '@hugeicons/core-free-icons';
	import { untrack, type Snippet } from 'svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import * as Select from '$lib/components/ui/select';
	import * as api from '$lib/api';
	import { playback, prefs, toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		DISCORD_DEFAULTS,
		parseDiscordConfig,
		type DiscordConfig,
		type PreviewTrack
	} from '$lib/discord';
	import DiscordCard from '$lib/components/DiscordCard.svelte';

	let { settings }: { settings: Record<string, string> } = $props();

	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';

	// `prefs`, not the local `settings` record: the titlebar has its own Discord button, and two
	// copies of the same flag meant toggling one left the other's indicator stale.
	const enabled = $derived(prefs.discordRpc);
	// Seeded once from the record the modal already loaded, then owned here. Reading it in a
	// $derived instead would fight the user: every keystroke would be overwritten by the last saved
	// blob until the debounce landed.
	let cfg = $state<DiscordConfig>(untrack(() => parseDiscordConfig(settings.discord_rpc_config)));

	let saveTimer: ReturnType<typeof setTimeout> | undefined;
	function save() {
		const json = JSON.stringify(cfg);
		settings.discord_rpc_config = json;
		clearTimeout(saveTimer);
		saveTimer = setTimeout(() => {
			api.setSetting('discord_rpc_config', json).catch((e) => toast.error(String(e)));
		}, 250);
	}

	function set(patch: Partial<DiscordConfig>) {
		Object.assign(cfg, patch);
		save();
	}

	async function setEnabled(on: boolean) {
		prefs.discordRpc = on;
		settings.discord_rpc = on ? 'true' : 'false';
		try {
			await api.setSetting('discord_rpc', settings.discord_rpc);
		} catch (e) {
			prefs.discordRpc = !on;
			toast.error(String(e));
		}
	}

	function reset() {
		cfg = { ...DISCORD_DEFAULTS };
		save();
	}

	const isDefault = $derived(JSON.stringify(cfg) === JSON.stringify(DISCORD_DEFAULTS));

	// --- Option lists. The values are the strings `discord.rs` matches on. ---
	const CONTENT = $derived({
		title: t('settings.discord.content_title'),
		artist: t('settings.discord.content_artist'),
		album: t('settings.discord.content_album'),
		artist_album: t('settings.discord.content_artist_album'),
		off: t('settings.discord.content_off')
	} as Record<string, string>);

	const LINE1_OPTIONS = ['title', 'artist', 'album'];
	const LINE2_OPTIONS = ['artist', 'artist_album', 'album', 'title', 'off'];
	const LINE3_OPTIONS = ['album', 'artist', 'title', 'off'];
	const BUTTON_OPTIONS = ['listen', 'album', 'artist', 'app', 'off'];

	const BUTTONS = $derived({
		listen: t('settings.discord.button_listen'),
		album: t('settings.discord.button_album'),
		artist: t('settings.discord.button_artist'),
		app: t('settings.discord.button_app'),
		off: t('settings.discord.button_off')
	} as Record<string, string>);

	// The status dropdown picks a *slot*, so each option says what that slot currently holds —
	// "Line 2 · Artist" moves with the Line 2 setting above it, and the preview confirms it.
	const STATUS_OPTIONS = $derived([
		{ id: 'app', label: t('settings.discord.status_app') },
		{ id: 'line1', label: `${t('settings.discord.status_line1')} · ${CONTENT[cfg.line1]}` },
		{ id: 'line2', label: `${t('settings.discord.status_line2')} · ${CONTENT[cfg.line2]}` }
	]);
	const statusLabel = $derived(
		STATUS_OPTIONS.find((o) => o.id === cfg.status_line)?.label ?? STATUS_OPTIONS[0].label
	);

	// --- What the preview renders: the real track when one is playing, a stand-in otherwise. ---
	// `now` carries no album, so the matching queue row supplies it (and the album's browseId, which
	// is what an "open the album" link needs).
	const row = $derived(playback.queue.items[playback.queue.currentIndex]);
	const SAMPLE: PreviewTrack = {
		videoId: 'dQw4w9WgXcQ',
		title: 'Higher Ground',
		artists: 'Nova Sky',
		artistId: 'UCsample',
		album: 'Night Signals',
		albumId: 'MPREsample'
	};
	const track = $derived<PreviewTrack>(
		playback.now
			? {
					videoId: playback.now.videoId,
					title: playback.now.title,
					artists: playback.now.artists,
					artistId: playback.now.artistId,
					album: row?.video_id === playback.now.videoId ? row.album : undefined,
					albumId: row?.video_id === playback.now.videoId ? row.album_id : undefined,
					thumbnail: playback.now.thumbnail,
					local: api.isLocalId(playback.now.videoId)
				}
			: SAMPLE
	);
	// A sample track needs a plausible bar; a real one uses the position the player already ticks.
	const previewPosition = $derived(playback.now ? playback.position : 66);
	const previewDuration = $derived(playback.now ? playback.duration : 204);
	const previewPaused = $derived(!!playback.now && playback.paused);
</script>

{#snippet row_(o: { title: string; desc?: string; control?: Snippet; below?: Snippet })}
	<!-- The description sits *under* the control, not beside it. These hints are long (they have to
	     explain what a Discord slot is), and boxing them into the column left of the dropdown wasted
	     the whole width under it and turned every one into six short lines. -->
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

{#snippet picker(value: string, options: string[], labels: Record<string, string>, aria: string, onpick: (v: string) => void)}
	<Select.Root type="single" {value} onValueChange={onpick}>
		<Select.Trigger class="w-48 shrink-0" aria-label={aria}>
			<span class="flex-1 truncate text-left">{labels[value] ?? value}</span>
		</Select.Trigger>
		<Select.Content>
			{#each options as id (id)}
				<Select.Item value={id} label={labels[id]}>{labels[id]}</Select.Item>
			{/each}
		</Select.Content>
	</Select.Root>
{/snippet}

<!-- Options scroll, the preview does not. Side by side from `lg` up (where the modal is wide
     enough for both). Below `lg` the card would keep its height and squeeze the controls to
     nothing, so the wrapper becomes the scroller and the preview lands at the end of the list. -->
<div class="flex min-h-0 flex-1 max-lg:flex-col max-lg:overflow-y-auto max-lg:overflow-x-hidden">
	<div class="min-w-0 flex-1 overflow-y-auto px-6 py-5 max-lg:flex-none max-lg:overflow-visible">
		<section class={GROUP}>
			<div class={CARD}>
				{@render row_({
					title: t('settings.discord.enable'),
					desc: t('settings.discord.enable_hint'),
					control: enableSwitch
				})}
				{@render row_({
					title: t('settings.discord.show_paused'),
					desc: t('settings.discord.show_paused_hint'),
					control: pausedSwitch
				})}
				{@render row_({
					title: t('settings.discord.hide_details'),
					desc: t('settings.discord.hide_details_hint'),
					control: hideDetailsSwitch
				})}
			</div>
		</section>

		<!-- Everything below only shapes the card, so it is dimmed while presence is off and gone
		     entirely under "hide details", where none of it reaches Discord at all. Leaving dead
		     controls on screen would be the more confusing choice. -->
		{#if !cfg.hide_details}
		<div class={enabled ? '' : 'opacity-60'}>
			<section class={GROUP}>
				<h3 class={LABEL}>{t('settings.sections.discord_status')}</h3>
				<div class={CARD}>
					{@render row_({
						title: t('settings.discord.status_line'),
						desc: t('settings.discord.status_line_hint'),
						control: statusPicker
					})}
					{@render row_({
						title: t('settings.discord.app_name'),
						desc: t('settings.discord.app_name_hint'),
						control: appNameInput
					})}
				</div>
			</section>

			<section class={GROUP}>
				<h3 class={LABEL}>{t('settings.sections.discord_card')}</h3>
				<div class={CARD}>
					{@render row_({
						title: t('settings.discord.line1'),
						desc: t('settings.discord.line1_hint'),
						control: line1Picker
					})}
					{@render row_({
						title: t('settings.discord.line2'),
						desc: t('settings.discord.line2_hint'),
						control: line2Picker
					})}
					{@render row_({
						title: t('settings.discord.cover'),
						desc: t('settings.discord.cover_hint'),
						control: coverSwitch
					})}
					{#if cfg.cover}
						{@render row_({
							title: t('settings.discord.badge'),
							desc: t('settings.discord.badge_hint'),
							control: badgeSwitch
						})}
						{@render row_({
							title: t('settings.discord.line3'),
							desc: t('settings.discord.line3_hint'),
							control: line3Picker
						})}
					{/if}
					{@render row_({
						title: t('settings.discord.timestamps'),
						desc: t('settings.discord.timestamps_hint'),
						control: timestampsSwitch
					})}
				</div>
			</section>

			<section class={GROUP}>
				<h3 class={LABEL}>{t('settings.sections.discord_links')}</h3>
				<p class="mb-2 px-1 max-w-prose text-xs leading-relaxed text-muted-foreground">
					{t('settings.discord.links_hint')}
				</p>
				<div class={CARD}>
					{@render row_({ title: t('settings.discord.link_line1'), control: link1Switch })}
					{@render row_({ title: t('settings.discord.link_line2'), control: link2Switch })}
					{#if cfg.cover}
						{@render row_({ title: t('settings.discord.link_cover'), control: linkCoverSwitch })}
					{/if}
				</div>
			</section>

			<section class={GROUP}>
				<h3 class={LABEL}>{t('settings.sections.discord_buttons')}</h3>
				<p class="mb-2 px-1 max-w-prose text-xs leading-relaxed text-muted-foreground">
					{t('settings.discord.buttons_hint')}
				</p>
				<div class={CARD}>
					{@render row_({ title: t('settings.discord.button1'), control: button1Picker })}
					{@render row_({ title: t('settings.discord.button2'), control: button2Picker })}
				</div>
			</section>
		</div>
		{/if}
	</div>

	<aside
		class="flex shrink-0 flex-col bg-muted/30 px-5 py-5 lg:w-[22rem] lg:border-l max-lg:border-t"
	>
		<div class="mb-2.5 flex items-center gap-2">
			<h3 class="text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
				{t('settings.discord.preview')}
			</h3>
			<span class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
				{playback.now ? t('settings.discord.preview_live') : t('settings.discord.preview_sample')}
			</span>
		</div>
		<DiscordCard
			{cfg}
			{track}
			{enabled}
			paused={previewPaused}
			position={previewPosition}
			duration={previewDuration}
		/>
		<Button
			variant="ghost"
			size="sm"
			class="mt-3 h-7 w-fit gap-1.5 px-2 text-xs lg:mt-auto"
			disabled={isDefault}
			onclick={reset}
		>
			<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-3.5 w-3.5" />
			{t('settings.discord.reset')}
		</Button>
	</aside>
</div>

{#snippet enableSwitch()}<Switch checked={enabled} onCheckedChange={setEnabled} />{/snippet}
{#snippet pausedSwitch()}<Switch
		checked={cfg.show_paused}
		onCheckedChange={(v) => set({ show_paused: v })}
	/>{/snippet}
{#snippet hideDetailsSwitch()}<Switch
		checked={cfg.hide_details}
		onCheckedChange={(v) => set({ hide_details: v })}
	/>{/snippet}
{#snippet badgeSwitch()}<Switch checked={cfg.badge} onCheckedChange={(v) => set({ badge: v })} />{/snippet}
{#snippet coverSwitch()}<Switch checked={cfg.cover} onCheckedChange={(v) => set({ cover: v })} />{/snippet}
{#snippet timestampsSwitch()}<Switch
		checked={cfg.timestamps}
		onCheckedChange={(v) => set({ timestamps: v })}
	/>{/snippet}
{#snippet link1Switch()}<Switch
		checked={cfg.link_line1}
		onCheckedChange={(v) => set({ link_line1: v })}
	/>{/snippet}
{#snippet link2Switch()}<Switch
		checked={cfg.link_line2}
		onCheckedChange={(v) => set({ link_line2: v })}
	/>{/snippet}
{#snippet linkCoverSwitch()}<Switch
		checked={cfg.link_cover}
		onCheckedChange={(v) => set({ link_cover: v })}
	/>{/snippet}

{#snippet statusPicker()}
	<Select.Root
		type="single"
		value={cfg.status_line}
		onValueChange={(v) => set({ status_line: v })}
	>
		<Select.Trigger class="w-48 shrink-0" aria-label={t('settings.discord.status_line')}>
			<span class="flex-1 truncate text-left">{statusLabel}</span>
		</Select.Trigger>
		<Select.Content>
			{#each STATUS_OPTIONS as o (o.id)}
				<Select.Item value={o.id} label={o.label}>{o.label}</Select.Item>
			{/each}
		</Select.Content>
	</Select.Root>
{/snippet}

{#snippet appNameInput()}
	<Input
		class="w-48"
		value={cfg.app_name}
		oninput={(e) => set({ app_name: e.currentTarget.value })}
		placeholder={t('settings.discord.app_name_placeholder')}
		aria-label={t('settings.discord.app_name')}
		spellcheck={false}
	/>
{/snippet}

{#snippet line1Picker()}{@render picker(cfg.line1, LINE1_OPTIONS, CONTENT, t('settings.discord.line1'), (v) => set({ line1: v }))}{/snippet}
{#snippet line2Picker()}{@render picker(cfg.line2, LINE2_OPTIONS, CONTENT, t('settings.discord.line2'), (v) => set({ line2: v }))}{/snippet}
{#snippet line3Picker()}{@render picker(cfg.line3, LINE3_OPTIONS, CONTENT, t('settings.discord.line3'), (v) => set({ line3: v }))}{/snippet}
{#snippet button1Picker()}{@render picker(cfg.button1, BUTTON_OPTIONS, BUTTONS, t('settings.discord.button1'), (v) => set({ button1: v }))}{/snippet}
{#snippet button2Picker()}{@render picker(cfg.button2, BUTTON_OPTIONS, BUTTONS, t('settings.discord.button2'), (v) => set({ button2: v }))}{/snippet}
