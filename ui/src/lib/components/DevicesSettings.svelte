<script lang="ts">
	// The Devices tab: link Limusic on a phone so it can control this desktop and take over what's
	// playing (src-tauri/src/devicelink.rs). Three states: nothing linked, showing a pairing code,
	// and the list of linked devices.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { LaptopPhoneSyncIcon, SmartPhone01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as api from '$lib/api';
	import { link } from '$lib/link.svelte';
	import { toast } from '$lib/player.svelte';
	import { makeInvite } from '$lib/invite';
	import { t } from '$lib/i18n.svelte';
	import type { LinkDevice } from '$lib/api';

	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';

	/// "Link a phone" was pressed and the code hasn't come back yet.
	let requested = $state(false);
	let now = $state(Date.now());
	let unlinking = $state<LinkDevice | null>(null);
	let confirmOpen = $state(false);

	$effect(() => {
		if (link.pairing || link.status === 'unsupported' || link.error) requested = false;
	});
	$effect(() => {
		if (!link.pairing) return;
		now = Date.now();
		const id = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(id);
	});

	const left = $derived(link.pairing ? Math.max(0, Math.ceil((link.pairing.expiresAt - now) / 1000)) : 0);
	const countdown = $derived(`${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`);
	const showCode = $derived(!!link.pairing || requested);

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
		unlinking = d;
		confirmOpen = true;
	}
	function unlink() {
		if (unlinking) run(api.linkUnlink(unlinking.id));
		unlinking = null;
	}
</script>

<div class="relative min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-6 py-5">
	{#if showCode}
		<section class={GROUP}>
			<div class="{CARD} flex flex-col items-center gap-4 px-6 py-8 text-center">
				{#if link.pairing && left > 0}
					<p
						class="select-all font-mono text-4xl font-semibold tracking-[0.2em]"
						aria-live="polite"
					>
						{makeInvite(link.server, link.pairing.code)}
					</p>
					<p class="max-w-sm text-sm text-muted-foreground">{t('settings.devices.type_code')}</p>
					<p class="text-xs tabular-nums text-muted-foreground">
						{t('settings.devices.expires_in', { time: countdown })}
					</p>
				{:else if link.pairing}
					<p class="text-sm text-muted-foreground">{t('settings.devices.expired')}</p>
					<Button onclick={startPairing}>{t('settings.devices.new_code')}</Button>
				{:else}
					<p class="text-sm text-muted-foreground">{t('settings.devices.getting_code')}</p>
				{/if}
				<Button variant="ghost" onclick={cancelPairing}>{t('common.cancel')}</Button>
			</div>
		</section>
	{:else if link.devices.length > 0}
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.devices.linked')}</h3>
			<div class={CARD}>
				{#each link.devices as d (d.id)}
					<div class="flex items-center gap-3 px-4 py-3">
						<HugeiconsIcon icon={SmartPhone01Icon} class="size-5 shrink-0 text-muted-foreground" />
						<div class="min-w-0 flex-1">
							<p class="truncate text-sm font-medium">{d.name}</p>
							<p class="text-xs text-muted-foreground">
								{d.online ? t('settings.devices.online') : t('settings.devices.offline')}
							</p>
						</div>
						{#if d.playing}
							<span class="rounded-full bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">
								{t('settings.devices.playing')}
							</span>
						{/if}
						<Button variant="outline" size="sm" onclick={() => askUnlink(d)}>
							{t('settings.devices.unlink')}
						</Button>
					</div>
				{/each}
			</div>
			{#if link.status === 'unsupported'}
				<p class="mt-3 px-1 text-sm text-destructive">{t('settings.devices.unsupported')}</p>
			{:else if link.status === 'connecting'}
				<p class="mt-3 px-1 text-xs text-muted-foreground">{t('settings.devices.connecting')}</p>
			{/if}
		</section>
		<Button variant="outline" onclick={startPairing}>{t('settings.devices.link_another')}</Button>
	{:else}
		<section class={GROUP}>
			<div class="{CARD} flex flex-col items-center gap-4 px-6 py-8 text-center">
				<HugeiconsIcon icon={LaptopPhoneSyncIcon} class="size-8 text-muted-foreground" />
				<p class="max-w-sm text-sm text-muted-foreground">{t('settings.devices.intro')}</p>
				{#if link.status === 'unsupported'}
					<p class="text-sm text-destructive">{t('settings.devices.unsupported')}</p>
				{:else if link.error}
					<p class="text-sm text-destructive">{link.error}</p>
				{/if}
				<Button onclick={startPairing}>
					{link.status === 'unsupported' ? t('settings.devices.try_again') : t('settings.devices.link_phone')}
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
