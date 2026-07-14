<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Alert, AlertDescription } from '$lib/components/ui/alert';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as api from '$lib/api';
	import { ui, toast } from '$lib/player.svelte';
	import { THEMES, theme, applyTheme } from '$lib/theme.svelte';
	import { updateState, checkForUpdatesInteractive, installUpdate } from '$lib/updater.svelte';
	import { getVersion } from '@tauri-apps/api/app';

	type TabId = 'general' | 'playback' | 'data' | 'about';
	const TABS: { id: TabId; label: string }[] = [
		{ id: 'general', label: 'General' },
		{ id: 'playback', label: 'Playback' },
		{ id: 'data', label: 'Data & storage' },
		{ id: 'about', label: 'About' }
	];

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
	$effect(() => {
		if (ui.settingsOpen) {
			load();
			updateResult = null;
		}
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
			toast(String(e));
		}
		loaded = true;
	}

	const quality = $derived(settings.quality ?? 'HIGH');
	const historyOn = $derived(settings.enable_history !== 'false');
	const discordOn = $derived(settings.discord_rpc === 'true');
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
		toast('Audio quality updated');
	}

	async function setHistory(on: boolean) {
		settings.enable_history = on ? 'true' : 'false';
		await api.setSetting('enable_history', settings.enable_history);
	}

	async function setDiscord(on: boolean) {
		settings.discord_rpc = on ? 'true' : 'false';
		await api.setSetting('discord_rpc', settings.discord_rpc);
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
		toast('Proxy saved — restart to apply');
	}

	async function doClearCaches() {
		clearing = true;
		try {
			await api.clearCaches();
			toast('Caches cleared');
		} finally {
			clearing = false;
		}
	}
</script>

<Dialog.Root bind:open={ui.settingsOpen}>
	<Dialog.Content class="gap-0 overflow-hidden p-0 sm:max-w-3xl">
		<div class="flex items-center border-b px-6 py-4">
			<Dialog.Title class="text-lg font-semibold">Settings</Dialog.Title>
			<Dialog.Description class="sr-only">Application settings</Dialog.Description>
		</div>

		<div class="flex h-[28rem]">
			<!-- Tab rail -->
			<nav class="w-48 shrink-0 border-r p-2">
				{#each TABS as t (t.id)}
					<button
						onclick={() => (tab = t.id)}
						class="w-full rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors {tab ===
						t.id
							? 'bg-accent text-accent-foreground'
							: 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'}"
					>
						{t.label}
					</button>
				{/each}
			</nav>

			<!-- Content pane -->
			<div class="flex-1 overflow-y-auto px-6 py-4">
				{#if !loaded}
					<p class="text-sm text-muted-foreground">Loading…</p>
				{:else if tab === 'general'}
					<div class="border-b py-3">
						<div class="font-medium">Theme</div>
						<p class="mt-0.5 mb-3 text-sm text-muted-foreground">
							Accent colors tint the default look; presets swap the whole palette.
						</p>
						<div role="radiogroup" aria-label="Theme" class="grid grid-cols-2 gap-2 sm:grid-cols-3">
							{#each THEMES as t (t.id)}
								<label
									class="flex cursor-pointer items-center gap-3 rounded-lg border p-3 transition-colors hover:bg-accent/10 focus-within:ring-2 focus-within:ring-ring {theme.id ===
									t.id
										? 'border-primary'
										: 'border-border'}"
								>
									<input
										type="radio"
										name="accent-theme"
										value={t.id}
										checked={theme.id === t.id}
										onchange={() => applyTheme(t.id)}
										class="sr-only"
									/>
									<span
										class="h-6 w-6 shrink-0 rounded-full ring-1 ring-black/10"
										style="background:{t.color}"
									></span>
									<span class="flex-1 text-sm font-medium">{t.label}</span>
									<span
										class="flex h-4 w-4 shrink-0 items-center justify-center rounded-full border {theme.id ===
										t.id
											? 'border-primary'
											: 'border-muted-foreground/50'}"
									>
										{#if theme.id === t.id}
											<span class="h-2 w-2 rounded-full bg-primary"></span>
										{/if}
									</span>
								</label>
							{/each}
						</div>
					</div>
					<div class="flex items-start justify-between gap-4 border-b py-3">
						<div class="min-w-0">
							<div class="font-medium">Watch history</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Register plays in your YouTube Music history. Needs sign-in.
							</p>
						</div>
						<Switch checked={historyOn} onCheckedChange={setHistory} />
					</div>
					<div class="flex items-start justify-between gap-4 py-3">
						<div class="min-w-0">
							<div class="font-medium">Discord rich presence</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								Show what you're listening to on your Discord profile. Needs the Discord desktop app
								running — no login here.
							</p>
						</div>
						<Switch checked={discordOn} onCheckedChange={setDiscord} />
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
					<div class="flex items-center justify-between gap-4 py-3">
						<div class="min-w-0">
							<div class="font-medium">Updates</div>
							<p class="mt-0.5 text-sm text-muted-foreground">
								{#if updateState.available}
									Version {updateState.available.version} is available.
								{:else}
									Check GitHub for a newer release.
								{/if}
							</p>
						</div>
						{#if updateState.available}
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
				{/if}
			</div>
		</div>
	</Dialog.Content>
</Dialog.Root>
