<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { HistoryIcon, Search01Icon } from '@hugeicons/core-free-icons';
	import SearchSuggest from '$lib/components/SearchSuggest.svelte';
	import { auth, personal, playback } from '$lib/player.svelte';
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

<header class="relative">
	<!-- The canvas. It used to be clipped to the header with a rule under it, which boxed the greeting
	     in a strip; now it runs on well past the header, under the mood chips and the first section,
	     and fades into the page instead of ending at a line. -z-10 puts it under everything that
	     follows, inside the stacking context home's wrapper opens (`isolate`), so it can never slip
	     behind the window's own background.
	     `art-wash` is on the canvas, not the image, so the wash and the gradients that fade it out
	     rasterize into one layer. With the image promoted alone, WebKitGTK's animated wheel scroll
	     put the two layers a pixel apart at some offsets, and a row of the raw wash flickered under
	     the fade's last line (perf/navprobe.py --hold --wheel: 8 of 20 captures). -->
	<div
		class="art-wash pointer-events-none absolute inset-x-0 top-0 -z-10 h-[30rem] overflow-hidden"
		aria-hidden="true"
	>
		{#if personal.home.backdrop && playback.now?.thumbnail && !artFailed}
			<!-- 96px, not display size: blur-2xl is a 40px blur, so every detail above a handful of
			     pixels is thrown away anyway. The old 1200px source decoded to 5.7 MiB for this, and
			     re-decoded on every track change. -->
			<img
				src={thumb(playback.now.thumbnail, 96)}
				alt=""
				class="absolute inset-0 h-full w-full scale-110 object-cover opacity-70 blur-2xl"
				onerror={() => (artFailed = true)}
			/>
		{:else}
			<!-- Nothing playing, or the artwork switched off in Edit home: an accent wash keeps it a
			     header rather than a bare greeting. Inline style so it can't be lost to a stale dev
			     stylesheet, and it rides --primary so every preset theme gets its own. -->
			<div
				class="absolute inset-0 opacity-[0.2]"
				style="background:radial-gradient(90% 75% at 10% 0%, var(--primary) 0%, transparent 70%)"
			></div>
		{/if}
		<!-- Ends on the page's own background exactly at the canvas's bottom edge, so there is no seam. -->
		<div
			class="absolute inset-0 bg-gradient-to-b from-background/25 via-background/70 to-background"
		></div>
		<div
			class="absolute inset-0 bg-gradient-to-r from-background/60 via-background/15 to-transparent"
		></div>
	</div>

	<div class="flex items-center justify-between gap-6 px-6 pb-4 pt-10 max-sm:pb-3 max-sm:pt-6">
		<!-- Below sm the sidebar is a drawer, so the avatar and greeting have nothing to line up with
		     and the search field needs the row: the greeting goes, the row keeps history and search. -->
		<div class="flex min-w-0 items-center gap-4 max-sm:hidden">
			{#if auth.account?.signedIn && auth.account.thumbnail}
				<!-- max-width:none defeats Tailwind Preflight's `img{max-width:100%}`, which in a tight box
				     clamps width to the content-box while height stays fixed → a vertical oval. Inline so
				     it's immune to Preflight and to stale dev CSS. -->
				<img
					src={thumb(auth.account.thumbnail, 128)}
					alt=""
					style="width:3rem;height:3rem;max-width:none"
					class="shrink-0 rounded-full object-cover shadow-md"
				/>
			{/if}
			<h1 class="truncate font-heading text-4xl font-bold tracking-tight drop-shadow-sm">
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
</header>
