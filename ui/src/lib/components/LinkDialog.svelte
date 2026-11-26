<script lang="ts">
	// "Open link": paste a YouTube Music URL and land on the item (#63). The way into a playlist
	// that is shared by link only, so it never turns up in search or the library.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { ltGetState, ltJoinRoom, ltSetServerUrl, onOpenLink, takeLaunchArgs } from '$lib/api';
	import { hrefFor } from '$lib/browse';
	import { makeInvite, parseInviteLink } from '$lib/invite';
	import { applyLtState, lt } from '$lib/lt.svelte';
	import { parseYtLink, type LinkTarget } from '$lib/ytlink';
	import { startRadio, toast, ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { isSpotifyLink, openSpotifyLink } from '$lib/import.svelte';

	let url = $state('');

	// A song has no page to open, so it plays, with its radio behind it like every other song
	// you click. The link carries the id and nothing else, so the title comes from YouTube.
	function open(target: LinkTarget) {
		if (target.kind === 'song') startRadio('song', target.id, undefined, target.start);
		else goto(hrefFor({ ...target, title: '' }));
	}

	async function openListenInvite(invite: { server: string; code: string }) {
		if (!lt.defaultServerUrl) {
			await ltGetState().then(applyLtState).catch(() => {});
		}
		if (lt.role !== 'none' || lt.requesting) {
			ui.ltOpen = true;
			if (lt.roomCode?.toUpperCase() !== invite.code) {
				toast.error(t('dialogs.listen_together.err_already_in_room'));
			}
			return;
		}
		const server = invite.server && invite.server !== lt.defaultServerUrl ? invite.server : '';
		ui.ltPendingInvite = makeInvite(server, invite.code);
		ui.ltOpen = true;
		const savedName = (localStorage.getItem('lt_name') ?? '').trim();
		if (!savedName) return;
		try {
			if (server !== lt.serverUrl) await ltSetServerUrl(server);
			await ltJoinRoom(invite.code, savedName);
		} catch (e) {
			toast.error(String(e));
		}
	}

	function submit(e: Event) {
		e.preventDefault();
		const invite = parseInviteLink(url);
		if (invite) {
			ui.linkOpen = false;
			url = '';
			void openListenInvite(invite);
			return;
		}
		// A Spotify link opens its YouTube Music counterpart, or the import for a playlist (#375).
		if (isSpotifyLink(url)) {
			ui.linkOpen = false;
			openSpotifyLink(url.trim());
			url = '';
			return;
		}
		const target = parseYtLink(url);
		if (!target) {
			toast.error(t('dialogs.link.invalid_link'));
			return;
		}
		ui.linkOpen = false;
		url = '';
		open(target);
	}

	// The same from outside the app (#348): `limusic-app <link>`, from Rust at launch or from a
	// second launch while this one runs. Flags ride along in argv (`--autostart`, macOS's `-psn_`),
	// and a leading word like `open` is skipped too, since the first argument that parses wins.
	onMount(() => {
		const fromArgs = (args: string[]) => {
			const given = args.filter((a) => !a.startsWith('-'));
			if (!given.length) return;
			const invite = given.map(parseInviteLink).find((x) => x);
			if (invite) return void openListenInvite(invite);
			const spotify = given.find(isSpotifyLink);
			if (spotify) return openSpotifyLink(spotify);
			const target = given.map(parseYtLink).find((x) => x);
			if (target) open(target);
			else toast.error(t('dialogs.link.invalid_link'));
		};
		const un = onOpenLink(fromArgs);
		takeLaunchArgs()
			.then(fromArgs)
			.catch(() => {});
		return () => un.then((u) => u());
	});
</script>

<Dialog.Root bind:open={ui.linkOpen}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.link.title')}</Dialog.Title>
			<Dialog.Description>
				{t('dialogs.link.desc')}
			</Dialog.Description>
		</Dialog.Header>
		<form class="flex gap-2" onsubmit={submit}>
			<Input bind:value={url} placeholder="https://music.youtube.com/playlist?list=..." />
			<Button type="submit" disabled={!url.trim()}>{t('dialogs.link.open')}</Button>
		</form>
	</Dialog.Content>
</Dialog.Root>
