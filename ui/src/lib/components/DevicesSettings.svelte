<script lang="ts">
	// The Devices tab: link Limusic on a phone so the two control each other and move the music
	// between them (src-tauri/src/devicelink.rs). Three states: nothing linked, pairing (steps and
	// the code), and the devices as cards: this computer first, then each linked phone with what it
	// plays and a button to move the music there or back.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ComputerIcon,
		MoreHorizontalIcon,
		MusicNote01Icon,
		SmartPhone01Icon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as Popover from '$lib/components/ui/popover';
	import * as api from '$lib/api';
	import { link } from '$lib/link.svelte';
	import { playback, toast } from '$lib/player.svelte';
	import { makeInvite } from '$lib/invite';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import type { LinkDevice } from '$lib/api';

	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'rounded-xl border bg-card';
	/** A pairing code lives this long on the server (sync-server's PAIR_TTL). */
	const CODE_SECS = 120;
	const RING = 2 * Math.PI * 15;

	/// "Link a phone" was pressed and the code hasn't come back yet.
	let requested = $state(false);
	let now = $state(Date.now());
	let unlinking = $state<LinkDevice | null>(null);
	let confirmOpen = $state(false);
	let menuFor = $state<string | null>(null);

	$effect(() => {
		if (link.pairing || link.status === 'unsupported' || link.error) requested = false;
	});
	$effect(() => {
		if (!link.pairing) return;
		now = Date.now();
		const id = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(id);
	});

	const left = $derived(
		link.pairing ? Math.max(0, Math.ceil((link.pairing.expiresAt - now) / 1000)) : 0
	);
	const countdown = $derived(`${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`);
	const showCode = $derived(!!link.pairing || requested);
	const expired = $derived(!!link.pairing && left === 0);

	// What this computer is doing, for its own card.
	const hereStatus = $derived(
		link.remote && link.player
			? t('settings.devices.remote_for', { name: link.player.name })
			: playback.now
				? playback.paused
					? t('settings.devices.paused')
					: t('settings.devices.playing_here')
				: t('settings.devices.ready')
	);

	function run(p: Promise<void>) {
		p.catch((e) => toast.error(String(e)));
	}
	function startPairing() {
		requested = true;
		run(api.linkStartPairing());
	}
	function cancelPairing() {
		requested = false;
		run(api.linkCancelPairing());
	}
	function askUnlink(d: LinkDevice) {
		menuFor = null;
		unlinking = d;
		confirmOpen = true;
	}
	function unlink() {
		if (unlinking) run(api.linkUnlink(unlinking.id));
		unlinking = null;
	}

	const isPlayer = (d: LinkDevice) => link.player?.id === d.id;
	const status = (d: LinkDevice) => {
		if (!d.online) return t('settings.devices.offline');
		if (isPlayer(d) && link.track)
			return link.isPlaying ? t('settings.devices.playing') : t('settings.devices.paused');
		return t('settings.devices.online');
	};
</script>

{#snippet deviceIcon(icon: typeof ComputerIcon, active: boolean)}
	<span
		class="flex size-10 shrink-0 items-center justify-center rounded-full {active
			? 'bg-primary text-primary-foreground'
			: 'bg-muted text-muted-foreground'}"
	>
		<HugeiconsIcon {icon} class="size-5" />
	</span>
{/snippet}

{#snippet nowPlaying(cover: string | null | undefined, title: string, artist: string, line: string)}
	<div class="mt-3 flex items-center gap-3 rounded-lg bg-muted/50 p-2">
		{#if cover}
			<img src={thumb(cover, 80)} alt="" class="size-10 shrink-0 rounded-md object-cover" />
		{:else}
			<span
				class="flex size-10 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground/60"
			>
				<HugeiconsIcon icon={MusicNote01Icon} class="size-4" />
			</span>
		{/if}
		<div class="min-w-0 flex-1">
			<p class="truncate text-sm font-medium">{title}</p>
			<p class="truncate text-xs text-muted-foreground">{artist} · {line}</p>
		</div>
	</div>
{/snippet}

<div class="relative min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-6 py-5">
	<header class="mb-7 flex items-center gap-4">
		<div class="flex shrink-0 items-center" aria-hidden="true">
			<span class="flex size-11 items-center justify-center rounded-full bg-primary/15 text-primary">
				<HugeiconsIcon icon={ComputerIcon} class="size-5" />
			</span>
			<span class="w-6 border-t-2 border-dashed border-primary/40"></span>
			<span class="flex size-11 items-center justify-center rounded-full bg-primary/15 text-primary">
				<HugeiconsIcon icon={SmartPhone01Icon} class="size-5" />
			</span>
		</div>
		<div class="min-w-0">
			<h3 class="text-base font-semibold">{t('settings.devices.title')}</h3>
			<p class="text-sm text-muted-foreground">{t('settings.devices.subtitle')}</p>
		</div>
	</header>

	{#if showCode}
		<section class={GROUP}>
			<div class="{CARD} px-6 py-5">
				<h4 class="text-sm font-semibold">{t('settings.devices.link_phone')}</h4>
				<ol class="mt-4 space-y-5">
					<li class="flex gap-3">
						<span
							class="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary"
							>1</span
						>
						<p class="pt-0.5 text-sm">{t('settings.devices.step_open')}</p>
					</li>
					<li class="flex gap-3">
						<span
							class="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary"
							>2</span
						>
						<div class="min-w-0 flex-1">
							<p class="pt-0.5 text-sm">{t('settings.devices.step_type')}</p>
							<div class="mt-3 flex items-center gap-5" aria-live="polite">
								<div class="flex gap-1.5 {expired ? 'opacity-40' : ''}">
									<!-- The letter is read inside the tile: as each items, values that change under
									     the same key are never written back (Svelte 5 compiles them fixed), so tiles
									     drawn before the code arrived stayed empty. -->
									{#each [0, 1, 2, 3, 4, 5] as i (i)}
										{@const ch = link.pairing?.code[i] ?? ''}
										<span
											class="flex h-14 w-11 items-center justify-center rounded-lg border bg-muted/40 font-mono text-2xl font-semibold {ch
												? ''
												: 'animate-pulse'}">{ch}</span
										>
									{/each}
								</div>
								{#if link.pairing && !expired}
									<div class="relative size-10 shrink-0" title={t('settings.devices.expires_in', { time: countdown })}>
										<svg viewBox="0 0 36 36" class="size-10 -rotate-90">
											<circle cx="18" cy="18" r="15" fill="none" stroke-width="3" class="stroke-muted" />
											<circle
												cx="18"
												cy="18"
												r="15"
												fill="none"
												stroke-width="3"
												stroke-linecap="round"
												class="stroke-primary transition-[stroke-dashoffset] duration-1000 ease-linear"
												stroke-dasharray={RING}
												stroke-dashoffset={RING * (1 - Math.min(left, CODE_SECS) / CODE_SECS)}
											/>
										</svg>
										<span
											class="absolute inset-0 flex items-center justify-center text-[10px] tabular-nums text-muted-foreground"
											>{countdown}</span
										>
									</div>
								{/if}
							</div>
							{#if expired}
								<p class="mt-2 text-xs text-muted-foreground">{t('settings.devices.expired')}</p>
							{:else if link.pairing && link.server}
								<!-- On a server of your own the phone needs it too: the invite, not just the code. -->
								<p class="mt-2 select-all text-xs text-muted-foreground">
									{t('settings.devices.custom_server', {
										invite: makeInvite(link.server, link.pairing.code)
									})}
								</p>
							{/if}
						</div>
					</li>
				</ol>
				<div class="mt-6 flex items-center gap-2 pl-9">
					{#if expired}
						<Button onclick={startPairing}>{t('settings.devices.new_code')}</Button>
					{/if}
					<Button variant="ghost" onclick={cancelPairing}>{t('common.cancel')}</Button>
				</div>
			</div>
		</section>
	{:else if link.devices.length > 0}
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.devices.this_computer')}</h3>
			<div class="{CARD} p-4">
				<div class="flex items-center gap-3">
					{@render deviceIcon(ComputerIcon, !link.remote && !!playback.now && !playback.paused)}
					<div class="min-w-0 flex-1">
						<p class="truncate text-sm font-medium">{link.selfName}</p>
						<p class="truncate text-xs text-muted-foreground">{hereStatus}</p>
					</div>
					{#if link.remote}
						<Button size="sm" onclick={() => run(api.linkPlayOn(''))}>
							{t('settings.devices.play_here')}
						</Button>
					{/if}
				</div>
				{#if !link.remote && playback.now}
					{@render nowPlaying(
						playback.now.thumbnail,
						playback.now.title,
						playback.now.artists,
						playback.paused ? t('settings.devices.paused') : t('settings.devices.playing')
					)}
				{/if}
			</div>
		</section>
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.devices.linked')}</h3>
			<div class="space-y-2">
				{#each link.devices as d (d.id)}
					<div class="{CARD} p-4 {d.online ? '' : 'opacity-70'}">
						<div class="flex items-center gap-3">
							{@render deviceIcon(SmartPhone01Icon, isPlayer(d) && link.isPlaying && d.online)}
							<div class="min-w-0 flex-1">
								<p class="truncate text-sm font-medium">{d.name}</p>
								<p class="flex items-center gap-1.5 text-xs text-muted-foreground">
									<span
										class="size-1.5 shrink-0 rounded-full {d.online
											? 'bg-primary'
											: 'bg-muted-foreground/40'}"
									></span>
									<span class="truncate">{status(d)}</span>
								</p>
							</div>
							{#if d.online && !(link.remote && isPlayer(d))}
								<Button size="sm" variant="secondary" onclick={() => run(api.linkPlayOn(d.id))}>
									{t('settings.devices.play_on_phone')}
								</Button>
							{/if}
							<Popover.Root open={menuFor === d.id} onOpenChange={(o) => (menuFor = o ? d.id : null)}>
								<Popover.Trigger
									class="inline-flex size-8 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground"
									aria-label={t('settings.devices.more')}
								>
									<HugeiconsIcon icon={MoreHorizontalIcon} class="size-4" />
								</Popover.Trigger>
								<Popover.Content align="end" class="w-44 p-1">
									<button
										class="w-full cursor-pointer rounded-md px-2.5 py-1.5 text-left text-sm text-destructive transition hover:bg-muted"
										onclick={() => askUnlink(d)}
									>
										{t('settings.devices.unlink')}
									</button>
								</Popover.Content>
							</Popover.Root>
						</div>
						{#if isPlayer(d) && link.track}
							{@render nowPlaying(
								link.track.thumbnail,
								link.track.title,
								link.track.artist,
								link.isPlaying ? t('settings.devices.playing') : t('settings.devices.paused')
							)}
						{/if}
					</div>
				{/each}
			</div>
			{#if link.status === 'unsupported'}
				<p class="mt-3 px-1 text-sm text-destructive">{t('settings.devices.unsupported')}</p>
			{:else if link.status === 'connecting'}
				<p class="mt-3 px-1 text-xs text-muted-foreground">{t('settings.devices.connecting')}</p>
			{/if}
		</section>
		<Button variant="outline" onclick={startPairing}>+ {t('settings.devices.link_another')}</Button>
	{:else}
		<section class={GROUP}>
			<div class="{CARD} flex flex-col items-start gap-4 px-6 py-5">
				<p class="max-w-md text-sm text-muted-foreground">{t('settings.devices.intro')}</p>
				{#if link.status === 'unsupported'}
					<p class="text-sm text-destructive">{t('settings.devices.unsupported')}</p>
				{:else if link.error}
					<p class="text-sm text-destructive">{link.error}</p>
				{/if}
				<Button onclick={startPairing}>
					{link.status === 'unsupported'
						? t('settings.devices.try_again')
						: t('settings.devices.link_phone')}
				</Button>
			</div>
		</section>
	{/if}
</div>

<AlertDialog.Root bind:open={confirmOpen}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Media class="text-muted-foreground">
				<HugeiconsIcon icon={SmartPhone01Icon} />
			</AlertDialog.Media>
			<AlertDialog.Title>
				{t('settings.devices.unlink_title', { name: unlinking?.name ?? '' })}
			</AlertDialog.Title>
			<AlertDialog.Description>{t('settings.devices.unlink_desc')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action onclick={unlink}>{t('settings.devices.unlink')}</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
