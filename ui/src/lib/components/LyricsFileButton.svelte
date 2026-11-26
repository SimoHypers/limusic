<script lang="ts">
	import { untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Download04Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import { limusic } from '$lib/limusic.svelte';
	import { toast } from '$lib/player.svelte';

	/**
	 * "Save as LRC file", in the lyrics footer (#772): the lyric on screen, word by word, as a file
	 * for other players and editors. Only on the user's own Limusic lyric: the app doesn't hand out
	 * a provider's lyrics, or another contributor's, as a file. Styled like the footer's other icons.
	 */
	let { lyrics, track }: { lyrics: api.Lyrics | null; track: api.LyricsTrack } = $props();

	/** Could be this account's, so worth asking the service: a community lyric (one Limusic
	 *  mirrored from a provider has `via` and no author), and for a linked account one that credits
	 *  its handle. The service credits no anonymous account, so for one of those every community
	 *  lyric is asked about. Everyone else asks nothing. */
	const maybe = $derived.by(() => {
		if (!lyrics || lyrics.provider !== 'limusic' || lyrics.revision == null || lyrics.via) return false;
		if (!limusic.signedIn) return false;
		if (limusic.anonymous) return true;
		const handle = limusic.handle?.toLowerCase();
		return !!handle && !!lyrics.authors?.some((a) => a.toLowerCase() === handle);
	});

	let own = $state(false);
	/** The song, revision and account `own` was asked for. A late answer for another is dropped. */
	let asked = '';
	$effect(() => {
		const key = maybe ? `${track.videoId} ${lyrics?.revision} ${limusic.handle ?? ''}` : '';
		const videoId = track.videoId;
		const revision = lyrics?.revision;
		untrack(() => ask(key, videoId, revision));
	});

	async function ask(key: string, videoId: string, revision: number | undefined) {
		if (key === asked) return;
		asked = key;
		own = false;
		if (!key) return;
		const s = await api.limusicLyricState(videoId).catch(() => null);
		// Only the live revision is known to be theirs: an older copy on screen is not asked about.
		if (asked === key) own = !!s?.own && s.revision === revision;
	}

	let saving = $state(false);
	async function save() {
		if (!lyrics || saving) return;
		saving = true;
		try {
			if (await api.saveSyncedLyrics(track.title, track.artists, lyrics.lines)) toast.success(t('lyrics.save_lrc_done'));
		} catch {
			toast.error(t('lyrics.save_lrc_failed'));
		} finally {
			saving = false;
		}
	}
</script>

{#if own}
	<button
		class="flex shrink-0 cursor-pointer items-center rounded-full px-2 py-0.5 transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
		disabled={saving}
		aria-label={t('lyrics.save_lrc')}
		title={t('lyrics.save_lrc_hint')}
		onclick={save}
	>
		<HugeiconsIcon icon={Download04Icon} class="h-3.5 w-3.5" />
	</button>
{/if}
