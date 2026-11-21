<script lang="ts">
	// What reviewers decided about my lyrics: rejects, takedowns, and approvals that fixed the lines
	// or came with a word. Never who. Opening the tab reads them; their dots stay for this visit.
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import { limusic, notesRead } from '$lib/limusic.svelte';
	import { ago, coverOf } from '$lib/review';
	import { currentLocale, t, type TranslationKey } from '$lib/i18n.svelte';

	let { group, label }: { group: string; label: string } = $props();

	let notes = $state.raw<api.LimusicNote[]>([]);
	const now = Math.floor(Date.now() / 1000);

	/** Which session the list belongs to: an answer for an earlier one is dropped. */
	let asked = 0;
	async function load(mine: number) {
		let found: api.LimusicNote[];
		try {
			found = await api.limusicNotes();
		} catch {
			// An older service, or unreachable: no section.
			return;
		}
		if (mine !== asked) return;
		notes = found;
		// Newest first, so the first one is as far as there is to read.
		if (found[0]?.unread) {
			api.limusicNotesSeen(found[0].id)
				.then(() => mine === asked && notesRead())
				.catch(() => {});
		}
	}
	// Again for another account: signing out and into another one with the tab open.
	$effect(() => {
		const signedIn = limusic.signedIn;
		void limusic.handle;
		untrack(() => {
			const mine = ++asked;
			notes = [];
			if (signedIn) load(mine);
		});
	});

	function badge(n: api.LimusicNote): { key: TranslationKey; cls: string } {
		if (n.verdict === 'reject') return { key: 'limusic.note_rejected', cls: 'bg-destructive/15 text-destructive' };
		if (n.verdict === 'takedown') return { key: 'limusic.note_taken_down', cls: 'bg-destructive/15 text-destructive' };
		return { key: n.fixed ? 'limusic.note_fixed' : 'limusic.note_approved', cls: 'bg-primary/12 text-primary' };
	}
	const body = (n: api.LimusicNote) =>
		n.reason ? t('limusic.note_quote', { reason: n.reason }) : n.verdict === 'approve' ? t('limusic.note_fixed_silent') : t('limusic.note_no_reason');
</script>

{#if notes.length}
	<section class={group}>
		<h3 class={label}>{t('limusic.notes_title')}</h3>
		<p class="-mt-0.5 mb-2.5 px-1 text-[13px] text-muted-foreground">{t('limusic.notes_hint')}</p>
		<div data-settings-card class="divide-y divide-border/60 overflow-hidden rounded-xl border bg-card">
			{#each notes as n (n.id)}
				{@const b = badge(n)}
				{@const cover = coverOf(n.video_id)}
				<div class="flex items-start gap-3 px-4 py-3">
					<span class="mt-[18px] size-2 shrink-0 rounded-full {n.unread ? 'bg-primary' : ''}"></span>
					{#if cover}
						<img src={cover} alt="" class="size-10 shrink-0 rounded-md object-cover" draggable="false" loading="lazy" />
					{/if}
					<div class="min-w-0 flex-1">
						<div class="flex flex-wrap items-center gap-x-2 gap-y-1">
							<span class="truncate text-sm font-semibold">{n.title}</span>
							<span class="truncate text-[13px] text-muted-foreground">{n.artist}</span>
							<span class="rounded-full px-2 py-px text-[11px] font-semibold {b.cls}">{t(b.key)}</span>
							<span class="ml-auto text-xs text-muted-foreground">{ago(n.at, now, currentLocale.id)}</span>
						</div>
						<p class="mt-1 text-[13px] leading-relaxed {n.reason ? '' : 'text-muted-foreground'}">{body(n)}</p>
					</div>
				</div>
			{/each}
		</div>
	</section>
{/if}
