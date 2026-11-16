<script lang="ts">
	// The lyrics team, for admins: who reviews Limusic Lyrics, who could, and bans. Staff first, then
	// the contributors with points, best record first, so a reviewer is picked from their work. A
	// change shows at once and goes back if the service refuses it.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowLeft01Icon, LockIcon, MoreHorizontalIcon, Search01Icon, UserIcon } from '@hugeicons/core-free-icons';
	import { Button, buttonVariants } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as Popover from '$lib/components/ui/popover';
	import * as Select from '$lib/components/ui/select';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import { toast } from '$lib/player.svelte';
	import { limusic, refreshAccount } from '$lib/limusic.svelte';
	import { currentLocale, t, type TranslationKey } from '$lib/i18n.svelte';

	type Member = api.LimusicMember;
	type Role = Member['role'];

	let allowed = $state(true);
	let loading = $state(true);
	let loadError = $state('');
	let members = $state.raw<Member[]>([]);
	let q = $state('');
	/** Which search the list belongs to: an answer to an older one is dropped. */
	let asked = 0;

	const errorKey = (code: string): TranslationKey =>
		code === 'forbidden' ? 'team.err_forbidden' : code === 'own' ? 'team.err_own' : code === 'not_found' ? 'team.err_gone' : 'review.err_unavailable';

	async function load() {
		const mine = ++asked;
		loadError = '';
		try {
			const found = await api.limusicAdminAccounts(q);
			if (mine === asked) members = found;
		} catch (e) {
			if (mine !== asked) return;
			if (String(e) === 'forbidden') allowed = false;
			else loadError = t(errorKey(String(e)));
		} finally {
			if (mine === asked) loading = false;
		}
	}

	async function start() {
		loading = true;
		loadError = '';
		if (!limusic.account) await refreshAccount();
		if (!limusic.account && limusic.signedIn) {
			// The service couldn't say who this is, which is not the same as not being an admin.
			loadError = t('review.err_unavailable');
			loading = false;
			return;
		}
		if (limusic.account?.role !== 'admin' || limusic.anonymous) {
			allowed = false;
			loading = false;
			return;
		}
		await load();
	}
	onMount(start);

	let searchTimer: ReturnType<typeof setTimeout> | undefined;
	function search(value: string) {
		q = value;
		clearTimeout(searchTimer);
		searchTimer = setTimeout(load, 250);
	}

	/** Optimistic: the row changes now, and comes back as it was if the service says no. */
	async function change(m: Member, to: { role?: Role; banned?: boolean }) {
		// Only this row goes back on a refusal: others may have changed, or a search replaced the list.
		const before = members.find((x) => x.handle === m.handle) ?? m;
		members = members.map((x) => (x.handle === m.handle ? { ...x, ...to } : x));
		try {
			const now = await api.limusicAdminUpdate(m.handle, to);
			members = members.map((x) => (x.handle === m.handle ? now : x));
			if (to.role) toast.success(t('team.role_set', { handle: m.handle, role: roleLabel(to.role) }));
			else toast.success(t(to.banned ? 'team.banned_toast' : 'team.unbanned_toast', { handle: m.handle }));
		} catch (e) {
			members = members.map((x) => (x.handle === m.handle ? before : x));
			toast.error(t(errorKey(String(e))));
		}
	}

	let menuFor = $state<string | null>(null);
	let banning = $state<Member | null>(null);

	const ROLES: Role[] = ['user', 'reviewer', 'admin'];
	const roleLabel = (r: Role) => t(r === 'admin' ? 'team.role_admin' : r === 'reviewer' ? 'team.role_reviewer' : 'team.role_user');
	const staff = $derived(q.trim() ? [] : members.filter((m) => m.role !== 'user'));
	const others = $derived(q.trim() ? members : members.filter((m) => m.role === 'user'));
	const joined = (secs: number) =>
		new Intl.DateTimeFormat(currentLocale.id, { month: 'short', year: 'numeric' }).format(new Date(secs * 1000));
	const LABEL = 'mb-2 ml-1 text-[11.5px] font-bold tracking-[0.07em] text-muted-foreground uppercase';
</script>

{#snippet avatar(m: Member, size: string)}
	<span class="flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted text-[13px] font-bold uppercase" style="width:{size};height:{size}">
		{#if m.avatar_url}
			<img src={m.avatar_url} alt="" class="size-full object-cover" draggable="false" loading="lazy" />
		{:else}
			{m.handle[0]}
		{/if}
	</span>
{/snippet}

{#snippet menu(m: Member)}
	<Popover.Root open={menuFor === m.handle} onOpenChange={(o) => (menuFor = o ? m.handle : null)}>
		<Popover.Trigger class={buttonVariants({ variant: 'ghost', size: 'icon' })} aria-label={t('team.more', { handle: m.handle })}>
			<HugeiconsIcon icon={MoreHorizontalIcon} class="size-[18px]" />
		</Popover.Trigger>
		<Popover.Content align="end" class="w-60 gap-0 p-1.5">
			<button
				type="button"
				class="flex w-full cursor-pointer items-center rounded-lg px-2.5 py-1.5 text-left text-sm hover:bg-muted"
				onclick={() => {
					menuFor = null;
					api.openExternal(`https://limusic.app/lyrics/?u=${encodeURIComponent(m.handle)}`);
				}}
			>
				{t('team.open_profile')}
			</button>
			<button
				type="button"
				class="flex w-full cursor-pointer items-center rounded-lg px-2.5 py-1.5 text-left text-sm hover:bg-muted {m.banned ? '' : 'text-destructive'}"
				onclick={() => {
					menuFor = null;
					if (m.banned) change(m, { banned: false });
					else banning = m;
				}}
			>
				{m.banned ? t('team.unban') : t('team.ban')}
			</button>
		</Popover.Content>
	</Popover.Root>
{/snippet}

{#snippet row(m: Member, picker: boolean)}
	<div class="flex flex-wrap items-center gap-x-3.5 gap-y-2 px-4 py-3">
		{@render avatar(m, '2.25rem')}
		<div class="min-w-40 flex-1">
			<p class="flex items-center gap-2 truncate text-[14.5px] font-semibold">
				{m.handle}
				{#if m.you}<span class="text-[12.5px] font-medium text-muted-foreground">{t('team.you')}</span>{/if}
				{#if m.banned}
					<span class="rounded-full bg-destructive/15 px-2 py-px text-[11px] font-semibold text-destructive">{t('team.banned')}</span>
				{/if}
			</p>
			<p class="truncate text-[12.5px] text-muted-foreground">
				{t('team.stats', { score: m.score, approved: m.approved, rejected: m.rejected })} · {t('team.joined', { when: joined(m.joined_at) })}
			</p>
		</div>
		{#if m.you}
			<span class="px-3 text-[13px] font-semibold text-muted-foreground">{roleLabel(m.role)}</span>
		{:else}
			{#if picker}
				<Select.Root type="single" value={m.role} onValueChange={(v) => v !== m.role && change(m, { role: v as Role })}>
					<Select.Trigger class="h-9 w-32" aria-label={t('team.role_for', { handle: m.handle })}>
						<span class="flex-1 truncate text-left">{roleLabel(m.role)}</span>
					</Select.Trigger>
					<Select.Content>
						{#each ROLES as r (r)}
							<Select.Item value={r} label={roleLabel(r)}>{roleLabel(r)}</Select.Item>
						{/each}
					</Select.Content>
				</Select.Root>
			{:else}
				<Button variant="outline" size="sm" disabled={m.banned} onclick={() => change(m, { role: 'reviewer' })}>{t('team.make_reviewer')}</Button>
			{/if}
			{@render menu(m)}
		{/if}
	</div>
{/snippet}

<div class="flex h-full flex-col">
	<header class="flex shrink-0 items-center gap-3.5 px-6 pt-5 pb-4">
		<Button variant="ghost" size="icon" class="-ml-2" aria-label={t('common.back')} title={t('common.back')} onclick={() => history.back()}>
			<HugeiconsIcon icon={ArrowLeft01Icon} class="size-[18px]" />
		</Button>
		<div class="min-w-0 flex-1">
			<h1 class="truncate font-heading text-2xl leading-tight font-bold tracking-tight">{t('team.title')}</h1>
			<p class="truncate text-sm text-muted-foreground">{t('team.subtitle')}</p>
		</div>
	</header>

	{#if !allowed}
		<div class="flex flex-1 flex-col items-center justify-center gap-1.5 border-t px-6 text-center">
			<span class="mb-1 flex size-12 items-center justify-center rounded-[13px] bg-muted"><HugeiconsIcon icon={LockIcon} class="size-[22px]" /></span>
			<p class="font-heading text-base font-semibold">{t('team.not_allowed_title')}</p>
			<p class="text-[13px] text-muted-foreground">{t('team.not_allowed')}</p>
		</div>
	{:else}
		<div class="min-h-0 flex-1 overflow-y-auto border-t">
			<div class="mx-auto flex max-w-[980px] flex-col gap-6 px-6 pt-5 pb-10">
				<label class="flex h-11 items-center gap-2.5 rounded-xl border bg-card px-3.5 focus-within:border-primary">
					<HugeiconsIcon icon={Search01Icon} class="size-[17px] shrink-0 text-muted-foreground" />
					<span class="sr-only">{t('team.search')}</span>
					<input
						value={q}
						oninput={(e) => search(e.currentTarget.value)}
						maxlength="40"
						placeholder={t('team.search')}
						class="min-w-0 flex-1 bg-transparent text-sm outline-none"
					/>
				</label>

				{#if loading}
					<div class="overflow-hidden rounded-xl border bg-card">
						{#each Array(4) as _, i (i)}
							<div class="flex items-center gap-3.5 px-4 py-3">
								<Skeleton class="size-9 rounded-full" />
								<div class="flex-1 space-y-2"><Skeleton class="h-4 w-1/4 rounded" /><Skeleton class="h-3 w-2/5 rounded" /></div>
							</div>
						{/each}
					</div>
				{:else if loadError}
					<ErrorState message={loadError} onRetry={start} />
				{:else}
					{#if staff.length}
						<section>
							<h3 class={LABEL}>{t('team.staff')}</h3>
							<div class="divide-y divide-border/60 overflow-hidden rounded-xl border bg-card">
								{#each staff as m (m.handle)}{@render row(m, true)}{/each}
							</div>
						</section>
					{/if}
					<section>
						<h3 class={LABEL}>{q.trim() ? t('team.results') : t('team.top')}</h3>
						{#if !q.trim()}
							<p class="-mt-0.5 mb-2.5 ml-1 text-[13px] text-muted-foreground">{t('team.top_hint')}</p>
						{/if}
						{#if others.length}
							<div class="divide-y divide-border/60 overflow-hidden rounded-xl border bg-card">
								{#each others as m (m.handle)}{@render row(m, m.role !== 'user')}{/each}
							</div>
						{:else}
							<div class="flex items-center gap-3 rounded-xl border bg-card px-4 py-4 text-[13px] text-muted-foreground">
								<HugeiconsIcon icon={UserIcon} class="size-4" />
								{q.trim() ? t('team.no_results') : t('team.no_contributors')}
							</div>
						{/if}
					</section>
				{/if}
			</div>
		</div>
	{/if}
</div>

<AlertDialog.Root open={banning !== null} onOpenChange={(o) => !o && (banning = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('team.ban_title', { handle: banning?.handle ?? '' })}</AlertDialog.Title>
			<AlertDialog.Description>{t('team.ban_body')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action
				class={buttonVariants({ variant: 'destructive' })}
				onclick={() => {
					const m = banning;
					banning = null;
					if (m) change(m, { banned: true });
				}}
			>
				{t('team.ban')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
