<!--
	What's new, in Settings > About. The text is the GitHub release description verbatim (see
	RELEASING.md for the format releases are written in), so the changelog has one source of truth.

	The markdown here is deliberately tiny: headings, bullets, bold, code, links, @mentions, images.
	That is the whole vocabulary the release-note format uses, so a markdown dependency would be
	40 KB to render six constructs. GitHub's own editor drops in raw HTML for images and their
	<p align="center"> wrappers, so those are handled too and any other stray tag is dropped rather
	than shown as text.
-->
<script lang="ts">
	import { releaseNotes, openExternal } from '$lib/api';
	import { t } from '$lib/i18n.svelte';

	let { current }: { current: string } = $props();

	const load = releaseNotes();

	type Span = { t: 'text' | 'b' | 'code' | 'link'; s: string; href?: string };
	type Block =
		| { t: 'h'; spans: Span[] }
		| { t: 'p'; spans: Span[] }
		| { t: 'ul'; items: Span[][] }
		| { t: 'pre'; text: string }
		| { t: 'img'; src: string; alt: string };

	/** ![alt](src) and GitHub's raw <img src=...>, which it writes when you paste a screenshot. */
	const IMAGE = /!\[([^\]]*)\]\(([^)]+)\)|<img\b[^>]*?\bsrc=["']([^"']+)["'][^>]*>/gi;

	const INLINE = /\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\(([^)]+)\)|(https?:\/\/\S+)|@([\w-]+)/g;

	function inline(text: string): Span[] {
		const out: Span[] = [];
		let last = 0;
		for (const m of text.matchAll(INLINE)) {
			if (m.index > last) out.push({ t: 'text', s: text.slice(last, m.index) });
			if (m[1]) out.push({ t: 'b', s: m[1] });
			else if (m[2]) out.push({ t: 'code', s: m[2] });
			else if (m[3]) out.push({ t: 'link', s: m[3], href: m[4] });
			else if (m[5]) out.push({ t: 'link', s: m[5], href: m[5] });
			else out.push({ t: 'link', s: `@${m[6]}`, href: `https://github.com/${m[6]}` });
			last = m.index + m[0].length;
		}
		if (last < text.length) out.push({ t: 'text', s: text.slice(last) });
		return out;
	}

	/** Line-based on purpose: release notes are headings, bullets and one-line paragraphs. */
	function parse(body: string): Block[] {
		const blocks: Block[] = [];
		let fenced: string[] | null = null;
		for (const raw of body.replaceAll('\r', '').split('\n')) {
			let line = raw.trim();
			if (line.startsWith('```')) {
				if (fenced) blocks.push({ t: 'pre', text: fenced.join('\n') });
				fenced = fenced ? null : [];
				continue;
			}
			if (fenced) {
				fenced.push(raw);
				continue;
			}
			const images = [...line.matchAll(IMAGE)];
			// Whatever HTML is left over (<p align="center">, <br>, <details>) is layout, not content.
			line = line.replace(IMAGE, '').replace(/<\/?[a-z][^>]*>/gi, '').trim();
			for (const m of images) blocks.push({ t: 'img', src: m[2] ?? m[3], alt: m[1] ?? '' });
			if (!line) continue;
			const heading = line.match(/^#{1,6}\s+(.*)/);
			const bullet = line.match(/^[-*]\s+(.*)/);
			if (heading) blocks.push({ t: 'h', spans: inline(heading[1]) });
			else if (bullet) {
				const prev = blocks.at(-1);
				if (prev?.t === 'ul') prev.items.push(inline(bullet[1]));
				else blocks.push({ t: 'ul', items: [inline(bullet[1])] });
			} else blocks.push({ t: 'p', spans: inline(line) });
		}
		return blocks;
	}

	const day = (iso: string) =>
		iso ? new Date(iso).toLocaleDateString(undefined, { dateStyle: 'medium' }) : '';
</script>

<!-- prettier-ignore -->
{#snippet spans(list: Span[])}{#each list as s}{#if s.t === 'b'}<strong class="font-medium text-foreground">{s.s}</strong>{:else if s.t === 'code'}<code class="rounded bg-muted px-1 py-0.5 text-xs">{s.s}</code>{:else if s.t === 'link'}<button type="button" class="text-primary hover:underline" onclick={() => openExternal(s.href!)}>{s.s}</button>{:else}{s.s}{/if}{/each}{/snippet}

{#await load}
	<p class="py-2 text-sm text-muted-foreground">{t('common.loading')}</p>
{:then releases}
	{#each releases as r, i (r.version)}
		<details class="border-b last:border-b-0" open={i === 0}>
			<summary class="flex cursor-pointer items-center gap-2 py-2 text-sm marker:text-muted-foreground">
				<span class="font-medium">{t('changelog.version', { version: r.version })}</span>
				{#if r.version === current}
					<span class="rounded bg-primary/15 px-1.5 py-0.5 text-xs text-primary">{t('changelog.installed')}</span>
				{/if}
				<span class="ml-auto text-xs text-muted-foreground">{day(r.date)}</span>
			</summary>
			<div class="pb-3 text-sm text-muted-foreground">
				{#each parse(r.body) as b}
					{#if b.t === 'h'}
						<h4 class="mt-3 mb-1 font-medium text-foreground">{@render spans(b.spans)}</h4>
					{:else if b.t === 'p'}
						<p class="mt-1">{@render spans(b.spans)}</p>
					{:else if b.t === 'img'}
						<img
							src={b.src}
							alt={b.alt}
							loading="lazy"
							class="mt-2 max-w-full rounded-lg border"
						/>
					{:else if b.t === 'pre'}
						<pre
							class="mt-2 overflow-x-auto rounded bg-muted px-2 py-1.5 text-xs">{b.text}</pre>
					{:else}
						<ul class="mt-1 ml-4 list-disc space-y-1">
							{#each b.items as item}<li>{@render spans(item)}</li>{/each}
						</ul>
					{/if}
				{/each}
			</div>
		</details>
	{:else}
		<p class="py-2 text-sm text-muted-foreground">{t('changelog.no_releases')}</p>
	{/each}
{:catch e}
	<p class="py-2 text-sm text-muted-foreground">{t('changelog.load_failed', { error: e })}</p>
{/await}
