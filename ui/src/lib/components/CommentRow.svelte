<script lang="ts">
	import type { Snippet } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CheckmarkBadge01Icon,
		FavouriteIcon,
		Pin02Icon,
		ThumbsUpIcon
	} from '@hugeicons/core-free-icons';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar';
	import type { Comment } from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import { thumb } from '$lib/thumb';

	// A comment is another person's text: everything below renders as text (never markup), and a
	// link in it is just characters. No transitions: rows stream under a stationary pointer while
	// the list scrolls (docs/UI-PERFORMANCE.md).
	let {
		comment,
		reply = false,
		children
	}: { comment: Comment; reply?: boolean; children?: Snippet } = $props();

	// A comment this long is cut to a few lines with a "Show more". By length, not by measuring:
	// layout is read on demand, never at mount.
	const LONG = 280;
	const long = $derived(comment.text.length > LONG || comment.text.split('\n').length > 6);
	let expanded = $state(false);

	const author = $derived(comment.author);
</script>

<div class="flex gap-3 px-4 py-2.5" data-row>
	<!-- after:hidden: the shadcn avatar's blended hairline is a pseudo-element per row. -->
	<Avatar class="{reply ? 'size-6' : 'size-8'} after:hidden">
		{#if author.avatar}<AvatarImage src={thumb(author.avatar, 64)} alt="" />{/if}
		<AvatarFallback class="text-xs"
			>{author.name.replace(/^@/, '').slice(0, 1).toUpperCase()}</AvatarFallback
		>
	</Avatar>
	<div class="min-w-0 flex-1">
		{#if comment.pinned}
			<p class="mb-0.5 flex items-center gap-1 text-xs text-muted-foreground">
				<HugeiconsIcon icon={Pin02Icon} class="h-3 w-3" />
				{t('comments.pinned')}
			</p>
		{/if}
		<div class="flex flex-wrap items-center gap-x-1.5 text-xs">
			<span class="max-w-full min-w-0 font-medium break-words">{author.name}</span>
			{#if author.verified}
				<span class="text-muted-foreground" title={t('comments.verified')}>
					<HugeiconsIcon icon={CheckmarkBadge01Icon} class="h-3.5 w-3.5" />
					<span class="sr-only">{t('comments.verified')}</span>
				</span>
			{/if}
			{#if author.is_creator}
				<span class="rounded bg-muted px-1 text-[0.625rem] text-muted-foreground"
					>{t('comments.creator')}</span
				>
			{/if}
			{#if author.is_artist}
				<span class="rounded bg-muted px-1 text-[0.625rem] text-muted-foreground"
					>{t('comments.artist')}</span
				>
			{/if}
			{#if comment.published}<span class="text-muted-foreground">{comment.published}</span>{/if}
		</div>
		<p
			class="mt-1 text-sm break-words whitespace-pre-wrap {long && !expanded ? 'line-clamp-5' : ''}"
		>
			{comment.text}
		</p>
		{#if long}
			<button
				class="mt-0.5 cursor-pointer text-xs text-muted-foreground hover:text-foreground"
				onclick={() => (expanded = !expanded)}
			>
				{expanded ? t('comments.show_less') : t('comments.show_more')}
			</button>
		{/if}
		<div class="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
			{#if comment.like_count}
				<span class="flex items-center gap-1">
					<HugeiconsIcon icon={ThumbsUpIcon} class="h-3.5 w-3.5" />
					{comment.like_count}
				</span>
			{/if}
			{#if comment.hearted}
				<span class="flex items-center gap-1" title={t('comments.hearted')}>
					<HugeiconsIcon icon={FavouriteIcon} class="h-3.5 w-3.5 text-primary" />
					<span class="sr-only">{t('comments.hearted')}</span>
				</span>
			{/if}
		</div>
		{@render children?.()}
	</div>
</div>
