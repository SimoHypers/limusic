<script lang="ts">
    import { page } from "$app/state";
    import { goto } from "$app/navigation";
    import { HugeiconsIcon } from "@hugeicons/svelte";
    import {
        PlayIcon,
        MoreVerticalIcon,
        ShuffleIcon,
        PlayListAddIcon,
        DashboardSquare02Icon,
    } from "@hugeicons/core-free-icons";
    import TrackRow from "$lib/components/TrackRow.svelte";
    import TrackRowSkeleton from "$lib/components/TrackRowSkeleton.svelte";
    import ErrorState from "$lib/components/ErrorState.svelte";
    import { Skeleton } from "$lib/components/ui/skeleton";
    import * as api from "$lib/api";
    import type { AlbumPage, BrowseItem } from "$lib/api";
    import {
        addPick,
        playback,
        openAddManyToPlaylist,
        playFrom,
    } from "$lib/player.svelte";
    import { getCached, putCached } from "$lib/pagecache";

    let album = $state<AlbumPage | null>(null);
    let artistHero = $state<string | null>(null);
    let loading = $state(true);
    let error = $state<string | null>(null);
    let expanded = $state(false);
    let menuOpen = $state(false);

    const id = $derived(page.params.id ?? "");
    const nowId = $derived(playback.now?.videoId);

    async function load(aid: string) {
        const key = `album:${aid}`;
        const hit = getCached<AlbumPage>(key);
        artistHero = null;
        if (hit) {
            album = hit;
            loadHero(aid, hit);
            loading = false;
        } else {
            loading = true;
            album = null;
        }
        error = null;
        expanded = false;
        try {
            const fresh = await api.getAlbum(aid);
            if (aid !== id) return; // superseded by navigation — drop the stale response
            album = fresh;
            putCached(key, fresh);
            loadHero(aid, fresh);
        } catch (e) {
            if (aid !== id) return;
            if (!hit) error = String(e);
        } finally {
            if (aid === id) loading = false;
        }
    }

    // The album's artist image becomes the hero backdrop (like the artist page). Non-blocking —
    // the page already shows; the backdrop fades in when it arrives. Guarded against navigation.
    // ponytail: reuses the full artist browse just for its hero image; `album.artistThumbnail`
    // already carries a straplineThumbnail — swap to it to drop this second fetch if it ever matters.
    function loadHero(aid: string, a: AlbumPage) {
        if (!a.artistId) return;
        api.getArtist(a.artistId)
            .then((art) => {
                if (aid === id) artistHero = art.thumbnail ?? null;
            })
            .catch(() => {});
    }

    $effect(() => {
        if (id) load(id);
    });

    // This album as a card, for the sidebar's last-played sort and the Quick Picks grid.
    const asItem = (): BrowseItem => ({
        kind: "album",
        id,
        title: album?.title ?? "Album",
        subtitle: album?.artist,
        thumbnail: album?.thumbnail,
    });

    function playAll(start: number | null) {
        if (album) playFrom(asItem(), album.items, start);
    }
    function shuffle() {
        if (!album?.items.length) return;
        menuOpen = false;
        // ponytail: shuffles the album's own tracks (a finite album is small); no radio seed.
        const order = [...album.items].sort(() => Math.random() - 0.5);
        playFrom(asItem(), order, 0);
    }
    function saveToPlaylist() {
        if (!album?.items.length) return;
        menuOpen = false;
        openAddManyToPlaylist(album.items.map((t) => t.video_id));
    }
</script>

{#if loading}
    <div class="flex flex-col gap-5 p-6 pt-10">
        <div class="flex items-end gap-5">
            <Skeleton class="h-28 w-28 shrink-0 rounded-xl" />
            <div class="flex-1 space-y-3">
                <Skeleton class="h-3 w-16 rounded" />
                <Skeleton class="h-10 w-1/2 rounded-lg" />
                <Skeleton class="h-4 w-40 rounded" />
            </div>
        </div>
        <div class="flex gap-3">
            <Skeleton class="h-11 w-28 rounded-full" />
            <Skeleton class="h-11 w-28 rounded-full" />
        </div>
    </div>
    <div class="p-6 pt-2">
        {#each Array(8) as _, i (i)}
            <TrackRowSkeleton hideThumb />
        {/each}
    </div>
{:else if error}
    <div class="p-6"><ErrorState message={error} onRetry={() => load(id)} /></div>
{:else if album}
    <!-- Header with the artist image as a hero backdrop -->
    <div class="relative overflow-hidden">
        {#if artistHero}
            <img
                src={artistHero}
                alt=""
                class="absolute inset-0 h-full w-full object-cover object-top"
            />
        {:else if album.thumbnail}
            <img
                src={album.thumbnail}
                alt=""
                class="absolute inset-0 h-full w-full scale-110 object-cover opacity-50 blur-2xl"
            />
        {/if}
        <div
            class="absolute inset-0 bg-gradient-to-t from-background via-background/75 to-background/40"
        ></div>

        <div class="relative flex flex-col gap-5 p-6 pt-10">
            <div class="flex items-end gap-5">
                <!-- Inline width/height so the size holds even against a stale dev-server CSS that -->
                <!-- hasn't regenerated a newly-used spacing utility (would fall back to intrinsic size). -->
                {#if album.thumbnail}
                    <img
                        src={album.thumbnail}
                        alt=""
                        style="width:7rem;height:7rem"
                        class="shrink-0 rounded-xl object-cover shadow-2xl"
                    />
                {:else}
                    <div
                        style="width:7rem;height:7rem"
                        class="shrink-0 rounded-xl bg-muted"
                    ></div>
                {/if}
                <div class="min-w-0">
                    <div
                        class="text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                    >
                        {album.subtitle ?? "Album"}
                    </div>
                    <h1
                        class="mt-1 font-heading text-4xl font-bold tracking-tight drop-shadow"
                    >
                        {album.title ?? "Album"}
                    </h1>
                    <div
                        class="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground"
                    >
                        {#if album.artist}
                            <button
                                class="flex items-center gap-1.5 font-medium text-foreground hover:underline disabled:cursor-default disabled:no-underline"
                                class:cursor-pointer={!!album.artistId}
                                onclick={() =>
                                    album!.artistId &&
                                    goto(
                                        `/artist/${encodeURIComponent(album!.artistId)}`,
                                    )}
                                disabled={!album.artistId}
                            >
                                {#if album.artistThumbnail}
                                    <img
                                        src={album.artistThumbnail}
                                        alt=""
                                        class="h-5 w-5 rounded-full object-cover"
                                    />
                                {/if}
                                {album.artist}
                            </button>
                        {/if}
                        {#if album.secondSubtitle}
                            <span class="text-muted-foreground/60">•</span>
                            <span>{album.secondSubtitle}</span>
                        {/if}
                    </div>
                </div>
            </div>

            {#if album.description}
                <div class="max-w-2xl">
                    <p
                        class="text-sm text-foreground/80 {expanded
                            ? ''
                            : 'line-clamp-2'}"
                    >
                        {album.description}
                    </p>
                    <button
                        class="mt-1 cursor-pointer text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
                        onclick={() => (expanded = !expanded)}
                    >
                        {expanded ? "Less" : "More"}
                    </button>
                </div>
            {/if}

            <!-- Controls -->
            <div class="relative flex items-center gap-3">
                <button
                    class="flex cursor-pointer items-center gap-2 rounded-full text-foreground bg-primary px-6 py-2.5 text-sm font-semibold transition hover:opacity-90 disabled:opacity-50"
                    onclick={() => playAll(null)}
                    disabled={!album.items.length}
                >
                    <HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> Play
                </button>
                <button
                    class="flex cursor-pointer items-center gap-2 rounded-full border px-5 py-2.5 text-sm font-semibold transition hover:bg-accent/10 disabled:opacity-50"
                    onclick={shuffle}
                    disabled={!album.items.length}
                >
                    <HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> Shuffle
                </button>
                <button
                    class="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full border text-muted-foreground transition hover:bg-accent/10 hover:text-foreground"
                    onclick={() => (menuOpen = !menuOpen)}
                    aria-label="More options"
                >
                    <HugeiconsIcon icon={MoreVerticalIcon} class="h-5 w-5" />
                </button>

                {#if menuOpen}
                    <button
                        class="fixed inset-0 z-40 cursor-default"
                        onclick={() => (menuOpen = false)}
                        aria-label="Close menu"
                    ></button>
                    <div
                        class="absolute bottom-12 left-40 z-50 min-w-48 origin-bottom-left animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
                    >
                        <button
                            class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
                            onclick={saveToPlaylist}
                        >
                            <HugeiconsIcon
                                icon={PlayListAddIcon}
                                class="h-4 w-4"
                            /> Save to playlist
                        </button>
                        <button
                            class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
                            onclick={() => {
                                menuOpen = false;
                                addPick(asItem());
                            }}
                        >
                            <HugeiconsIcon
                                icon={DashboardSquare02Icon}
                                class="h-4 w-4"
                            /> Add to Quick Picks
                        </button>
                    </div>
                {/if}
            </div>
        </div>
    </div>

    <!-- Numbered track list -->
    <div class="p-6 pt-2">
        {#each album.items as item, i (item.video_id + i)}
            <TrackRow
                song={item}
                index={i}
                hideThumb
                active={item.video_id === nowId}
                onplay={() => playAll(i)}
                onAdd={() => openAddManyToPlaylist([item.video_id])}
            />
        {:else}
            <p class="p-4 text-sm text-muted-foreground">
                This album is empty.
            </p>
        {/each}
    </div>
{/if}
