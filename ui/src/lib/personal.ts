// Local personalization: the home Shortcuts grid, sidebar pins, and the play-recency / top-artist
// bookkeeping both of those need. Pure and rune-free on purpose — `personal.check.ts` runs it under
// plain node (`node --experimental-strip-types`). The reactive wrapper and persistence live in
// `player.svelte.ts`; nothing here touches storage, the network, or Svelte.
import type { BrowseItem } from './api';

/**
 * Grid capacity. Everything on the grid was put there by hand, with one exception: `seedPick` lets
 * the app suggest a tile (today only On Repeat, once it has enough songs to be worth one). A seeded
 * tile is never forced: it can't evict a hand-picked one, and removing it is permanent.
 */
export const MAX_PICKS = 18;
export const MAX_PINS = 3;
const MAX_RECENT = 100;
const MAX_ARTISTS = 100;

/**
  * A Shortcuts tile — always something the user added by hand. Array order in `picks` IS display
  * order: the user arranges the grid by dragging, so nothing else may reshuffle it.
  */
export type Pick = BrowseItem & {
	/** Bumped on every play/click — drives eviction only, never order. */
	lastUsedAt: number;
};

export type RecentEntry = BrowseItem & { at: number };

/**
 * A row in the local library. `synced` means the Library page's sync button has since put it on the
 * signed-in account too; the row stays here either way, because an account can be signed out of and
 * this machine's library is not the account's.
 */
export type Saved = BrowseItem & { synced?: boolean };

export type Personal = {
	picks: Pick[];
	/**
	 * Playlists, albums and artists saved to the library from this machine, newest first. This is
	 * the whole library for a signed-out user, and it sits alongside YouTube's own for a signed-in
	 * one: nothing here is account-scoped, so saves made before signing in stay put afterwards.
	 */
	saved: Saved[];
	/** Pinned sidebar playlists, at most MAX_PINS; array order is display order. */
	pins: string[];
	/** Last time each playlist/album/artist was played from, keyed by browseId. */
	recent: Record<string, RecentEntry>;
	/** Play counts per artist, keyed by channel id (or name when there's no id). */
	artists: Record<string, { name: string; count: number }>;
	/**
	 * Tiles the user has taken off the grid. Only `seedPick` consults it, so this is purely "don't
	 * suggest this again": a hand-added tile is unaffected, since `addPick` ignores the list.
	 */
	dismissedSeeds: string[];
	home: HomeLayout;
};

export type CardSize = 'small' | 'medium' | 'large';
export const CARD_SIZES: CardSize[] = ['small', 'medium', 'large'];

/** Everything Edit home sets. */
export type HomeLayout = {
	/**
	 * Section keys, see `arrangeSections`. `seen` is every shelf home has ever shown, so Edit home
	 * can list the ones the feed hasn't reached yet. It is the feed's, not the user's: Reset keeps it.
	 */
	order: string[];
	hidden: string[];
	seen: string[];
	/**
	 * A section missing from `order` arrived after the last arrangement. On, it lands hidden instead
	 * of shown: hiding every shelf used to be undone by the next page of the feed (#317).
	 */
	hideNew: boolean;
	/** The mood chip row. */
	chips: boolean;
	/** The playing track's artwork behind the greeting; off, the header keeps the accent wash. */
	backdrop: boolean;
	/** Width of the cards on home's shelves. Rows of songs don't change. */
	cards: CardSize;
};

const defaultHome = (): HomeLayout => ({
	order: [],
	hidden: [],
	seen: [],
	hideNew: false,
	chips: true,
	backdrop: true,
	cards: 'medium'
});

export function empty(): Personal {
	return {
		picks: [],
		saved: [],
		pins: [],
		recent: {},
		artists: {},
		dismissedSeeds: [],
		home: defaultHome()
	};
}

/** Tolerant parse of a persisted blob — a corrupt or older shape degrades to empty, never throws. */
export function hydrate(raw: unknown): Personal {
	const base = empty();
	if (!raw || typeof raw !== 'object') return base;
	const o = raw as Partial<Personal>;
	if (Array.isArray(o.picks)) {
		// `manual: false` marks a tile from the old auto-seeding build, which filled the grid on the
		// user's behalf. Those are dropped rather than inherited forever. (Today's `seedPick` is a
		// narrower thing: one suggestion, refusable, and it writes no `manual` flag.)
		base.picks = o.picks.filter(
			(p) => p && typeof p.id === 'string' && (p as { manual?: boolean }).manual !== false
		);
	}
	if (Array.isArray(o.saved)) {
		base.saved = o.saved.filter((s) => s && typeof s.id === 'string' && typeof s.kind === 'string');
	}
	if (Array.isArray(o.pins)) {
		base.pins = o.pins.filter((p) => typeof p === 'string').slice(0, MAX_PINS);
	}
	if (o.recent && typeof o.recent === 'object') base.recent = o.recent;
	if (o.artists && typeof o.artists === 'object') base.artists = o.artists;
	if (Array.isArray(o.dismissedSeeds)) {
		base.dismissedSeeds = o.dismissedSeeds.filter((id) => typeof id === 'string');
	}
	if (o.home && typeof o.home === 'object') {
		const h = o.home as Partial<HomeLayout>;
		const keys = (v: unknown) =>
			Array.isArray(v) ? v.filter((k): k is string => typeof k === 'string') : [];
		const flag = (v: unknown, fallback: boolean) => (typeof v === 'boolean' ? v : fallback);
		base.home = {
			order: keys(h.order),
			hidden: keys(h.hidden),
			seen: keys(h.seen),
			hideNew: flag(h.hideNew, false),
			chips: flag(h.chips, true),
			backdrop: flag(h.backdrop, true),
			cards: CARD_SIZES.includes(h.cards as CardSize) ? (h.cards as CardSize) : 'medium'
		};
		// '@familiar' shipped after some users had already saved an arrangement, and an unranked key
		// sorts to the bottom of the feed. Slot it where the code puts it, once.
		const order = base.home.order;
		if (order.length && !order.includes('@familiar')) {
			const at = order.indexOf('@recent');
			order.splice(at < 0 ? order.length : at + 1, 0, '@familiar');
		}
		// Shortcuts sat above the arrangement until it became a section of its own; it stays on top.
		if (order.length && !order.includes('@shortcuts')) order.unshift('@shortcuts');
	}
	return base;
}

// --- Shortcuts grid -------------------------------------------------------------------------------

/** Append. Returns false when it was already on the grid (its recency is refreshed instead). */
export function addPick(p: Personal, item: BrowseItem, now = Date.now()): boolean {
	const existing = p.picks.find((x) => x.id === item.id);
	if (existing) {
		existing.lastUsedAt = now;
		return false;
	}
	p.picks.push({ ...item, lastUsedAt: now });
	evictStalest(p);
	return true;
}

/**
 * Where a drag lands: put `item` immediately before `beforeId`, or at the end when it's null.
 * Handles both jobs the grid needs — moving a tile already on it, and taking one dropped in from a
 * shelf. Addressing the drop by neighbour rather than by index keeps a rightward move off by one.
 */
export function placePick(
	p: Personal,
	item: BrowseItem,
	beforeId: string | null,
	now = Date.now()
): void {
	if (beforeId === item.id) return; // dropped on itself
	const existing = p.picks.find((x) => x.id === item.id);
	const rest = p.picks.filter((x) => x.id !== item.id);
	const tile: Pick = existing ?? { ...item, lastUsedAt: now };
	const at = beforeId ? rest.findIndex((x) => x.id === beforeId) : -1;
	if (at < 0) rest.push(tile);
	else rest.splice(at, 0, tile);
	p.picks = rest;
	evictStalest(p);
}

/** Over capacity: drop the tile the user has gone longest without playing or opening. */
function evictStalest(p: Personal): void {
	while (p.picks.length > MAX_PICKS) {
		const stalest = p.picks.reduce((a, b) => (b.lastUsedAt < a.lastUsedAt ? b : a));
		p.picks = p.picks.filter((x) => x !== stalest);
	}
}

/**
 * A tile the app suggests rather than one the user added. Refused if it's already on the grid, if
 * the user has ever removed it, or if the grid is full (a suggestion never evicts something the
 * user chose). Returns whether the grid changed.
 */
export function seedPick(p: Personal, item: BrowseItem, now = Date.now()): boolean {
	if (p.picks.length >= MAX_PICKS) return false;
	if (p.dismissedSeeds.includes(item.id)) return false;
	if (p.picks.some((x) => x.id === item.id)) return false;
	p.picks.push({ ...item, lastUsedAt: now });
	return true;
}

export function removePick(p: Personal, id: string): void {
	p.picks = p.picks.filter((x) => x.id !== id);
	// Every removal is remembered, not just seeded ones: the list only ever gates `seedPick`, and
	// working out which tiles were suggested would mean storing provenance we otherwise don't need.
	// ponytail: grows one string per distinct tile ever removed, so it's bounded by how many times
	// a human clicks the ✕. Cap it if that ever stops being true.
	if (!p.dismissedSeeds.includes(id)) p.dismissedSeeds.push(id);
}

/**
 * Drop everything that points at something which no longer exists — local files the user deleted
 * off disk. Returns how many grid tiles went, so the caller can say so. Unlike `removePick` this
 * doesn't remember the id: nothing was refused, the thing just stopped existing.
 */
export function forgetIds(p: Personal, ids: string[]): number {
	if (!ids.length) return 0;
	const gone = new Set(ids);
	const before = p.picks.length;
	p.picks = p.picks.filter((x) => !gone.has(x.id));
	p.pins = p.pins.filter((id) => !gone.has(id));
	for (const id of ids) delete p.recent[id];
	return before - p.picks.length;
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

/**
 * A stored card read back against the live library list. A Shortcuts tile and a recent both keep the
 * card as it looked the day it went in, so a playlist that has since gained tracks or changed cover
 * kept showing the old count right next to a library grid showing the new one (#67). The library
 * list is fetched and every add/remove patches it, so it wins wherever it holds the same id.
 * Anything the library doesn't know (On Repeat, a local album, something never saved) keeps its
 * snapshot, which is also what makes those tiles render offline.
 *
 * ponytail: library playlists and liked music, because those are what change under you. Feed it
 * `local.albums` too if a rescan ever leaves a local tile's count stale in the same way.
 */
export function freshen<T extends BrowseItem>(item: T, live: BrowseItem[]): T {
	const row = live.find((i) => i.id === item.id);
	if (!row) return item;
	return {
		...item,
		title: row.title,
		subtitle: row.subtitle ?? item.subtitle,
		thumbnail: row.thumbnail ?? item.thumbnail
	};
}

// --- Saved library ------------------------------------------------------------------------------

/** Save or unsave a playlist/album/artist. Returns whether it is saved now. */
export function toggleSaved(p: Personal, item: BrowseItem): boolean {
	if (p.saved.some((s) => s.id === item.id)) {
		p.saved = p.saved.filter((s) => s.id !== item.id);
		return false;
	}
	// The card as it was on screen: opening it refetches everything anyway, so a stale subtitle is
	// the worst this can go wrong, and storing it is what makes the grid render offline.
	p.saved = [{ ...item }, ...p.saved];
	return true;
}

export const isSaved = (p: Personal, id: string): boolean => p.saved.some((s) => s.id === id);

/** Saved here and also on the account, so the account's copy is the one to unsave while signed in. */
export const isSynced = (p: Personal, id: string): boolean =>
	p.saved.some((s) => s.id === id && s.synced === true);

/** What the Library's sync button still has to push. */
export const unsynced = (p: Personal): Saved[] => p.saved.filter((s) => !s.synced);

/**
 * Flag the rows that made it onto the account. They are kept, not dropped: deleting them is what
 * emptied the library the moment the user signed out again, and `mergeSaved` already collapses the
 * local row and YouTube's own into one card while signed in.
 */
export function markSynced(p: Personal, ids: string[]) {
	const done = new Set(ids);
	p.saved = p.saved.map((s) => (done.has(s.id) ? { ...s, synced: true } : s));
}

/**
 * One kind's saved cards followed by YouTube's own, minus anything in both. Deduped by id, which
 * matches across the two: a library grid and a search card come out of the same parser, so an album
 * saved here and liked on YouTube is one `MPRE…` either way.
 */
export function mergeSaved(
	p: Personal,
	items: BrowseItem[],
	kind: BrowseItem['kind']
): BrowseItem[] {
	const local = p.saved.filter((s) => s.kind === kind);
	if (!local.length) return items;
	const have = new Set(items.map((i) => i.id));
	return [...local.filter((s) => !have.has(s.id)), ...items];
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

// --- Home arrangement ---------------------------------------------------------------------------

/**
 * Home's sections in the order the user put them, hidden ones included (the Edit home panel has to
 * list those to offer them back). A section YouTube only started sending after the last save has no
 * rank and sorts to the end, keeping the feed's own order among its peers.
 *
 * Sorting only, never filtering: `hiddenSections` is the other half, and the two callers need the
 * unfiltered list and the visible one respectively.
 *
 * ponytail: sections are keyed by shelf title, which is what the user sees and all YouTube gives us
 * that's stable across sessions. Two shelves sharing a title share one setting — key on
 * `moreBrowseId` if that ever shows up in the wild.
 */
export function arrangeSections<T extends { key: string }>(sections: T[], p: Personal): T[] {
	const ranks = new Map(p.home.order.map((key, i) => [key, i]));
	// A finite sentinel, not Infinity: two unranked sections would subtract to NaN and the comparator
	// would stop being a comparator. Stable sort, so equal ranks keep the order they arrived in.
	const rank = (key: string) => ranks.get(key) ?? Number.MAX_SAFE_INTEGER;
	return sections.slice().sort((a, b) => rank(a.key) - rank(b.key));
}

/**
 * Which sections stay off the page: the ones switched off, plus, under `hideNew`, any the last
 * arrangement didn't rank. Needs an arrangement to be new relative to, so an empty `order` hides
 * nothing extra.
 */
export function hiddenSections(p: Personal): { has(key: string): boolean } {
	const hidden = new Set(p.home.hidden);
	const ranked = new Set(p.home.order);
	const fresh = p.home.hideNew && ranked.size > 0;
	return { has: (key) => hidden.has(key) || (fresh && !ranked.has(key)) };
}

/** Home as it ships. The feed's memory of shelves (`seen`) is not a setting, so it stays. */
export function resetHome(p: Personal): void {
	p.home = { ...defaultHome(), seen: p.home.seen };
}

export function homeIsDefault(p: Personal): boolean {
	const d = defaultHome();
	const h = p.home;
	return (
		!h.order.length &&
		!h.hidden.length &&
		h.hideNew === d.hideNew &&
		h.chips === d.chips &&
		h.backdrop === d.backdrop &&
		h.cards === d.cards
	);
}

/**
 * Home arrives a page at a time, so at any moment the feed holds only the shelves the reader has
 * scrolled to. Edit home lists sections, and listing only the loaded ones meant the list showed
 * five entries before a scroll and fifteen after one. Remember every shelf title the feed has ever
 * rendered and the modal can offer all of them, whether or not this visit has fetched them yet.
 *
 * Returns whether anything was added, so a caller that runs on every page can skip the write.
 *
 * ponytail: newest-first, capped, so a shelf YouTube stops sending ages out instead of sitting in
 * the modal forever. No timestamps — the cap is the only thing that reads the order.
 */
const MAX_SEEN = 40;
export function noteSections(p: Personal, titles: string[]): boolean {
	const have = new Set(p.home.seen);
	const fresh = [...new Set(titles.filter((t) => t && !have.has(t)))];
	if (!fresh.length) return false;
	p.home.seen = [...fresh, ...p.home.seen].slice(0, MAX_SEEN);
	return true;
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
/**
 * Top artists by play count that carry a real channel id. Names-only keys (a song whose artist
 * wasn't linked) are dropped: without an id there is no artist page to look up or open.
 */
export function topArtistIds(p: Personal, n = 7): string[] {
	return Object.entries(p.artists)
		.filter(([id]) => id.startsWith('UC'))
		.sort((a, b) => b[1].count - a[1].count)
		.slice(0, n)
		.map(([id]) => id);
}

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
