// Local personalization: the Quick Picks grid, sidebar pins, and the play-recency / top-artist
// bookkeeping both of those need. Pure and rune-free on purpose — `personal.check.ts` runs it under
// plain node (`node --experimental-strip-types`). The reactive wrapper and persistence live in
// `player.svelte.ts`; nothing here touches storage, the network, or Svelte.
import type { BrowseItem } from './api';

/** Grid capacity. The grid holds only what the user puts in it — nothing is ever auto-added. */
export const MAX_PICKS = 18;
export const MAX_PINS = 3;
const MAX_RECENT = 100;
const MAX_ARTISTS = 100;

/** A Quick Picks tile — always something the user added by hand. */
export type Pick = BrowseItem & {
	/** Fixed at insertion — drives display order, so tiles never move while you listen. */
	addedAt: number;
	/** Bumped on every play/click — drives eviction only. */
	lastUsedAt: number;
};

export type RecentEntry = BrowseItem & { at: number };

export type Personal = {
	picks: Pick[];
	/** Pinned sidebar playlists, at most MAX_PINS; array order is display order. */
	pins: string[];
	/** Last time each playlist/album/artist was played from, keyed by browseId. */
	recent: Record<string, RecentEntry>;
	/** Play counts per artist, keyed by channel id (or name when there's no id). */
	artists: Record<string, { name: string; count: number }>;
};

export function empty(): Personal {
	return { picks: [], pins: [], recent: {}, artists: {} };
}

/** Tolerant parse of a persisted blob — a corrupt or older shape degrades to empty, never throws. */
export function hydrate(raw: unknown): Personal {
	const base = empty();
	if (!raw || typeof raw !== 'object') return base;
	const o = raw as Partial<Personal>;
	if (Array.isArray(o.picks)) {
		// `manual: false` marks a tile from the old auto-seeding build. Seeding is gone — the grid is
		// the user's alone now — so those are dropped instead of being inherited forever.
		base.picks = o.picks.filter(
			(p) => p && typeof p.id === 'string' && (p as { manual?: boolean }).manual !== false
		);
	}
	if (Array.isArray(o.pins)) {
		base.pins = o.pins.filter((p) => typeof p === 'string').slice(0, MAX_PINS);
	}
	if (o.recent && typeof o.recent === 'object') base.recent = o.recent;
	if (o.artists && typeof o.artists === 'object') base.artists = o.artists;
	return base;
}

// --- Quick Picks -------------------------------------------------------------------------------

/** Add. Returns false when it was already on the grid (its recency is refreshed instead). */
export function addPick(p: Personal, item: BrowseItem, now = Date.now()): boolean {
	const existing = p.picks.find((x) => x.id === item.id);
	if (existing) {
		existing.lastUsedAt = now;
		return false;
	}
	// Full: drop the tile the user has gone longest without playing or opening.
	if (p.picks.length >= MAX_PICKS) {
		const stalest = p.picks.reduce((a, b) => (b.lastUsedAt < a.lastUsedAt ? b : a));
		p.picks = p.picks.filter((x) => x !== stalest);
	}
	p.picks.push({ ...item, addedAt: now, lastUsedAt: now });
	return true;
}

export function removePick(p: Personal, id: string): void {
	p.picks = p.picks.filter((x) => x.id !== id);
}

/**
 * Mark a tile as used (played or clicked). Returns whether anything changed — most calls come from
 * cards that aren't on the grid, and the caller uses this to skip a pointless write.
 */
export function touchPick(p: Personal, id: string, now = Date.now()): boolean {
	const hit = p.picks.find((x) => x.id === id);
	if (!hit) return false;
	hit.lastUsedAt = now;
	return true;
}

/** Display order: the order they were added. Stable — independent of `lastUsedAt`. */
export function orderedPicks(p: Personal): Pick[] {
	return [...p.picks].sort((a, b) => a.addedAt - b.addedAt);
}

// --- Sidebar pins + ordering -------------------------------------------------------------------

export function togglePin(p: Personal, id: string): 'pinned' | 'unpinned' | 'full' {
	if (p.pins.includes(id)) {
		p.pins = p.pins.filter((x) => x !== id);
		return 'unpinned';
	}
	if (p.pins.length >= MAX_PINS) return 'full';
	p.pins.push(id);
	return 'pinned';
}

/**
 * Pinned first in pin order, then everything else by last played (ties and never-played items keep
 * the backend's order). Pinned ids are resolved through the live list and excluded from the tail,
 * so a playlist can never appear twice and a pin left over from a deleted playlist just vanishes.
 */
export function orderLibrary(items: BrowseItem[], p: Personal): BrowseItem[] {
	const byId = new Map(items.map((i) => [i.id, i]));
	const pinned = p.pins.map((id) => byId.get(id)).filter((i): i is BrowseItem => !!i);
	const pinnedIds = new Set(pinned.map((i) => i.id));
	const rest = items
		.map((item, index) => ({ item, index }))
		.filter(({ item }) => !pinnedIds.has(item.id))
		.sort(
			(a, b) =>
				(p.recent[b.item.id]?.at ?? 0) - (p.recent[a.item.id]?.at ?? 0) || a.index - b.index
		)
		.map(({ item }) => item);
	return [...pinned, ...rest];
}

// --- Recency + artist counts -------------------------------------------------------------------

/** Record that a playlist/album/artist was played from. */
export function noteRecent(p: Personal, item: BrowseItem, now = Date.now()): void {
	p.recent[item.id] = { ...item, at: now };
	const ids = Object.keys(p.recent);
	if (ids.length > MAX_RECENT) {
		// ponytail: newest-N window, not a history log. A real plays table is the upgrade if this
		// ever needs depth (counts, date ranges, a stats view).
		for (const id of ids.sort((a, b) => p.recent[b].at - p.recent[a].at).slice(MAX_RECENT)) {
			delete p.recent[id];
		}
	}
}

/** The most recently played-from playlists/albums/artists, newest first. */
export function recentItems(p: Personal, n = 12): RecentEntry[] {
	return Object.values(p.recent)
		.sort((a, b) => b.at - a.at)
		.slice(0, n);
}

/** The lead artist out of a joined credit string — a usable search seed, unlike the whole list. */
export function firstArtist(artists: string): string {
	return artists.split(/[,&•]|\sfeat\.?\s|\sft\.?\s/i)[0].trim();
}

export function noteArtist(p: Personal, key: string, name: string): void {
	const cur = p.artists[key];
	if (cur) {
		cur.count++;
		if (name) cur.name = name;
	} else {
		p.artists[key] = { name, count: 1 };
	}
	const keys = Object.keys(p.artists);
	if (keys.length > MAX_ARTISTS) {
		for (const k of keys
			.sort((a, b) => p.artists[b].count - p.artists[a].count)
			.slice(MAX_ARTISTS)) {
			delete p.artists[k];
		}
	}
}

/** The user's most-played artist names — the seed for the community shelf. */
export function topArtists(p: Personal, n = 3): string[] {
	return Object.values(p.artists)
		.filter((a) => a.name)
		.sort((a, b) => b.count - a.count)
		.slice(0, n)
		.map((a) => a.name);
}

/** Round-robin several result lists into one, deduped by id. */
export function interleave<T extends { id: string }>(lists: T[][], cap: number): T[] {
	const out: T[] = [];
	const seen = new Set<string>();
	const longest = lists.reduce((m, l) => Math.max(m, l.length), 0);
	for (let i = 0; i < longest && out.length < cap; i++) {
		for (const list of lists) {
			if (out.length >= cap) break;
			const item = list[i];
			if (item && !seen.has(item.id)) {
				seen.add(item.id);
				out.push(item);
			}
		}
	}
	return out;
}
