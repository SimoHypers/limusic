<script lang="ts">
	// The Shortcuts grid's "+" — pick from your library without hunting for a ⋯ menu. Stays open so
	// several can go in at once; already-added rows show a tick instead of vanishing.
	// ponytail: library only. Anything else in YouTube still gets there by dragging its card onto the
	// grid, or ⋯ → Add to shortcuts — wire a search box in here if that turns out to be the common case.
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Cancel01Icon, Tick02Icon, Add01Icon } from '@hugeicons/core-free-icons';
	import { thumb } from '$lib/thumb';
	import { addPick, library, loadLibrary, personal } from '$lib/player.svelte';
	import { mergeSaved } from '$lib/personal';
	import { t } from '$lib/i18n.svelte';

	let { onclose }: { onclose: () => void } = $props();

	let filter = $state('');
	// Playlists saved on this machine count as library too: signed out they're all there is.
	const matches = $derived(
		mergeSaved(personal, library.items, 'playlist').filter((i) =>
			i.title.toLowerCase().includes(filter.trim().toLowerCase())
		)
	);
	const already = (id: string) => personal.picks.some((p) => p.id === id);

	loadLibrary(); // no-op once the sidebar has it
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div
	transition:fade={{ duration: 150 }}
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
>
	<div
		transition:scale={{ duration: 180, start: 0.96, easing: cubicOut }}
		class="flex max-h-[32rem] w-full max-w-sm flex-col rounded-xl border bg-card p-4 shadow-xl"
	>
		<div class="mb-3 flex items-center justify-between">
			<h2 class="font-heading text-base font-semibold">{t('home.add_shortcut')}</h2>
			<button
				class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
				onclick={onclose}
				aria-label={t('a11y.close')}
			>
				<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
			</button>
		</div>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			bind:value={filter}
			autofocus
			placeholder={t('common.filter_library')}
			class="mb-2 w-full rounded-lg border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
		/>
		{#if library.loading && !library.items.length}
			<p class="p-2 text-sm text-muted-foreground">{t('common.loading')}</p>
		{:else if matches.length}
			<div class="min-h-0 flex-1 overflow-y-auto">
				{#each matches as item (item.id)}
					{@const on = already(item.id)}
					<button
						class="flex w-full cursor-pointer items-center gap-3 rounded-lg p-2 text-left hover:bg-accent/10 disabled:cursor-default disabled:opacity-60"
						disabled={on}
						onclick={() => addPick(item)}
					>
						{#if item.thumbnail}
							<img
								src={thumb(item.thumbnail, 96)}
								alt=""
								class="h-10 w-10 shrink-0 rounded-md object-cover"
							/>
						{:else}
							<div class="h-10 w-10 shrink-0 rounded-md bg-muted"></div>
						{/if}
						<div class="min-w-0 flex-1">
							<div class="truncate text-sm font-medium">{item.title}</div>
							{#if item.subtitle}
								<div class="truncate text-xs text-muted-foreground">{item.subtitle}</div>
							{/if}
						</div>
						<!-- altIcon/showAlt, not a ternary on `icon`: HugeiconsIcon freezes `icon` at mount. -->
						<HugeiconsIcon
							icon={Add01Icon}
							altIcon={Tick02Icon}
							showAlt={on}
							class="h-4 w-4 shrink-0 {on ? 'text-primary' : 'text-muted-foreground'}"
						/>
					</button>
				{/each}
			</div>
		{:else}
			<p class="p-2 text-sm text-muted-foreground">
				{filter.trim() ? t('common.nothing_matches') : t('library.no_playlists_hint')}
			</p>
		{/if}
	</div>
</div>
