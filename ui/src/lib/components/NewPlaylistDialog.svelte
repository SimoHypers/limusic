<script lang="ts">
	// "New playlist", from the sidebar, the Library page and the picker's first row. One dialog for
	// all three, so where a playlist ends up is decided the same way wherever it is made: on the
	// YouTube Music account, or on this machine (#251), which is the only choice signed out.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { CloudIcon, ComputerIcon } from '@hugeicons/core-free-icons';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as api from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import {
		addSongsToPlaylist,
		auth,
		createLibraryPlaylist,
		toast,
		ui
	} from '$lib/player.svelte';

	type Where = 'account' | 'device';
	// The last pick sticks until changed: someone who keeps their playlists on this machine
	// shouldn't have to say so every time. Unset, it is the account, which is what "New playlist"
	// always meant signed in.
	const WHERE_KEY = 'new_playlist_where';
	function storedWhere(): Where {
		try {
			return localStorage.getItem(WHERE_KEY) === 'device' ? 'device' : 'account';
		} catch {
			return 'account';
		}
	}
	let where = $state<Where>(storedWhere());

	let title = $state('');
	let creating = $state(false);

	const signedIn = $derived(!!auth.account?.signedIn);
	const songs = $derived(ui.newPlaylist?.songs ?? []);
	// A file on disk has no YouTube identity, so a batch holding one can only go on this machine.
	const hasLocalFiles = $derived(songs.some((s) => api.isLocalId(s.video_id)));
	const local = $derived(!signedIn || hasLocalFiles || where === 'device');

	// A fresh name each time it opens, so a cancelled one isn't waiting in the box next time.
	$effect(() => {
		if (ui.newPlaylist) title = '';
	});

	const close = () => (ui.newPlaylist = null);

	async function create() {
		const name = title.trim();
		if (!name || creating) return;
		creating = true;
		// Taken now: closing the dialog below clears `ui.newPlaylist`, and the songs with it.
		const batch = songs;
		try {
			const item = await createLibraryPlaylist(name, local);
			close();
			// Adding says what happened on its own ("Added to …"), which covers the creation too.
			if (batch.length) await addSongsToPlaylist(item, batch);
			else toast.success(t('toasts.playlist_created', { title: name }));
		} catch (e) {
			toast.error(String(e));
		} finally {
			creating = false;
		}
	}

	function choose(v: string) {
		where = v === 'device' ? 'device' : 'account';
		try {
			localStorage.setItem(WHERE_KEY, where);
		} catch {
			/* a blocked store just means the choice isn't remembered */
		}
	}

	const options = [
		{ value: 'account', icon: CloudIcon, label: 'account', desc: 'account_desc' },
		{ value: 'device', icon: ComputerIcon, label: 'device', desc: 'device_desc' }
	] as const;
</script>

<Dialog.Root open={!!ui.newPlaylist} onOpenChange={(v) => !v && close()}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.edit_playlist.new_title')}</Dialog.Title>
			<Dialog.Description>{t('dialogs.edit_playlist.new_desc')}</Dialog.Description>
		</Dialog.Header>
		<form
			class="flex flex-col gap-4"
			onsubmit={(e) => {
				e.preventDefault();
				create();
			}}
		>
			<Input
				bind:value={title}
				placeholder={t('dialogs.edit_playlist.name_placeholder')}
				aria-label={t('dialogs.edit_playlist.name_label')}
				autofocus
			/>
			{#if signedIn}
				<RadioGroup.Root
					value={local ? 'device' : 'account'}
					onValueChange={choose}
					class="gap-2"
					aria-label={t('dialogs.new_playlist.where')}
				>
					{#each options as o (o.value)}
						{@const off = o.value === 'account' && hasLocalFiles}
						<label
							class="flex items-center gap-3 rounded-2xl border px-3 py-2.5 transition-colors has-[[data-state=checked]]:border-primary has-[[data-state=checked]]:bg-primary/5 {off
								? 'cursor-not-allowed opacity-50'
								: 'cursor-pointer hover:bg-accent/10'}"
						>
							<RadioGroup.Item value={o.value} disabled={off} />
							<HugeiconsIcon icon={o.icon} class="h-5 w-5 shrink-0 text-muted-foreground" />
							<div class="min-w-0">
								<div class="text-sm font-medium">{t(`dialogs.new_playlist.${o.label}`)}</div>
								<p class="text-xs text-muted-foreground">{t(`dialogs.new_playlist.${o.desc}`)}</p>
							</div>
						</label>
					{/each}
				</RadioGroup.Root>
				{#if hasLocalFiles}
					<p class="text-xs text-muted-foreground">{t('dialogs.new_playlist.local_files')}</p>
				{/if}
			{:else}
				<p class="flex items-start gap-2 text-xs text-muted-foreground">
					<HugeiconsIcon icon={ComputerIcon} class="mt-px h-3.5 w-3.5 shrink-0" />
					{t('dialogs.new_playlist.device_only')}
				</p>
			{/if}
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={close}>{t('common.cancel')}</Button>
				<Button type="submit" disabled={creating || !title.trim()}>
					{creating ? t('common.loading') : t('common.create')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
