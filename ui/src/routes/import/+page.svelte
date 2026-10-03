<script lang="ts">
	import { open as pickFile } from '@tauri-apps/plugin-dialog';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		FileImportIcon,
		FolderOpenIcon,
		PlayIcon,
		Cancel01Icon
	} from '@hugeicons/core-free-icons';
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Textarea } from '$lib/components/ui/textarea';
	import * as Tabs from '$lib/components/ui/tabs';
	import { importerState, loadFromFile, loadFromText, loadFromSpotifyLink, startMatching, cancel, pickAllMatched, selectedSongs } from '$lib/importer.svelte';
	import { openNewPlaylist, openAddManyToPlaylist, ui } from '$lib/player.svelte';
	import ImportRow from '$lib/components/ImportRow.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import { t } from '$lib/i18n.svelte';

	let pasteText = $state('');
	let spotifyUrl = $state('');
	let showPaste = $state(false);
	let activeTab = $state('all');

	async function handlePickFile() {
		try {
			const picked = await pickFile({
				filters: [{ name: 'CSV', extensions: ['csv'] }]
			});
			if (typeof picked === 'string') {
				await loadFromFile(picked);
			}
		} catch (e) {
			console.error('Failed to pick file', e);
		}
	}

	async function handleParseText() {
		if (!pasteText.trim()) return;
		await loadFromText(pasteText);
		showPaste = false;
		pasteText = '';
	}

	async function handleLoadSpotify() {
		if (!spotifyUrl.trim() || importerState.loading) return;
		await loadFromSpotifyLink(spotifyUrl);
		spotifyUrl = '';
	}

	const counts = $derived.by(() => {
		let matched = 0;
		let review = 0;
		let notFound = 0;
		for (let i = 0; i < importerState.tracks.length; i++) {
			const res = importerState.results[i];
			if (!res) continue;
			if (res.status === 'matched') matched++;
			else if (res.status === 'review') review++;
			else if (res.status === 'not_found') notFound++;
		}
		return {
			all: importerState.tracks.length,
			matched,
			review,
			notFound
		};
	});

	const filteredIndices = $derived.by(() => {
		const indices: number[] = [];
		for (let i = 0; i < importerState.tracks.length; i++) {
			const res = importerState.results[i];
			if (activeTab === 'all') {
				indices.push(i);
			} else if (activeTab === 'matched' && res?.status === 'matched') {
				indices.push(i);
			} else if (activeTab === 'review' && res?.status === 'review') {
				indices.push(i);
			} else if (activeTab === 'not_found' && (res?.status === 'not_found' || !res)) {
				indices.push(i);
			}
		}
		return indices;
	});

	const selectedCount = $derived(importerState.picked.filter(Boolean).length);
	const totalCount = $derived(importerState.tracks.length);
	const hasResults = $derived(importerState.results.some((r) => r !== undefined));
</script>

<div class="p-6 max-w-6xl mx-auto pb-32">
	<div class="mb-6">
		<h1 class="font-heading text-2xl font-bold tracking-tight">{t('import.title')}</h1>
		<p class="mt-1 text-sm text-muted-foreground">{t('import.subtitle')}</p>
	</div>

	<form
		class="mb-6 flex gap-2"
		onsubmit={(e) => {
			e.preventDefault();
			handleLoadSpotify();
		}}
	>
		<Input
			bind:value={spotifyUrl}
			placeholder={t('import.spotify_placeholder')}
			disabled={importerState.loading}
			class="flex-1"
		/>
		<Button type="submit" disabled={!spotifyUrl.trim() || importerState.loading}>
			{importerState.loading ? t('common.loading') : t('import.load_playlist')}
		</Button>
	</form>

	<div class="mb-4 text-xs text-muted-foreground">
		{t('import.or_csv')}
	</div>

	<div class="mb-6 flex flex-wrap items-center gap-2">
		<Button variant="outline" class="gap-2 rounded-full" onclick={handlePickFile}>
			<HugeiconsIcon icon={FolderOpenIcon} class="h-4 w-4" /> {t('import.select_file')}
		</Button>
		<Button
			variant="outline"
			class="gap-2 rounded-full"
			onclick={() => (showPaste = !showPaste)}
		>
			<HugeiconsIcon icon={FileImportIcon} class="h-4 w-4" /> {t('import.paste_csv')}
		</Button>
	</div>

	{#if showPaste}
		<div class="mb-6 rounded-2xl border bg-card p-4 space-y-3">
			<Textarea
				bind:value={pasteText}
				placeholder={t('import.paste_placeholder')}
				rows={6}
				class="font-mono text-xs"
			/>
			<div class="flex justify-end gap-2">
				<Button variant="ghost" onclick={() => (showPaste = false)}>{t('common.cancel')}</Button>
				<Button onclick={handleParseText} disabled={!pasteText.trim()}>{t('import.load_text')}</Button>
			</div>
		</div>
	{/if}

	{#if importerState.error}
		<div class="mb-6">
			<ErrorState
				message={importerState.error}
				onRetry={importerState.retryable ? () => (importerState.error = null) : undefined}
			/>
		</div>
	{/if}

	{#if importerState.fromSpotify}
		<p class="mb-4 text-xs text-muted-foreground">{t('import.spotify_limit')}</p>
	{/if}

	{#if importerState.tracks.length > 0}
		<div class="mb-6 flex flex-wrap items-center justify-between gap-4 rounded-2xl border bg-card/40 p-4">
			<div class="flex items-center gap-3">
				<div class="text-sm font-medium">
					{importerState.tracks.length} tracks loaded
				</div>
				{#if importerState.running}
					<div class="flex items-center gap-2 text-xs text-muted-foreground">
						<span>{t('import.progress', { done: importerState.done, total: importerState.total })}</span>
					</div>
				{/if}
			</div>

			<div class="flex items-center gap-3">
				{#if importerState.running}
					<Button variant="destructive" class="gap-2 rounded-full" onclick={cancel}>
						<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" /> {t('import.cancel')}
					</Button>
				{:else}
					<Button class="gap-2 rounded-full" onclick={startMatching}>
						<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> {t('import.find_matches')}
					</Button>
				{/if}
			</div>

			{#if importerState.running}
				<div class="w-full bg-secondary h-2 rounded-full overflow-hidden mt-2">
					<div
						class="bg-primary h-full transition-all duration-200"
						style="width: {importerState.total > 0 ? (importerState.done / importerState.total) * 100 : 0}%"
					></div>
				</div>
			{/if}
		</div>

		<Tabs.Root bind:value={activeTab} class="space-y-4">
			<Tabs.List>
				<Tabs.Trigger value="all">{t('import.tab_all', { count: counts.all })}</Tabs.Trigger>
				<Tabs.Trigger value="matched">{t('import.tab_matched', { count: counts.matched })}</Tabs.Trigger>
				<Tabs.Trigger value="review">{t('import.tab_review', { count: counts.review })}</Tabs.Trigger>
				<Tabs.Trigger value="not_found">{t('import.tab_not_found', { count: counts.notFound })}</Tabs.Trigger>
			</Tabs.List>

			<Tabs.Content value={activeTab} class="space-y-2">
				{#if filteredIndices.length === 0}
					<p class="py-8 text-center text-sm text-muted-foreground">{t('common.no_matches')}</p>
				{:else}
					{#each filteredIndices as i (i)}
						<ImportRow
							index={i}
							track={importerState.tracks[i]}
							result={importerState.results[i]}
							choice={importerState.choice[i]}
						/>
					{/each}
				{/if}
			</Tabs.Content>
		</Tabs.Root>
	{:else if !showPaste}
		<div class="py-16 text-center text-muted-foreground">
			<p class="text-sm">{t('import.subtitle')}</p>
		</div>
	{/if}

	{#if hasResults}
		<div
			transition:fly={{ y: 16, duration: 220, easing: cubicOut }}
			class="fixed bottom-24 left-1/2 z-40 flex max-w-[min(48rem,calc(100vw-2rem))] -translate-x-1/2 flex-wrap items-center justify-between gap-3 rounded-2xl border bg-card px-4 py-3 shadow-lg"
		>
			<div class="flex items-center gap-3 text-sm">
				<span class="font-medium">
					{t('import.selected_of_total', { selected: selectedCount, total: totalCount })}
				</span>
			</div>
			<div class="flex flex-wrap items-center gap-2">
				<Button variant="outline" size="sm" class="rounded-full text-xs" onclick={pickAllMatched}>
					{t('import.select_all_matched')}
				</Button>
				<Button
					variant="outline"
					size="sm"
					class="rounded-full text-xs"
					disabled={selectedCount === 0 || ui.addPending}
					onclick={() => openNewPlaylist(selectedSongs(), importerState.source.replace(/\.csv$/i, ''))}
				>
					{t('import.add_to_new_playlist')}
				</Button>
				<Button
					size="sm"
					class="rounded-full text-xs"
					disabled={selectedCount === 0 || ui.addPending}
					onclick={() => openAddManyToPlaylist(selectedSongs())}
				>
					{t('import.add_to_existing_playlist')}
				</Button>
			</div>
		</div>
	{/if}
</div>
