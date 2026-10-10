<script lang="ts">
	import { untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowDown01Icon, ArrowUp01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as Tabs from '$lib/components/ui/tabs';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import type { Comment } from '$lib/api';
	import {
		cancelDelete,
		comments,
		confirmDelete,
		loadComments,
		loadMoreComments,
		loadMoreReplies,
		reloadComments,
		setCommentSort,
		submitCreate,
		toggleReplies,
		type ThreadView
	} from '$lib/comments.svelte';
	import { auth, playback } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import CommentRow from './CommentRow.svelte';
	import CommentComposer from './CommentComposer.svelte';

	// Only mounted while a comments surface is on screen (the docked panel or the player view's
	// tab), so nothing is fetched for a track nobody opened comments on. Reopened on the same
	// track, `loadComments` is a no-op.
	const videoId = $derived(playback.now?.videoId);
	// Also re-runs on a sign-in, sign-out or account switch (`auth.epoch`): this panel sits outside
	// the page the layout remounts on it, so it has to notice by itself that what it holds belongs
	// to whoever was signed in before. `loadComments` starts over when the epoch differs.
	$effect(() => {
		const id = videoId;
		void auth.epoch;
		if (id) untrack(() => loadComments(id));
	});

	let scroller = $state<HTMLElement | null>(null);

	// The dialog closes on the click that confirms it, which clears the request in the state: keep
	// the comment it was opened for until it is sent.
	let toDelete = $state<Comment | null>(null);
	$effect(() => {
		if (comments.confirmDelete) toDelete = comments.confirmDelete;
	});

	// A sentinel at the foot loads the next page. Keyed on the thread count below: one that stays in
	// view after a short page would otherwise never fire again.
	function watch(node: HTMLElement) {
		const io = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) loadMoreComments();
			},
			{ root: scroller, rootMargin: '200px' }
		);
		io.observe(node);
		return () => io.disconnect();
	}

	const selectedSort = $derived(comments.header?.sorts.find((s) => s.selected)?.key ?? 'top');
	const count = $derived(comments.header?.count_text);

	/** "1" is the only singular count; the rest are YouTube's abbreviated strings ("2.4M"). */
	function repliesLabel(n: string | undefined): string {
		if (!n) return t('comments.show_replies');
		return n === '1' ? t('comments.replies_one') : t('comments.replies', { count: n });
	}
</script>

{#snippet skeletons()}
	{#each [0, 1, 2, 3] as i (i)}
		<div class="flex gap-3 px-4 py-2.5">
			<Skeleton class="size-8 shrink-0 rounded-full" />
			<div class="min-w-0 flex-1 space-y-2">
				<Skeleton class="h-3 w-1/3 rounded" />
				<Skeleton class="h-3 w-full rounded" />
				<Skeleton class="h-3 w-2/3 rounded" />
			</div>
		</div>
	{/each}
{/snippet}

{#snippet replies(v: ThreadView)}
	{#if v.replies.length || v.repliesToken}
		<div class="mt-1">
			<button
				class="flex cursor-pointer items-center gap-1 text-xs font-medium text-primary"
				onclick={() => toggleReplies(v)}
			>
				<!-- altIcon/showAlt, not a ternary: `icon` is frozen at mount -->
				<HugeiconsIcon
					icon={ArrowDown01Icon}
					altIcon={ArrowUp01Icon}
					showAlt={v.open}
					class="h-3.5 w-3.5"
				/>
				{v.open ? t('comments.hide_replies') : repliesLabel(v.comment.reply_count)}
			</button>
		</div>
		{#if v.open}
			<div class="-mx-4 mt-1 pl-6">
				{#each v.replies as r (r.id)}
					{#if !comments.hidden[r.id]}<CommentRow comment={r} reply />{/if}
				{/each}
				{#if v.loading}
					<div class="px-4 py-2"><Skeleton class="h-3 w-1/2 rounded" /></div>
				{:else if v.error}
					<div class="px-4 py-1">
						<Button variant="ghost" size="sm" onclick={() => loadMoreReplies(v)}>
							{t('comments.retry')}
						</Button>
					</div>
				{:else if v.repliesToken}
					<div class="px-4 py-1">
						<button
							class="cursor-pointer text-xs font-medium text-primary"
							onclick={() => loadMoreReplies(v)}
						>
							{t('comments.show_more_replies')}
						</button>
					</div>
				{/if}
			</div>
		{/if}
	{/if}
{/snippet}

<div class="flex min-h-0 flex-1 flex-col">
	{#if comments.header?.sorts.length}
		<div class="flex items-center justify-between gap-2 border-b px-4 py-2">
			<span class="min-w-0 truncate text-xs text-muted-foreground">
				{#if count}
					{count === '1' ? t('comments.count_one') : t('comments.count', { count })}
				{/if}
			</span>
			<Tabs.Root value={selectedSort} onValueChange={(v) => setCommentSort(v as 'top' | 'newest')}>
				<Tabs.List>
					<Tabs.Trigger value="top">{t('comments.sort_top')}</Tabs.Trigger>
					<Tabs.Trigger value="newest">{t('comments.sort_newest')}</Tabs.Trigger>
				</Tabs.List>
			</Tabs.Root>
		</div>
	{/if}

	<div class="min-h-0 flex-1 overflow-y-auto" bind:this={scroller}>
		<!-- The comment box: only when the response carried a way to post (signed in). -->
		{#if comments.header?.composer && (comments.status === 'ready' || comments.status === 'loading')}
			<div class="border-b px-4 py-3">
				<CommentComposer
					bind:value={comments.create.text}
					placeholder={comments.header.composer.placeholder ?? t('comments.add_placeholder')}
					label={t('comments.composer_label')}
					submitLabel={t('comments.post')}
					pending={comments.create.pending}
					uncertain={comments.create.uncertain}
					onsubmit={submitCreate}
					onreload={reloadComments}
				/>
			</div>
		{/if}
		{#if comments.status === 'idle' || comments.status === 'loading'}
			{@render skeletons()}
		{:else if comments.status === 'reload'}
			<div class="flex flex-col items-center gap-3 px-6 py-10 text-center">
				<p class="text-sm text-muted-foreground">{t('comments.reload_needed')}</p>
				<Button
					variant="secondary"
					size="sm"
					onclick={() => videoId && loadComments(videoId, true)}
				>
					{t('comments.reload')}
				</Button>
			</div>
		{:else if comments.status === 'error'}
			<div class="flex flex-col items-center gap-3 px-6 py-10 text-center">
				<p class="text-sm text-muted-foreground">{t('comments.load_failed')}</p>
				<Button
					variant="secondary"
					size="sm"
					onclick={() => videoId && loadComments(videoId, true)}
				>
					{t('comments.retry')}
				</Button>
			</div>
		{:else if comments.state === 'disabled'}
			<p class="px-6 py-10 text-center text-sm text-muted-foreground">{t('comments.disabled')}</p>
		{:else if comments.state === 'empty' || comments.threads.length === 0}
			<p class="px-6 py-10 text-center text-sm text-muted-foreground">{t('comments.empty')}</p>
		{:else}
			{#each comments.threads as v (v.comment.id)}
				{#if !comments.hidden[v.comment.id]}
					<CommentRow comment={v.comment}>
						{@render replies(v)}
					</CommentRow>
				{/if}
			{/each}
			{#if comments.loadingMore}
				{@render skeletons()}
			{:else if comments.moreError}
				<div class="flex justify-center px-4 py-3">
					<Button variant="ghost" size="sm" onclick={loadMoreComments}>
						{t('comments.load_more')}
					</Button>
				</div>
			{:else if comments.continuation}
				{#key comments.threads.length}
					<div class="h-px" {@attach watch}></div>
				{/key}
			{/if}
		{/if}
	</div>
</div>

<AlertDialog.Root
	open={!!comments.confirmDelete}
	onOpenChange={(open) => {
		if (!open) cancelDelete();
	}}
>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('comments.delete_title')}</AlertDialog.Title>
			<AlertDialog.Description>{t('comments.delete_desc')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action onclick={() => confirmDelete(toDelete)}>
				{t('common.delete')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
