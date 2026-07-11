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
	import { ui, toast } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';

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
		if (!name.trim()) return toast('Enter a name first');
		const u = serverUrl.trim();
		if (!u) return toast('Enter your sync server URL');
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
		if (!name.trim()) return toast('Enter a name first');
		const parsed = parseInvite(inviteInput);
		if (!parsed || !parsed.code) return toast('Paste the invite code your friend sent');
		const server = parsed.server || lt.serverUrl;
		if (!server) return toast('Paste the full invite from the host — it carries the server address');
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
		navigator.clipboard
			.writeText(makeInvite(lt.serverUrl, lt.roomCode ?? ''))
			.then(() => toast('Invite copied — send it to a friend'));
	}
</script>

<Dialog.Root bind:open={ui.ltOpen}>
	<Dialog.Content class="overflow-hidden sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Listen Together</Dialog.Title>
			<Dialog.Description class="sr-only">Synced listening session</Dialog.Description>
		</Dialog.Header>

		{#if waiting}
			<!-- Asked to join / creating — waiting on the room. -->
			<div class="flex flex-col items-center gap-4 py-10">
				<div
					class="h-8 w-8 animate-spin rounded-full border-2 border-muted border-t-primary"
				></div>
				<p class="text-sm text-muted-foreground">
					{lt.status === 'connecting'
						? 'Connecting…'
						: 'Waiting for the host to let you in…'}
				</p>
				<Button variant="outline" size="sm" onclick={leave}>Cancel</Button>
			</div>
		{:else if !inRoom}
			<!-- Setup: join a friend (just a name + invite) or host your own. -->
			<div class="flex flex-col gap-4 pt-1">
				<div class="flex rounded-lg bg-muted p-1 text-sm">
					<button
						class="flex-1 rounded-md py-1.5 font-medium transition-colors {mode === 'join'
							? 'bg-background shadow-sm'
							: 'text-muted-foreground'}"
						onclick={() => (mode = 'join')}>Join</button
					>
					<button
						class="flex-1 rounded-md py-1.5 font-medium transition-colors {mode === 'host'
							? 'bg-background shadow-sm'
							: 'text-muted-foreground'}"
						onclick={() => (mode = 'host')}>Host</button
					>
				</div>

				{#if mode === 'join'}
					<form class="flex flex-col gap-4" onsubmit={join}>
						<div>
							<div class="mb-1 text-sm font-medium">Invite code</div>
							<Input bind:value={inviteInput} placeholder="Paste the invite your friend sent" />
							<p class="mt-1 text-xs text-muted-foreground">
								The invite carries the server address — nothing else to set up.
							</p>
						</div>
						<div>
							<div class="mb-1 text-sm font-medium">Your name</div>
							<Input bind:value={name} placeholder="Your name" />
						</div>
						<Button type="submit" disabled={busy}>Join session</Button>
					</form>
				{:else}
					<div class="flex flex-col gap-4">
						<div>
							<div class="mb-1 text-sm font-medium">Sync server</div>
							<Input bind:value={serverUrl} placeholder="wss://your-machine.ts.net/ws" />
							<p class="mt-1 text-xs text-muted-foreground">
								Your self-hosted server (e.g. the Tailscale Funnel URL). Saved for next time.
							</p>
						</div>
						<div>
							<div class="mb-1 text-sm font-medium">Your name</div>
							<Input bind:value={name} placeholder="Your name" />
						</div>
						<Button onclick={host} disabled={busy}>Start a session</Button>
					</div>
				{/if}
			</div>
		{:else}
			<!-- In a room. -->
			<div class="flex flex-col gap-4 pt-1">
				<!-- Role + room code -->
				<div class="rounded-lg border bg-muted/40 p-4 text-center">
					<div class="text-xs font-medium uppercase tracking-wide text-muted-foreground">
						{isHost ? 'Hosting' : 'Listening'} · {lt.status}
					</div>
					<div class="mt-1 font-mono text-3xl font-bold tracking-[0.2em]">{lt.roomCode}</div>
					<Button variant="outline" size="sm" class="mt-3" onclick={copyInvite}>
						<HugeiconsIcon icon={Copy01Icon} class="h-4 w-4" />
						Copy invite
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
						<div class="mb-2 text-sm font-medium">Join requests</div>
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
					<div class="mb-2 text-sm font-medium">In the room ({lt.users.length})</div>
					<div class="flex flex-col gap-1">
						{#each lt.users as u (u.user_id)}
							<div class="flex min-w-0 items-center gap-2 rounded-md px-1 py-1">
								<span
									class="h-2 w-2 shrink-0 rounded-full {u.is_connected
										? 'bg-green-500'
										: 'bg-muted-foreground/40'}"
									title={u.is_connected ? 'Connected' : 'Disconnected'}
								></span>
								<span class="min-w-0 flex-1 truncate text-sm {u.is_connected ? '' : 'opacity-50'}">
									{u.username}{u.user_id === lt.myId ? ' (you)' : ''}
								</span>
								{#if u.is_host}
									<HugeiconsIcon icon={CrownIcon} class="h-4 w-4 shrink-0 text-yellow-500" />
								{/if}
								{#if isHost && u.user_id !== lt.myId}
									<button
										class="shrink-0 text-muted-foreground hover:text-foreground"
										title="Make host"
										onclick={() => api.ltTransferHost(u.user_id)}
									>
										<HugeiconsIcon icon={Exchange01Icon} class="h-4 w-4" />
									</button>
									<button
										class="shrink-0 text-muted-foreground hover:text-destructive"
										title="Remove"
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
						<div class="mb-2 text-sm font-medium">Suggestions</div>
						<div class="flex flex-col gap-2">
							{#each lt.suggestions as s (s.id)}
								<div class="flex min-w-0 items-center gap-2">
									<div class="min-w-0 flex-1">
										<div class="truncate text-sm">{s.track.title}</div>
										<div class="truncate text-xs text-muted-foreground">
											{s.track.artist} · from {s.from_username}
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
							Re-sync
						</Button>
					{/if}
					<div class="flex-1"></div>
					<Button variant="destructive" size="sm" onclick={leave}>
						<HugeiconsIcon icon={Logout01Icon} class="h-4 w-4" />
						Leave
					</Button>
				</div>
			</div>
		{/if}
	</Dialog.Content>
</Dialog.Root>
