<script lang="ts">
	// "Edit playlist" on a playlist you own: name, description, visibility and a cover of your own.
	//
	// The three text/visibility fields are one write, sent on Save and only for what actually
	// changed. The cover applies the moment a file is picked: it is stored on this machine (so it
	// draws instantly and offline) and uploaded to YouTube Music behind the picker.
	import { open as pickFile } from '@tauri-apps/plugin-dialog';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ImageAdd02Icon, Delete02Icon } from '@hugeicons/core-free-icons';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import * as api from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	/** What the page shows while YouTube catches up, and what it puts back if the write fails. */
	type Edit = { title?: string; description?: string; privacy?: string; cover?: string };

	let {
		open = $bindable(false),
		id,
		title,
		description,
		privacy,
		cover,
		fallback,
		onchange
	}: {
		open: boolean;
		id: string;
		title?: string;
		description?: string;
		privacy?: string;
		/** Custom artwork already stored for this playlist. */
		cover?: string;
		/** YouTube's own artwork, shown when there is no custom one. */
		fallback?: string;
		onchange: (patch: Edit) => void;
	} = $props();

	let draftName = $state('');
	let draftDescription = $state('');
	let isPublic = $state(false);
	let saving = $state(false);
	let removing = $state(false);

	// Fill the form from the page each time it opens, so a cancelled edit leaves nothing behind.
	// Guarded on `open` before anything else is read: while closed, the props aren't tracked, so a
	// mid-edit optimistic update on the page can't reach in and rewrite the draft.
	$effect(() => {
		if (!open) return;
		draftName = title ?? '';
		draftDescription = description ?? '';
		isPublic = privacy === 'PUBLIC';
	});

	const preview = $derived(thumb(cover ?? fallback, 400));

	async function pickCover() {
		// JPEG and PNG only, because that is what YouTube's uploader accepts (WebP comes back 415).
		// Keeping the picker to those beats letting someone choose a file that can only ever be
		// this machine's copy.
		const picked = await pickFile({
			multiple: false,
			title: 'Choose playlist artwork',
			filters: [{ name: 'Images', extensions: ['jpg', 'jpeg', 'png'] }]
		});
		if (typeof picked === 'string') await storeCover(picked);
	}

	// Picking answers as soon as the file is copied. Removing waits on YouTube, because its own
	// thumbnail is the cover being removed: dropping the local copy first would swap the header to
	// that same image and only reach the rebuilt collage a beat later.
	async function storeCover(path: string | null) {
		if (removing) return;
		removing = path === null;
		try {
			const { cover: saved, thumbnail } = await api.setPlaylistCover(id, path);
			onchange({ cover: saved ?? undefined, ...(thumbnail ? { thumbnail } : {}) });
		} catch (e) {
			toast.error(String(e));
		} finally {
			removing = false;
		}
	}

	async function save() {
		if (saving) return;
		const name = draftName.trim();
		const changes: { name?: string; description?: string; public?: boolean } = {};
		if (name && name !== title) changes.name = name;
		if (draftDescription !== (description ?? '')) changes.description = draftDescription;
		if (isPublic !== (privacy === 'PUBLIC')) changes.public = isPublic;
		if (!Object.keys(changes).length) {
			open = false;
			return;
		}
		const before: Edit = { title, description, privacy };
		saving = true;
		onchange({
			title: changes.name ?? title,
			description: changes.description ?? description,
			privacy: changes.public === undefined ? privacy : changes.public ? 'PUBLIC' : 'PRIVATE'
		});
		open = false;
		try {
			await api.editPlaylistDetails(id, changes);
			toast.success('Playlist updated');
		} catch (e) {
			onchange(before);
			toast.error(String(e));
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-xl">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.edit_playlist.title')}</Dialog.Title>
			<Dialog.Description>{t('dialogs.edit_playlist.desc_placeholder')}</Dialog.Description>
		</Dialog.Header>
		<form
			class="flex flex-col gap-4"
			onsubmit={(e) => {
				e.preventDefault();
				save();
			}}
		>
			<div class="flex gap-4">
				<div class="flex shrink-0 flex-col items-center gap-1.5">
					<button
						type="button"
						class="group relative h-32 w-32 cursor-pointer overflow-hidden rounded-xl border bg-muted"
						onclick={pickCover}
						aria-label={t('dialogs.edit_playlist.change_cover')}
					>
						{#if preview}
							<img src={preview} alt="" class="h-full w-full object-cover" />
						{/if}
						<!-- One conditional base opacity, not `opacity-0` plus a second `opacity-100`:
						     two utilities of the same specificity are settled by stylesheet order, so
						     the prompt could stay invisible on the playlist that most needs it. -->
						<span
							class="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-black/60 text-xs font-medium text-white transition group-hover:opacity-100 group-focus-visible:opacity-100 {preview
								? 'opacity-0'
								: 'opacity-100'}"
						>
							<HugeiconsIcon icon={ImageAdd02Icon} class="h-6 w-6" />
							{t('dialogs.edit_playlist.change_cover')}
						</span>
					</button>
					{#if cover}
						<Button
							type="button"
							variant="ghost"
							size="sm"
							class="gap-1.5 text-xs text-muted-foreground"
							onclick={() => storeCover(null)}
							disabled={removing}
						>
							<HugeiconsIcon icon={Delete02Icon} class="h-3.5 w-3.5" />
							{removing ? t('common.loading') : t('dialogs.edit_playlist.remove_cover')}
						</Button>
					{/if}
				</div>
				<div class="flex min-w-0 flex-1 flex-col gap-3">
					<Input bind:value={draftName} placeholder={t('dialogs.edit_playlist.name_placeholder')} aria-label={t('dialogs.edit_playlist.name_label')} />
					<textarea
						bind:value={draftDescription}
						placeholder={t('dialogs.edit_playlist.desc_placeholder')}
						aria-label={t('dialogs.edit_playlist.desc_label')}
						rows="4"
						class="w-full flex-1 resize-none rounded-2xl border border-input bg-input/30 px-3 py-2 text-sm outline-none transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
					></textarea>
				</div>
			</div>
			<div class="flex items-center justify-between gap-4 rounded-2xl border px-3 py-2.5">
				<div class="min-w-0">
					<div class="text-sm font-medium">Public</div>
					<p class="text-xs text-muted-foreground">
						{isPublic
							? 'Anyone can find this playlist on YouTube Music.'
							: 'Only you can see this playlist.'}
					</p>
				</div>
				<Switch bind:checked={isPublic} aria-label="Public playlist" />
			</div>
			<p class="text-xs text-muted-foreground">
				Artwork applies here at once and uploads to YouTube Music in the background. Square JPEG or
				PNG works best.
			</p>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}>{t('common.cancel')}</Button>
				<Button type="submit" disabled={saving || !draftName.trim()}>
					{saving ? t('common.loading') : t('dialogs.edit_playlist.save_btn')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
