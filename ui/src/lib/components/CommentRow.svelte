<script lang="ts">
	import type { Snippet } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CheckmarkBadge01Icon,
		FavouriteIcon,
		ArrowTurnBackwardIcon,
		Delete02Icon,
		PencilEdit02Icon,
		Pin02Icon,
		ThumbsDownIcon,
		ThumbsUpIcon
	} from '@hugeicons/core-free-icons';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar';
	import type { Comment, CommentAction } from '$lib/api';
	import {
		actOnComment,
		cancelDraft,
		canAct,
		comments,
		isActing,
		openEdit,
		openReply,
		requestDelete,
		shownCount,
		submitDraft
	} from '$lib/comments.svelte';
	import CommentComposer from './CommentComposer.svelte';
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

	// Like and dislike buttons, only for a comment the response offered an action on (never signed
	// out). A lit button's action undoes its vote, an unlit one's casts it; one not on offer is
	// disabled.
	const count = $derived(shownCount(comment));
	const acting = $derived(isActing(comment));
	const liked = $derived(comment.vote === 'liked');
	const disliked = $derived(comment.vote === 'disliked');
	const likeAction = $derived<CommentAction>(liked ? 'unlike' : 'like');
	const dislikeAction = $derived<CommentAction>(disliked ? 'undislike' : 'dislike');
	const likeOffered = $derived(comment.actions.includes(likeAction));
	const dislikeOffered = $derived(comment.actions.includes(dislikeAction));
	// The inline composer (a reply under this comment, or an edit in place of its text) belongs to
	// this row when the draft names it.
	const draft = $derived(comments.draft);
	const editing = $derived(draft.target?.kind === 'edit' && draft.target.id === comment.id);
	const replying = $derived(draft.target?.kind === 'reply' && draft.target.id === comment.id);
	const canReply = $derived(comment.writes.includes('reply'));
	const canEdit = $derived(comment.writes.includes('edit'));
	const canDelete = $derived(comment.writes.includes('delete'));
	const writeLink =
		'inline-flex cursor-pointer items-center gap-1 text-xs text-muted-foreground enabled:hover:text-foreground disabled:cursor-default disabled:opacity-50';
	// Colour only: no transition on a row in a scrolling list (docs/UI-PERFORMANCE.md).
	const vote =
		'inline-flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground enabled:hover:text-foreground disabled:cursor-default disabled:opacity-50 aria-pressed:text-primary aria-pressed:disabled:opacity-100 focus-visible:outline-2 focus-visible:outline-ring';
</script>

{#snippet composer()}
	<CommentComposer
		bind:value={comments.draft.text}
		placeholder={editing
			? t('comments.edit_label')
			: (comment.reply_placeholder ?? t('comments.reply_placeholder'))}
		label={editing ? t('comments.edit_label') : t('comments.reply_placeholder')}
		submitLabel={editing ? t('comments.save') : t('comments.post_reply')}
		pending={draft.pending}
		focus
		onsubmit={submitDraft}
		oncancel={cancelDraft}
	/>
{/snippet}

<div class="flex gap-3 px-4 py-2.5">
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
		{#if editing}
			<div class="mt-1">{@render composer()}</div>
		{:else}
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
		{/if}
		<div class="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
			{#if canAct(comment)}
				<span class="-ml-1.5 flex items-center gap-0.5" role="group" aria-label={t('comments.vote')}>
					<button
						type="button"
						class={vote}
						aria-pressed={liked}
						aria-label={liked ? t('comments.remove_like') : t('comments.like')}
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
						aria-label={disliked ? t('comments.remove_dislike') : t('comments.dislike')}
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
			{#if canReply}
				<button
					type="button"
					class={writeLink}
					disabled={draft.pending}
					onclick={() => openReply(comment)}
				>
					<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-3.5 w-3.5" />
					{t('comments.reply')}
				</button>
			{/if}
			{#if canEdit}
				<button
					type="button"
					class={vote}
					aria-label={t('comments.edit')}
					title={t('comments.edit')}
					disabled={draft.pending}
					onclick={() => openEdit(comment)}
				>
					<HugeiconsIcon icon={PencilEdit02Icon} class="h-3.5 w-3.5" />
				</button>
			{/if}
			{#if canDelete}
				<button
					type="button"
					class={vote}
					aria-label={t('comments.delete')}
					title={t('comments.delete')}
					onclick={() => requestDelete(comment)}
				>
					<HugeiconsIcon icon={Delete02Icon} class="h-3.5 w-3.5" />
				</button>
			{/if}
		</div>
		{#if replying}
			<div class="mt-2">{@render composer()}</div>
		{/if}
		{@render children?.()}
	</div>
</div>
