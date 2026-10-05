<script lang="ts">
	// "Open link": paste a YouTube Music URL and land on the item (#63). The way into a playlist
	// that is shared by link only, so it never turns up in search or the library.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { onOpenLink, takeLaunchArgs } from '$lib/api';
	import { hrefFor } from '$lib/browse';
	import { parseYtLink, type LinkTarget } from '$lib/ytlink';
	import { startRadio, toast, ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { isSpotifyLink, openSpotifyLink } from '$lib/import.svelte';
	import { LOCAL_SONG_PREFIX } from '$lib/api';
	import { play } from '$lib/api';

	let url = $state('');

	// A song has no page to open, so it plays, with its radio behind it like every other song
	// you click. The link carries the id and nothing else, so the title comes from YouTube.
	function open(target: LinkTarget) {
		if (target.kind === 'song') startRadio('song', target.id);
		else goto(hrefFor({ ...target, title: '' }));
	}

	function submit(e: Event) {
		e.preventDefault();
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

	// Supported audio extensions from local.rs
	const AUDIO_EXTENSIONS = new Set([
		'mp3', 'flac', 'm4a', 'm4b', 'aac', 'ogg', 'oga', 'opus', 'wav', 'wma', 'aiff', 'aif', 'ape', 'wv', 'mka'
	]);

	function isLocalAudioFile(path: string): boolean {
		try {
			const url = new URL(path);
			// It's a URL, not a file path
			return false;
		} catch {
			// Not a URL, treat as potential file path
			const ext = path.split('.').pop()?.toLowerCase();
			return ext ? AUDIO_EXTENSIONS.has(ext) : false;
		}
	}

	async function playLocalFile(filePath: string) {
		const songItem = {
			video_id: `${LOCAL_SONG_PREFIX}${filePath}`,
			title: filePath.split(/[\\/]/).pop() || 'Local file',
			artists: 'Unknown artist',
			album: undefined,
			album_id: undefined,
			duration: undefined,
			thumbnail: undefined,
			play_count: undefined,
			artist_runs: undefined,
			set_video_id: undefined,
			added_by: undefined,
			added_by_avatar: undefined,
			rating: undefined,
			library: undefined,
			queued_by: undefined,
			queued: false,
			queued_end: false,
			queued_from: undefined,
			autoplay: false,
			explicit: false,
			is_video: false,
			is_upload: false,
			artist_id: undefined
		};
		await play(songItem);
	}

	// The same from outside the app (#348): `limusic-app <link>`, from Rust at launch or from a
	// second launch while this one runs. Flags ride along in argv (`--autostart`, macOS's `-psn_`),
	// and a leading word like `open` is skipped too, since the first argument that parses wins.
	onMount(() => {
		const fromArgs = async (args: string[]) => {
			const given = args.filter((a) => !a.startsWith('-'));
			if (!given.length) return;
			
			// Check each arg: Spotify link, YouTube link, or local audio file
			for (const arg of given) {
				if (isSpotifyLink(arg)) {
					await openSpotifyLink(arg);
					return;
				}
				const target = parseYtLink(arg);
				if (target) {
					open(target);
					return;
				}
				if (isLocalAudioFile(arg)) {
					await playLocalFile(arg);
					return;
				}
			}
			toast.error(t('dialogs.link.invalid_link'));
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
