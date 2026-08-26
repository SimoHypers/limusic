<script lang="ts">
	// "Open link": paste a YouTube Music URL and land on the item (#63). The way into a playlist
	// that is shared by link only, so it never turns up in search or the library.
	import { goto } from '$app/navigation';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { hrefFor } from '$lib/browse';
	import { parseYtLink } from '$lib/ytlink';
	import { startRadio, toast, ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let url = $state('');

	function submit(e: Event) {
		e.preventDefault();
		const target = parseYtLink(url);
		if (!target) {
			toast.error(t('dialogs.link.invalid_link'));
			return;
		}
		ui.linkOpen = false;
		url = '';
		// A song has no page to open, so it plays, with its radio behind it like every other song
		// you click. The link carries the id and nothing else, so the title comes from YouTube.
		if (target.kind === 'song') startRadio('song', target.id);
		else goto(hrefFor({ ...target, title: '' }));
	}
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
