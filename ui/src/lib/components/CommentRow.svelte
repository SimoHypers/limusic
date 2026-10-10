<script lang="ts">
	import type { Snippet } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CheckmarkBadge01Icon,
		FavouriteIcon,
		Pin02Icon,
		ThumbsDownIcon,
		ThumbsUpIcon
	} from '@hugeicons/core-free-icons';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar';
	import type { Comment, CommentAction } from '$lib/api';
	import { actOnComment, canAct, isActing, shownCount } from '$lib/comments.svelte';
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

	// Like and dislike buttons: only for a comment the response offered an action on (never signed
	// out) or one the viewer has voted on here. A lit button's action undoes its vote, an unlit
	// one's casts it; one that is not on offer stays visible but disabled, and says why.
	const count = $derived(shownCount(comment));
	const acting = $derived(isActing(comment));
	const liked = $derived(comment.vote === 'liked');
	const disliked = $derived(comment.vote === 'disliked');
	const likeAction = $derived<CommentAction>(liked ? 'unlike' : 'like');
	const dislikeAction = $derived<CommentAction>(disliked ? 'undislike' : 'dislike');
	const likeOffered = $derived(comment.actions.includes(likeAction));
	const dislikeOffered = $derived(comment.actions.includes(dislikeAction));
	// While a request is in flight the actions are in transit, not missing: keep the normal label.
	const likeLabel = $derived(
		liked
			? likeOffered || acting
				? t('comments.remove_like')
				: t('comments.liked_locked')
			: likeOffered || acting
				? t('comments.like')
				: t('comments.like_unavailable')
	);
	const dislikeLabel = $derived(
		disliked
			? dislikeOffered || acting
				? t('comments.remove_dislike')
				: t('comments.disliked_locked')
			: dislikeOffered || acting
				? t('comments.dislike')
				: t('comments.dislike_unavailable')
	);
	// Colour only: no transition on a row in a scrolling list (docs/UI-PERFORMANCE.md).
	const vote =
		'inline-flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground enabled:hover:text-foreground disabled:cursor-default disabled:opacity-50 aria-pressed:text-primary aria-pressed:disabled:opacity-100 focus-visible:outline-2 focus-visible:outline-ring';
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
			{#if canAct(comment)}
				<span class="-ml-1.5 flex items-center gap-0.5" role="group" aria-label={t('comments.vote')}>
					<button
						type="button"
						class={vote}
						aria-pressed={liked}
						aria-label={likeLabel}
						disabled={acting || !likeOffered}
						onclick={() => actOnComment(comment, likeAction)}
					>
						<HugeiconsIcon icon={ThumbsUpIcon} class="h-3.5 w-3.5 {liked ? 'fill-current' : ''}" />
					</button>
					{#if count}<span class="min-w-3 tabular-nums">{count}</span>{/if}
					<button
						type="button"
						class={vote}
						aria-pressed={disliked}
						aria-label={dislikeLabel}
						disabled={acting || !dislikeOffered}
						onclick={() => actOnComment(comment, dislikeAction)}
					>
						<HugeiconsIcon
							icon={ThumbsDownIcon}
							class="h-3.5 w-3.5 {disliked ? 'fill-current' : ''}"
						/>
					</button>
				</span>
			{:else if count}
				<span class="flex items-center gap-1">
					<HugeiconsIcon icon={ThumbsUpIcon} class="h-3.5 w-3.5" />
					{count}
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
