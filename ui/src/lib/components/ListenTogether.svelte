<script lang="ts">
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Copy01Icon,
		Logout01Icon,
		Tick02Icon,
		Cancel01Icon,
		UserRemove01Icon,
		Exchange01Icon,
		CrownIcon,
		RefreshIcon,
		UserGroupIcon,
		ServerStack01Icon,
		PlusSignIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { copyText } from '$lib/clipboard';
	import { ui, toast } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';
	import { t } from '$lib/i18n.svelte';

	let mode = $state<'join' | 'host'>('join');
	let name = $state('');
	let serverUrl = $state('');
	let showServer = $state(false);
	let inviteInput = $state('');
	let busy = $state(false);
	let copied = $state(false);

	// Seed inputs when the modal opens: remembered name + the persisted server URL. `serverUrl` is
	// empty on the default server, which is the whole point: the address only appears once the user
	// opens the panel below and asks for it.
	$effect(() => {
		if (ui.ltOpen) {
			name = localStorage.getItem('lt_name') ?? '';
			serverUrl = lt.serverUrl;
			showServer = false;
			copied = false;
		}
	});

	const inRoom = $derived(lt.role !== 'none');
	const isHost = $derived(lt.role === 'host');
	const onCustomServer = $derived(!!lt.serverUrl);
	// What a guest has to be sent. On the default server that is the bare room code and nothing
	// else; a self-hosted room has to carry its address or the code means nothing.
	const invite = $derived(makeInvite(lt.serverUrl, lt.roomCode ?? ''));
	// Sitting between "asked to join" and "in the room" — show a waiting state, block re-sends.
	const waiting = $derived(lt.requesting && lt.role === 'none');
	const statusLabel = $derived(
		lt.status === 'connecting'
			? t('dialogs.listen_together.status_connecting')
			: lt.status === 'connected'
				? t('dialogs.listen_together.status_connected')
				: t('dialogs.listen_together.status_disconnected')
	);
	const others = $derived(lt.users.filter((u) => u.user_id !== lt.myId).length);

	function rememberName() {
		localStorage.setItem('lt_name', name.trim());
	}

	/** First letter of a display name, for the participant circles. */
	function initial(username: string): string {
		return (username.trim()[0] ?? '?').toUpperCase();
	}

	// An invite bundles the server + code so a guest only pastes one thing: `<code>@<host>`.
	// `wss://` and the `/ws` path are implied, so the usual invite stays short and typable.
	// An empty server is the built-in default, which the backend fills in, so that invite is the
	// bare code: a host on the default never shares a hostname, and a guest never types one.
	function makeInvite(server: string, code: string): string {
		if (!server) return code;
		return code + '@' + server.replace(/^wss:\/\//, '').replace(/\/ws$/, '');
	}
	function parseInvite(raw: string): { server: string; code: string } | null {
		const s = raw.trim();
		// Invites minted before 0.6.5: `LMSC~<base64(server|code)>`.
		if (s.startsWith('LMSC~')) {
			try {
				const [server, code] = atob(s.slice(5)).split('|');
				return { server: server ?? '', code: (code ?? '').toUpperCase() };
			} catch {
				return null;
			}
		}
		const at = s.lastIndexOf('@');
		// A bare room code: reuse whatever server we last connected to.
		if (at < 0) return { server: '', code: s.toUpperCase() };
		let server = s.slice(at + 1);
		if (!/^wss?:\/\//.test(server)) server = 'wss://' + server;
		if (!server.replace(/^wss?:\/\//, '').includes('/')) server += '/ws';
		return { server, code: s.slice(0, at).toUpperCase() };
	}

	/** The default is stored as empty, so typing it in by hand still mints bare invites. */
	function normalize(url: string): string {
		const u = url.trim();
		return u === lt.defaultServerUrl ? '' : u;
	}

	function toggleServerPanel() {
		showServer = !showServer;
		// Opening the panel is the one moment the address is worth showing: it is what the field is
		// about to replace.
		if (showServer && !serverUrl) serverUrl = lt.defaultServerUrl;
	}

	async function host(e?: Event) {
		e?.preventDefault();
		if (!name.trim()) return toast.error(t('dialogs.listen_together.err_enter_name'));
		const u = normalize(serverUrl);
		busy = true;
		try {
			if (u !== lt.serverUrl) await api.ltSetServerUrl(u);
			rememberName();
			await api.ltCreateRoom(name.trim());
		} finally {
			busy = false;
		}
	}

	async function join(e?: Event) {
		e?.preventDefault();
		if (!name.trim()) return toast.error(t('dialogs.listen_together.err_enter_name'));
		const parsed = parseInvite(inviteInput);
		if (!parsed || !parsed.code) return toast.error(t('dialogs.listen_together.err_paste_code'));
		// A bare code joins whatever server we are already set to, which is the default until the
		// user picks their own.
		const server = parsed.server ? normalize(parsed.server) : lt.serverUrl;
		busy = true;
		try {
			if (server !== lt.serverUrl) await api.ltSetServerUrl(server);
			rememberName();
			await api.ltJoinRoom(parsed.code, name.trim());
		} finally {
			busy = false;
		}
	}

	async function leave() {
		await api.ltLeave();
	}

	function copyInvite() {
		copyText(invite).then(
			() => {
				copied = true;
				setTimeout(() => (copied = false), 1600);
			},
			() => toast.error(t('dialogs.listen_together.invite_copy_failed'))
		);
	}
</script>

<Dialog.Root bind:open={ui.ltOpen}>
	<Dialog.Content class="gap-0 overflow-hidden sm:max-w-md">
		<Dialog.Header class="gap-1">
			<Dialog.Title class="flex items-center gap-2">
				<HugeiconsIcon icon={UserGroupIcon} class="h-5 w-5 text-primary" />
				{t('dialogs.listen_together.title')}
			</Dialog.Title>
			<Dialog.Description class={inRoom || waiting ? 'sr-only' : 'text-xs'}>
				{inRoom || waiting
					? t('dialogs.listen_together.desc')
					: t('dialogs.listen_together.subtitle')}
			</Dialog.Description>
		</Dialog.Header>

		{#if waiting}
			<!-- Asked to join / creating — waiting on the room. -->
			<div class="flex min-w-0 flex-col items-center gap-4 py-12">
				<div class="h-9 w-9 animate-spin rounded-full border-2 border-muted border-t-primary"></div>
				<p class="text-center text-sm text-muted-foreground">
					{lt.status === 'connecting'
						? t('dialogs.listen_together.connecting')
						: t('dialogs.listen_together.waiting_for_host')}
				</p>
				<Button variant="ghost" size="sm" onclick={leave}>{t('common.cancel')}</Button>
			</div>
		{:else if !inRoom}
			<!-- Setup: join with a code, or open a room of your own. -->
			<div class="flex min-w-0 flex-col gap-5 pt-5">
				<div class="flex rounded-full bg-muted p-1 text-sm">
					<button
						class="flex-1 cursor-pointer rounded-full py-1.5 font-medium transition-colors {mode ===
						'join'
							? 'bg-background text-foreground shadow-sm'
							: 'text-muted-foreground hover:text-foreground'}"
						onclick={() => (mode = 'join')}>{t('dialogs.listen_together.join_tab')}</button
					>
					<button
						class="flex-1 cursor-pointer rounded-full py-1.5 font-medium transition-colors {mode ===
						'host'
							? 'bg-background text-foreground shadow-sm'
							: 'text-muted-foreground hover:text-foreground'}"
						onclick={() => (mode = 'host')}>{t('dialogs.listen_together.host_tab')}</button
					>
				</div>

				<!-- Both paths need a name, so it is asked once, above the split. -->
				<div class="flex flex-col gap-1.5">
					<label class="text-xs font-medium text-muted-foreground" for="lt-name">
						{t('dialogs.listen_together.your_name')}
					</label>
					<Input
						id="lt-name"
						bind:value={name}
						maxlength={24}
						placeholder={t('dialogs.listen_together.your_name_placeholder')}
					/>
				</div>

				{#if mode === 'join'}
					<form class="flex flex-col gap-5" onsubmit={join}>
						<div class="flex flex-col gap-1.5">
							<label class="text-xs font-medium text-muted-foreground" for="lt-invite">
								{t('dialogs.listen_together.invite_code')}
							</label>
							<Input
								id="lt-invite"
								bind:value={inviteInput}
								autocapitalize="characters"
								spellcheck={false}
								class="font-mono uppercase tracking-widest placeholder:normal-case placeholder:tracking-normal"
								placeholder={t('dialogs.listen_together.invite_placeholder')}
							/>
							<p class="text-xs text-muted-foreground">
								{t('dialogs.listen_together.invite_hint')}
							</p>
						</div>
						<Button type="submit" class="w-full" disabled={busy}>
							{t('dialogs.listen_together.join_button')}
						</Button>
					</form>
				{:else}
					<form class="flex flex-col gap-5" onsubmit={host}>
						<div class="flex gap-3 rounded-2xl border bg-muted/40 p-3.5">
							<HugeiconsIcon icon={PlusSignIcon} class="mt-0.5 h-4 w-4 shrink-0 text-primary" />
							<p class="text-xs leading-relaxed text-muted-foreground">
								{t('dialogs.listen_together.host_blurb')}
							</p>
						</div>
						<Button type="submit" class="w-full" disabled={busy}>
							{t('dialogs.listen_together.start_button')}
						</Button>

						<!-- The server address lives behind this, and nowhere else. Almost nobody runs
						     one, so the default path never shows a URL at all. -->
						<div class="border-t pt-3">
							<button
								type="button"
								class="flex w-full cursor-pointer items-center gap-2 text-xs text-muted-foreground transition-colors hover:text-foreground"
								onclick={toggleServerPanel}
								aria-expanded={showServer}
							>
								<HugeiconsIcon icon={ServerStack01Icon} class="h-3.5 w-3.5" />
								{t('dialogs.listen_together.change_server')}
								{#if onCustomServer}
									<span class="rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">
										{t('dialogs.listen_together.custom_server')}
									</span>
								{/if}
							</button>
							{#if showServer}
								<div class="mt-3 flex flex-col gap-1.5">
									<Input
										bind:value={serverUrl}
										spellcheck={false}
										class="font-mono text-xs"
										placeholder={t('dialogs.listen_together.sync_server_placeholder')}
									/>
									<div class="flex items-start justify-between gap-3">
										<p class="text-xs text-muted-foreground">
											{t('dialogs.listen_together.sync_server_hint')}
										</p>
										{#if normalize(serverUrl) !== ''}
											<button
												type="button"
												class="shrink-0 cursor-pointer text-xs text-primary hover:underline"
												onclick={() => (serverUrl = lt.defaultServerUrl)}
											>
												{t('dialogs.listen_together.use_default')}
											</button>
										{/if}
									</div>
								</div>
							{/if}
						</div>
					</form>
				{/if}
			</div>
		{:else}
			<!-- In a room. -->
			<div class="flex min-w-0 flex-col gap-4 pt-5">
				<!-- The invite, which is the only thing a host has to act on. -->
				<div class="rounded-2xl border bg-muted/40 p-4 text-center">
					<div
						class="flex items-center justify-center gap-1.5 text-xs font-medium text-muted-foreground"
					>
						<span
							class="h-1.5 w-1.5 rounded-full {lt.status === 'connected'
								? 'bg-green-500'
								: lt.status === 'connecting'
									? 'animate-pulse bg-amber-500'
									: 'bg-muted-foreground/40'}"
						></span>
						{statusLabel} · {isHost
							? t('dialogs.listen_together.hosting')
							: t('dialogs.listen_together.listening')}
					</div>

					<!-- Spaced out because this gets read aloud and typed by hand. The left padding
					     puts the trailing letter-space back on the right so it stays centred. -->
					<div class="mt-3 select-all pl-[0.3em] font-mono text-3xl font-semibold tracking-[0.3em]">
						{lt.roomCode ?? '······'}
					</div>
					{#if onCustomServer}
						<div class="mt-1.5 select-all truncate font-mono text-[11px] text-muted-foreground">
							{invite}
						</div>
					{/if}

					<Button variant="outline" size="sm" class="mt-4 w-full" onclick={copyInvite}>
						<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
						<HugeiconsIcon icon={Copy01Icon} altIcon={Tick02Icon} showAlt={copied} class="h-4 w-4" />
						{copied
							? t('dialogs.listen_together.copied')
							: onCustomServer
								? t('dialogs.listen_together.copy_invite')
								: t('dialogs.listen_together.copy_code')}
					</Button>
					<p class="mt-2 text-xs text-muted-foreground">
						{t('dialogs.listen_together.share_hint')}
					</p>
				</div>

				<!-- Now playing -->
				{#if lt.currentTrack}
					<div class="flex min-w-0 items-center gap-3 px-1">
						{#if lt.currentTrack.thumbnail}
							<img
								src={lt.currentTrack.thumbnail}
								alt=""
								class="h-10 w-10 shrink-0 rounded-lg object-cover"
							/>
						{/if}
						<div class="min-w-0 flex-1">
							<div class="truncate text-sm font-medium">{lt.currentTrack.title}</div>
							<div class="truncate text-xs text-muted-foreground">{lt.currentTrack.artist}</div>
						</div>
					</div>
				{/if}

				<!-- Everything that grows with the room scrolls; the invite and the footer do not. -->
				<div class="-mx-1 flex min-w-0 max-h-[38vh] flex-col gap-5 overflow-y-auto px-1">
					<!-- Host: pending join requests -->
					{#if isHost && lt.pendingJoins.length}
						<div>
							<div class="mb-2 text-xs font-medium text-muted-foreground">
								{t('dialogs.listen_together.join_requests')}
							</div>
							<div class="flex flex-col gap-2">
								{#each lt.pendingJoins as p (p.userId)}
									<div class="flex min-w-0 items-center gap-2.5 rounded-xl border bg-muted/40 p-2">
										<span
											class="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-semibold text-primary"
										>
											{initial(p.username)}
										</span>
										<span class="min-w-0 flex-1 truncate text-sm">{p.username}</span>
										<Button size="sm" onclick={() => api.ltApproveJoin(p.userId)}>
											{t('dialogs.listen_together.approve')}
										</Button>
										<Button size="sm" variant="ghost" onclick={() => api.ltRejectJoin(p.userId)}>
											{t('dialogs.listen_together.decline')}
										</Button>
									</div>
								{/each}
							</div>
						</div>
					{/if}

					<!-- Participants -->
					<div>
						<div class="mb-2 text-xs font-medium text-muted-foreground">
							{t('dialogs.listen_together.in_room', { count: lt.users.length })}
						</div>
						<div class="flex flex-col gap-0.5">
							{#each lt.users as u (u.user_id)}
								<div class="group flex min-w-0 items-center gap-2.5 rounded-lg py-1">
									<span class="relative shrink-0">
										<span
											class="flex h-7 w-7 items-center justify-center rounded-full text-xs font-semibold {u.is_host
												? 'bg-primary/10 text-primary'
												: 'bg-muted text-muted-foreground'} {u.is_connected ? '' : 'opacity-40'}"
										>
											{initial(u.username)}
										</span>
										<span
											class="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-popover {u.is_connected
												? 'bg-green-500'
												: 'bg-muted-foreground/40'}"
											title={u.is_connected
												? t('dialogs.listen_together.connected')
												: t('dialogs.listen_together.disconnected')}
										></span>
									</span>
									<span class="min-w-0 flex-1 truncate text-sm {u.is_connected ? '' : 'opacity-50'}">
										{u.username}{u.user_id === lt.myId
											? ` ${t('dialogs.listen_together.you')}`
											: ''}
									</span>
									{#if u.is_host}
										<HugeiconsIcon icon={CrownIcon} class="h-3.5 w-3.5 shrink-0 text-amber-500" />
									{/if}
									{#if isHost && u.user_id !== lt.myId}
										<!-- Revealed on hover, but always in the tab order and always named. -->
										<button
											class="shrink-0 cursor-pointer rounded-md p-1 text-muted-foreground opacity-0 transition hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
											onclick={() => api.ltTransferHost(u.user_id)}
										>
											<HugeiconsIcon icon={Exchange01Icon} class="h-4 w-4" />
											<span class="sr-only">{t('dialogs.listen_together.make_host')}</span>
										</button>
										<button
											class="shrink-0 cursor-pointer rounded-md p-1 text-muted-foreground opacity-0 transition hover:bg-muted hover:text-destructive focus-visible:opacity-100 group-hover:opacity-100"
											onclick={() => api.ltKick(u.user_id)}
										>
											<HugeiconsIcon icon={UserRemove01Icon} class="h-4 w-4" />
											<span class="sr-only">{t('dialogs.listen_together.remove')}</span>
										</button>
									{/if}
								</div>
							{/each}
						</div>
						{#if isHost && others === 0 && !lt.pendingJoins.length}
							<p class="mt-2 text-xs text-muted-foreground">
								{t('dialogs.listen_together.alone_hint')}
							</p>
						{/if}
					</div>

					<!-- Host: suggestions from guests -->
					{#if isHost && lt.suggestions.length}
						<div>
							<div class="mb-2 text-xs font-medium text-muted-foreground">
								{t('dialogs.listen_together.suggestions')}
							</div>
							<div class="flex flex-col gap-2">
								{#each lt.suggestions as s (s.id)}
									<div class="flex min-w-0 items-center gap-2.5 rounded-xl border bg-muted/40 p-2">
										<div class="min-w-0 flex-1">
											<div class="truncate text-sm">{s.track.title}</div>
											<div class="truncate text-xs text-muted-foreground">
												{s.track.artist} · {t('dialogs.listen_together.from_user', {
													user: s.from_username
												})}
											</div>
										</div>
										<Button size="sm" onclick={() => api.ltApproveSuggestion(s.id)}>
											{t('dialogs.listen_together.queue_it')}
										</Button>
										<Button size="sm" variant="ghost" onclick={() => api.ltRejectSuggestion(s.id)}>
											<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
											<span class="sr-only">{t('dialogs.listen_together.dismiss')}</span>
										</Button>
									</div>
								{/each}
							</div>
						</div>
					{/if}
				</div>

				<!-- Footer actions -->
				<div class="flex items-center gap-2 border-t pt-3">
					{#if !isHost}
						<Button variant="ghost" size="sm" onclick={() => api.ltRequestSync()}>
							<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
							{t('dialogs.listen_together.resync')}
						</Button>
					{/if}
					<div class="flex-1"></div>
					<Button variant="destructive" size="sm" onclick={leave}>
						<HugeiconsIcon icon={Logout01Icon} class="h-4 w-4" />
						{t('dialogs.listen_together.leave')}
					</Button>
				</div>
			</div>
		{/if}
	</Dialog.Content>
</Dialog.Root>
