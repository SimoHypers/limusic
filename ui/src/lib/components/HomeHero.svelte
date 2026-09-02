<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { HistoryIcon, Search01Icon } from '@hugeicons/core-free-icons';
	import SearchSuggest from '$lib/components/SearchSuggest.svelte';
	import { auth, playback } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t, type TranslationKey } from '$lib/i18n.svelte';

	// Fixed at mount — a greeting that flips mid-session is uncanny.
	const hour = new Date().getHours();
	const daypartKey: TranslationKey =
		hour < 5
			? 'home.good_night'
			: hour < 12
				? 'home.good_morning'
				: hour < 18
					? 'home.good_afternoon'
					: 'home.good_evening';
	const daypart = $derived(t(daypartKey));

	let searchQuery = $state('');

	function goSearch() {
		if (!searchQuery.trim()) return;
		goto(`/search?${new URLSearchParams({ q: searchQuery }).toString()}`);
	}

	// Google's CDN doesn't serve every rewritten size, so a 404'd backdrop must degrade to nothing
	// rendered, never a broken-image glyph. Re-arm whenever the track changes, mirroring MediaCard.
	let artFailed = $state(false);
	$effect(() => {
		playback.now?.thumbnail; // re-arm when the track changes
		artFailed = false;
	});
</script>

<!-- overflow-hidden lives on the backdrop wrapper, not the hero: the scaled blur has to be clipped,
     but the search preview below has to hang out past the bottom edge. -->
<div class="relative border-b">
	<div class="pointer-events-none absolute inset-0 overflow-hidden">
		{#if playback.now?.thumbnail && !artFailed}
			<!-- 96px, not display size: blur-2xl is a 40px blur, so every detail above a handful of
			     pixels is thrown away anyway. The old 1200px source decoded to 5.7 MiB for this, and
			     re-decoded on every track change. -->
			<img
				src={thumb(playback.now.thumbnail, 96)}
				alt=""
				class="pointer-events-none absolute inset-0 h-full w-full art-wash scale-110 object-cover opacity-60 blur-2xl"
				onerror={() => (artFailed = true)}
			/>
		{:else}
			<!-- Nothing playing: without this the header is a bare strip with a greeting in it. An accent
			     wash keeps it a header. Inline style so it can't be lost to a stale dev stylesheet, and it
			     rides --primary so every preset theme gets its own. -->
			<div
				class="pointer-events-none absolute inset-0 opacity-[0.18]"
				style="background:radial-gradient(120% 130% at 12% 0%, var(--primary) 0%, transparent 58%)"
			></div>
		{/if}
		<div
			class="absolute inset-0 bg-gradient-to-t from-background via-background/70 to-background/40"
		></div>
		<div
			class="absolute inset-0 bg-gradient-to-r from-background/80 via-background/30 to-transparent"
		></div>
	</div>
	<div class="relative p-6 pt-8">
		<div class="flex items-start justify-between gap-4">
			<div class="flex min-w-0 items-center gap-3">
				{#if auth.account?.signedIn && auth.account.thumbnail}
					<!-- max-width:none defeats Tailwind Preflight's `img{max-width:100%}`, which in a tight box
					     clamps width to the content-box while height stays fixed → a vertical oval. Inline so
					     it's immune to Preflight and to stale dev CSS. -->
					<img
						src={thumb(auth.account.thumbnail, 128)}
						alt=""
						style="width:2.75rem;height:2.75rem;max-width:none"
						class="shrink-0 rounded-full object-cover ring-2 ring-border"
					/>
				{/if}
				<h1 class="truncate font-heading text-4xl font-bold tracking-tight drop-shadow">
					{daypart}{auth.account?.name ? `, ${auth.account.name.split(' ')[0]}` : ''}
				</h1>
			</div>
			<div class="flex shrink-0 items-center gap-2">
				<!-- Listen Together moved out of here and lives on the titlebar alone: history is the thing
				     you reach for from the home page. -->
				<button
					onclick={() => goto('/history')}
					title={t('nav.history')}
					aria-label={t('nav.history')}
					class="flex h-9 w-9 shrink-0 cursor-pointer items-center justify-center rounded-full border border-border text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
				>
					<HugeiconsIcon icon={HistoryIcon} class="h-5 w-5" />
				</button>
				<form class="relative w-full max-w-xs" onsubmit={(e) => { e.preventDefault(); goSearch(); }}>
					<HugeiconsIcon
						icon={Search01Icon}
						class="pointer-events-none absolute left-3 top-1/2 z-10 h-4 w-4 -translate-y-1/2 text-muted-foreground"
					/>
					<!-- The panel is wider than this field and hangs off its right edge: the rows carry
					     artwork and two lines of text, which 20rem can't hold. -->
					<SearchSuggest
						bind:value={searchQuery}
						placeholder={t('common.search')}
						inputClass="rounded-full pl-9"
						panelClass="right-0 w-[26rem]"
					/>
				</form>
			</div>
		</div>
	</div>
</div>
