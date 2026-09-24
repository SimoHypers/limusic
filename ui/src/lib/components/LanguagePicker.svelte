<script lang="ts">
	// The language list, as a panel under the settings row rather than a dropdown. Twelve catalogs
	// already ran the select off the bottom of the dialog, and Weblate keeps sending more, so the
	// list needs a search field and a shape that grows sideways instead of down.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon, Tick02Icon, TranslateIcon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import * as api from '$lib/api';
	import { matchLocales } from '$lib/langlist';
	import { t, LOCALES, COVERAGE, currentLocale, type LocaleId } from '$lib/i18n.svelte';

	let { onpick, onclose }: { onpick: (id: LocaleId) => void; onclose: () => void } = $props();

	let query = $state('');
	let search = $state<HTMLInputElement | null>(null);
	let panel = $state<HTMLDivElement | null>(null);

	// The panel only exists while it is open, so this runs on open and nowhere else: type straight
	// away, and see where you currently are without hunting for the tick.
	$effect(() => {
		search?.focus();
		panel?.querySelector('[aria-current]')?.scrollIntoView({ block: 'nearest' });
	});

	const matches = $derived(matchLocales(LOCALES, query));

	// ponytail: 90% is where a catalog starts reading as English in normal use. Below it the number
	// is worth showing, above it it is noise on eleven rows out of twelve.
	const PARTIAL_BELOW = 0.9;

	/** English name, translated share, or both: whichever of the two this language has to say. */
	function subLabel(locale: (typeof LOCALES)[number]): string {
		const parts: string[] = [];
		if (locale.englishLabel !== locale.nativeLabel) parts.push(locale.englishLabel);
		const done = COVERAGE[locale.id as LocaleId];
		if (done < PARTIAL_BELOW) parts.push(`${Math.round(done * 100)}%`);
		return parts.join(' · ');
	}

	// Escape belongs to the panel while the panel is open, otherwise it closes the whole settings
	// dialog and the person loses their place.
	function onkeydown(e: KeyboardEvent) {
		if (e.key !== 'Escape') return;
		e.stopPropagation();
		e.preventDefault();
		if (query) query = '';
		else onclose();
	}
</script>

<div bind:this={panel} class="rounded-xl border bg-background/60 p-2" {onkeydown} role="presentation">
	<div class="relative">
		<HugeiconsIcon
			icon={Search01Icon}
			strokeWidth={2}
			class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
		/>
		<Input
			bind:ref={search}
			bind:value={query}
			class="h-9 pl-9"
			placeholder={t('settings.general.language_search')}
			aria-label={t('settings.general.language_search')}
		/>
	</div>

	{#if matches.length}
		<!-- Two columns: sixteen names down one column is the dropdown again, only inside a card. -->
		<div class="mt-2 grid max-h-64 grid-cols-2 gap-1.5 overflow-y-auto pr-1">
			{#each matches as locale (locale.id)}
				{@const active = locale.id === currentLocale.id}
				<button
					type="button"
					onclick={() => onpick(locale.id)}
					aria-current={active ? 'true' : undefined}
					class="flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left transition-colors {active
						? 'border-primary/50 bg-primary/10'
						: 'border-transparent bg-muted/40 hover:bg-muted'}"
				>
					<span class="min-w-0 flex-1">
						<span class="block truncate text-sm font-medium">{locale.nativeLabel}</span>
						<!-- Fixed height so a tile with nothing to say here is the same size as its
						     neighbour: English is its own English name and is never partial. -->
						<span class="block h-4 truncate text-[11px] leading-4 text-muted-foreground"
							>{subLabel(locale)}</span
						>
					</span>
					{#if active}
						<HugeiconsIcon icon={Tick02Icon} strokeWidth={2} class="size-4 shrink-0 text-primary" />
					{/if}
				</button>
			{/each}
		</div>
	{:else}
		<p class="px-1 py-6 text-center text-xs text-muted-foreground">{t('common.no_matches')}</p>
	{/if}

	<!-- Every catalog but English is volunteer work in progress, and this is where somebody notices
	     their language is at 61%. -->
	<button
		type="button"
		onclick={() => api.openExternal('https://hosted.weblate.org/engage/limusic/')}
		class="mt-2 flex w-full cursor-pointer items-center justify-center gap-1.5 rounded-lg px-2 py-1.5 text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
	>
		<HugeiconsIcon icon={TranslateIcon} strokeWidth={2} class="size-3.5" />
		{t('settings.general.language_contribute')}
	</button>
</div>
