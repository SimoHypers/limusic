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
		RefreshIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { copyText } from '$lib/clipboard';
	import { ui, toast } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';
	import { t } from '$lib/i18n.svelte';

	let mode = $state<'join' | 'host'>('join');
	let name = $state('');
	let serverUrl = $state('');
	let inviteInput = $state('');
	let busy = $state(false);

	// Seed inputs when the modal opens: remembered name + the persisted server URL (host mode).
	$effect(() => {
		if (ui.ltOpen) {
			name = localStorage.getItem('lt_name') ?? '';
			serverUrl = lt.serverUrl;
		}
	});

	const inRoom = $derived(lt.role !== 'none');
	const isHost = $derived(lt.role === 'host');
	// The only thing worth showing or sending: the bare room code is useless to a guest who doesn't
	// already know the server URL, and every self-hosted server has a different one.
	const invite = $derived(makeInvite(lt.serverUrl, lt.roomCode ?? ''));
	// Sitting between "asked to join" and "in the room" — show a waiting state, block re-sends.
	const waiting = $derived(lt.requesting && lt.role === 'none');

	function rememberName() {
		localStorage.setItem('lt_name', name.trim());
	}

	// An invite bundles the server + code so a guest only pastes one thing. `LMSC~<base64(server|code)>`.
	function makeInvite(server: string, code: string): string {
		return 'LMSC~' + btoa(`${server}|${code}`);
	}
	function parseInvite(raw: string): { server: string; code: string } | null {
		const s = raw.trim();
		if (s.startsWith('LMSC~')) {
			try {
				const [server, code] = atob(s.slice(5)).split('|');
				return { server: server ?? '', code: (code ?? '').toUpperCase() };
			} catch {
				return null;
			}
		}
		// A bare room code — reuse whatever server we last connected to.
		return { server: '', code: s.toUpperCase() };
	}

	async function host() {
		if (!name.trim()) return toast.error(t('dialogs.listen_together.err_enter_name'));
		const u = serverUrl.trim();
		if (!u) return toast.error(t('dialogs.listen_together.err_enter_server'));
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
		const server = parsed.server || lt.serverUrl;
		if (!server) return toast.error(t('dialogs.listen_together.err_paste_full_invite'));
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
			() => toast.success(t('dialogs.listen_together.invite_copied')),
			() => toast.error(t('dialogs.listen_together.invite_copy_failed'))
		);
	}
</script>

<Dialog.Root bind:open={ui.ltOpen}>
	<Dialog.Content class="overflow-hidden sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.listen_together.title')}</Dialog.Title>
			<Dialog.Description class="sr-only">{t('dialogs.listen_together.desc')}</Dialog.Description>
		</Dialog.Header>

		{#if waiting}
			<!-- Asked to join / creating — waiting on the room. -->
			<div class="flex flex-col items-center gap-4 py-10">
				<div
					class="h-8 w-8 animate-spin rounded-full border-2 border-muted border-t-primary"
				></div>
				<p class="text-sm text-muted-foreground">
					{lt.status === 'connecting'
						? t('dialogs.listen_together.connecting')
						: t('dialogs.listen_together.waiting_for_host')}
				</p>
				<Button variant="outline" size="sm" onclick={leave}>{t('common.cancel')}</Button>
			</div>
		{:else if !inRoom}
			<!-- Setup: join a friend (just a name + invite) or host your own. -->
			<div class="flex flex-col gap-4 pt-1">
				<div class="flex rounded-lg bg-muted p-1 text-sm">
					<button
						class="flex-1 rounded-md py-1.5 font-medium transition-colors {mode === 'join'
							? 'bg-background shadow-sm'
							: 'text-muted-foreground'}"
						onclick={() => (mode = 'join')}>{t('dialogs.listen_together.join_tab')}</button
					>
					<button
						class="flex-1 rounded-md py-1.5 font-medium transition-colors {mode === 'host'
							? 'bg-background shadow-sm'
							: 'text-muted-foreground'}"
						onclick={() => (mode = 'host')}>{t('dialogs.listen_together.host_tab')}</button
					>
				</div>

				{#if mode === 'join'}
					<form class="flex flex-col gap-4" onsubmit={join}>
						<div>
							<div class="mb-1 text-sm font-medium">{t('dialogs.listen_together.invite_code')}</div>
							<Input bind:value={inviteInput} placeholder={t('dialogs.listen_together.invite_placeholder')} />
							<p class="mt-1 text-xs text-muted-foreground">
								{t('dialogs.listen_together.invite_hint')}
							</p>
						</div>
						<div>
							<div class="mb-1 text-sm font-medium">{t('dialogs.listen_together.your_name')}</div>
							<Input bind:value={name} placeholder={t('dialogs.listen_together.your_name_placeholder')} />
						</div>
						<Button type="submit" disabled={busy}>{t('dialogs.listen_together.join_button')}</Button>
					</form>
				{:else}
					<div class="flex flex-col gap-4">
						<div>
							<div class="mb-1 text-sm font-medium">{t('dialogs.listen_together.sync_server')}</div>
							<Input bind:value={serverUrl} placeholder={t('dialogs.listen_together.sync_server_placeholder')} />
							<p class="mt-1 text-xs text-muted-foreground">
								{t('dialogs.listen_together.sync_server_hint')}
							</p>
						</div>
						<div>
							<div class="mb-1 text-sm font-medium">{t('dialogs.listen_together.your_name')}</div>
							<Input bind:value={name} placeholder={t('dialogs.listen_together.your_name_placeholder')} />
						</div>
						<Button onclick={host} disabled={busy}>{t('dialogs.listen_together.start_button')}</Button>
					</div>
				{/if}
			</div>
		{:else}
			<!-- In a room. -->
			<div class="flex flex-col gap-4 pt-1">
				<!-- Role + invite -->
				<div class="rounded-lg border bg-muted/40 p-4 text-center">
					<div class="text-xs font-medium uppercase tracking-wide text-muted-foreground">
						{isHost ? t('dialogs.listen_together.hosting') : t('dialogs.listen_together.listening')} · {lt.status}
					</div>
					<div
						class="mt-2 select-all break-all rounded-md bg-background px-2 py-1.5 text-left font-mono text-[11px] leading-snug"
					>
						{invite}
					</div>
					<Button variant="outline" size="sm" class="mt-3 w-full" onclick={copyInvite}>
						<HugeiconsIcon icon={Copy01Icon} class="h-4 w-4" />
						{t('dialogs.listen_together.copy_invite')}
					</Button>
				</div>

				<!-- Now playing -->
				{#if lt.currentTrack}
					<div class="flex min-w-0 items-center gap-3">
						{#if lt.currentTrack.thumbnail}
							<img
								src={lt.currentTrack.thumbnail}
								alt=""
								class="h-10 w-10 shrink-0 rounded object-cover"
							/>
						{/if}
						<div class="min-w-0 flex-1">
							<div class="truncate text-sm font-medium">{lt.currentTrack.title}</div>
							<div class="truncate text-xs text-muted-foreground">{lt.currentTrack.artist}</div>
						</div>
					</div>
				{/if}

				<!-- Host: pending join requests -->
				{#if isHost && lt.pendingJoins.length}
					<div>
						<div class="mb-2 text-sm font-medium">{t('dialogs.listen_together.join_requests')}</div>
						<div class="flex flex-col gap-2">
							{#each lt.pendingJoins as p (p.userId)}
								<div class="flex min-w-0 items-center gap-2">
									<span class="min-w-0 flex-1 truncate text-sm">{p.username}</span>
									<Button size="sm" onclick={() => api.ltApproveJoin(p.userId)}>
										<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4" />
									</Button>
									<Button size="sm" variant="outline" onclick={() => api.ltRejectJoin(p.userId)}>
										<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
									</Button>
								</div>
							{/each}
						</div>
					</div>
				{/if}

				<!-- Participants -->
				<div>
					<div class="mb-2 text-sm font-medium">{t('dialogs.listen_together.in_room', { count: lt.users.length })}</div>
					<div class="flex flex-col gap-1">
						{#each lt.users as u (u.user_id)}
							<div class="flex min-w-0 items-center gap-2 rounded-md px-1 py-1">
								<span
									class="h-2 w-2 shrink-0 rounded-full {u.is_connected
										? 'bg-green-500'
										: 'bg-muted-foreground/40'}"
									title={u.is_connected ? t('dialogs.listen_together.connected') : t('dialogs.listen_together.disconnected')}
								></span>
								<span class="min-w-0 flex-1 truncate text-sm {u.is_connected ? '' : 'opacity-50'}">
									{u.username}{u.user_id === lt.myId ? ` ${t('dialogs.listen_together.you')}` : ''}
								</span>
								{#if u.is_host}
									<HugeiconsIcon icon={CrownIcon} class="h-4 w-4 shrink-0 text-yellow-500" />
								{/if}
								{#if isHost && u.user_id !== lt.myId}
									<button
										class="shrink-0 text-muted-foreground hover:text-foreground"
										title={t('dialogs.listen_together.make_host')}
										onclick={() => api.ltTransferHost(u.user_id)}
									>
										<HugeiconsIcon icon={Exchange01Icon} class="h-4 w-4" />
									</button>
									<button
										class="shrink-0 text-muted-foreground hover:text-destructive"
										title={t('dialogs.listen_together.remove')}
										onclick={() => api.ltKick(u.user_id)}
									>
										<HugeiconsIcon icon={UserRemove01Icon} class="h-4 w-4" />
									</button>
								{/if}
							</div>
						{/each}
					</div>
				</div>

				<!-- Host: suggestions from guests -->
				{#if isHost && lt.suggestions.length}
					<div>
						<div class="mb-2 text-sm font-medium">{t('dialogs.listen_together.suggestions')}</div>
						<div class="flex flex-col gap-2">
							{#each lt.suggestions as s (s.id)}
								<div class="flex min-w-0 items-center gap-2">
									<div class="min-w-0 flex-1">
										<div class="truncate text-sm">{s.track.title}</div>
										<div class="truncate text-xs text-muted-foreground">
											{s.track.artist} · {t('dialogs.listen_together.from_user', { user: s.from_username })}
										</div>
									</div>
									<Button size="sm" onclick={() => api.ltApproveSuggestion(s.id)}>
										<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4" />
									</Button>
									<Button size="sm" variant="outline" onclick={() => api.ltRejectSuggestion(s.id)}>
										<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
									</Button>
								</div>
							{/each}
						</div>
					</div>
				{/if}

				<!-- Footer actions -->
				<div class="flex items-center gap-2 border-t pt-3">
					{#if !isHost}
						<Button variant="outline" size="sm" onclick={() => api.ltRequestSync()}>
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
