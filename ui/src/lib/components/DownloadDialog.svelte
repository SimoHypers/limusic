<script lang="ts">
	// One setup surface for every entry point: a track row's menu, the player bar, or a whole
	// playlist. Format and quality live here; "Remember this choice" persists them and makes later
	// downloads one click. Confirming hands the jobs to the background engine and closes, so the
	// titlebar's Downloads popover is where progress lives and another download can start right
	// away.
	import { untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Alert02Icon, Download01Icon, Tick02Icon } from '@hugeicons/core-free-icons';
	import { save } from '@tauri-apps/plugin-dialog';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import * as api from '$lib/api';
	import type { DownloadFormat, DownloadQuality } from '$lib/api';
	import { DOWNLOAD_FORMATS, DOWNLOAD_QUALITIES } from '$lib/download';
	import { checkYtdlp, closeDownloads, downloads, startDownload } from '$lib/downloads.svelte';
	import { prefs } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let format = $state<DownloadFormat>('mp3');
	let quality = $state<DownloadQuality>('best');
	let errorMsg = $state('');
	let starting = $state(false);

	const isPlaylist = $derived(!!downloads.playlist);
	const count = $derived(downloads.playlist?.songs.length ?? 1);
	const title = $derived(
		downloads.playlist
			? t('player.download_playlist_title', { name: downloads.playlist.name })
			: (downloads.target?.title ?? t('player.download_track'))
	);

	$effect(() => {
		if (!downloads.open) return;
		// Untracked: `checkYtdlp` flips its own flags, and retriggering this reset would clear a
		// just-shown yt-dlp error.
		untrack(() => {
			format = prefs.downloadFormat;
			quality = prefs.downloadQuality;
			errorMsg = '';
			void checkYtdlp();
		});
	});

	/** Closes the dialog through the shared engine so the pending target is cleared too. */
function setOpen(open: boolean) {
		if (open) downloads.open = true;
		else closeDownloads();
	}

	/** Persists the checkbox, restoring it if the write fails. */
async function remember(on: boolean) {
		const previous = prefs.downloadRemember;
		prefs.downloadRemember = on;
		try {
			await api.setSetting('download_remember', on ? 'true' : 'false');
		} catch (e) {
			prefs.downloadRemember = previous;
			errorMsg = String(e);
		}
	}

	/** yt-dlp appends its own extension, so hand the command the name without one. */
	function withoutExtension(path: string): string {
		const suffix = `.${format}`;
		return path.toLowerCase().endsWith(suffix) ? path.slice(0, -suffix.length) : path;
	}

	/** Confirms the choice and hands the jobs to the background queue. */
async function run() {
		if (starting) return;
		errorMsg = '';
		const target = downloads.target;

		// A one-off download keeps the save dialog; a playlist owns its folder, and a remembered
		// choice means "no more asking".
		if (!isPlaylist && !prefs.downloadRemember) {
			if (!target) return;
			starting = true;
			let path: string | null = null;
			try {
				path = await save({
					defaultPath: await api.suggestDownloadPath(target.artists, target.title, format),
					filters: [{ name: format.toUpperCase(), extensions: [format] }],
					title: t('player.download_save_dialog')
				});
			} catch (e) {
				errorMsg = String(e);
				return;
			} finally {
				starting = false;
			}
			if (!path) return;
			startDownload({ format, quality, remember: false, dest: withoutExtension(path) });
			return;
		}

		startDownload({ format, quality, remember: prefs.downloadRemember });
	}
</script>

<Dialog.Root bind:open={() => downloads.open, (v) => setOpen(v)}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title class="flex items-center gap-2">
				<HugeiconsIcon icon={Download01Icon} class="h-4 w-4 shrink-0 text-primary" />
				<span class="min-w-0 truncate">{title}</span>
			</Dialog.Title>
			<Dialog.Description class="text-xs leading-relaxed">
				{isPlaylist
					? t('player.download_playlist_desc', {
							count,
							name: downloads.playlist?.name ?? ''
						})
					: t('player.download_desc')}
			</Dialog.Description>
		</Dialog.Header>

		{#if downloads.ytdlp.error}
			<p
				class="flex items-start gap-2 rounded-md bg-destructive/10 px-3 py-2 text-xs leading-relaxed text-destructive"
				role="alert"
			>
				<HugeiconsIcon icon={Alert02Icon} class="mt-0.5 h-3.5 w-3.5 shrink-0" />
				<span>{t('player.download_ytdlp_missing')}</span>
			</p>
		{/if}

		<div class="flex flex-col gap-4">
			<div>
				<p class="mb-1.5 text-xs font-medium text-muted-foreground">
					{t('player.download_format')}
				</p>
				<div class="flex flex-wrap gap-1">
					{#each DOWNLOAD_FORMATS as value (value)}
						<button
							type="button"
							class="cursor-pointer rounded-md px-2.5 py-1 text-xs font-medium transition
								{format === value
								? 'bg-primary text-primary-foreground'
								: 'bg-muted text-muted-foreground hover:text-foreground'}"
							aria-pressed={format === value}
							onclick={() => (format = value)}
						>
							{value.toUpperCase()}
						</button>
					{/each}
				</div>
			</div>

			<div>
				<p class="mb-1 text-xs font-medium text-muted-foreground">
					{t('player.download_quality')}
				</p>
				<div class="flex flex-col gap-0.5">
					{#each DOWNLOAD_QUALITIES as value (value)}
						<button
							type="button"
							class="flex cursor-pointer items-center rounded-md px-2 py-1 text-left text-xs transition hover:bg-muted
								{quality === value ? 'font-semibold text-primary' : 'text-muted-foreground'}"
							onclick={() => (quality = value)}
						>
							{value === 'best'
								? t('player.download_quality_best')
								: t(`player.download_quality_${value}`)}
							{#if quality === value}
								<HugeiconsIcon icon={Tick02Icon} class="ml-auto h-3.5 w-3.5" />
							{/if}
						</button>
					{/each}
				</div>
			</div>

			<label class="flex cursor-pointer items-start gap-2.5 rounded-lg border px-3 py-2.5">
				<Checkbox
					checked={prefs.downloadRemember}
					onCheckedChange={(on) => remember(!!on)}
					class="mt-0.5"
				/>
				<span class="min-w-0">
					<span class="block text-sm font-medium">{t('player.download_remember')}</span>
					<span class="block text-xs leading-relaxed text-muted-foreground">
						{t('player.download_remember_hint')}
					</span>
				</span>
			</label>

			{#if errorMsg}
				<p class="max-h-24 overflow-auto rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
					{errorMsg}
				</p>
			{/if}
		</div>

		<Dialog.Footer>
			<Button size="sm" disabled={starting || !!downloads.ytdlp.error} onclick={run}>
				{starting ? t('player.downloading') : t('player.download')}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
