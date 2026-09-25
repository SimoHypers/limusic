// Self-check for the pure personalization logic in `personal.ts`. There is no test runner in `ui/`
// and this doesn't warrant adding one — node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/personal.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app, so it
// never reaches the bundle.
import type { BrowseItem } from './api';
import {
	MAX_PICKS,
	addPick,
	arrangeSections,
	empty,
	firstArtist,
	forgetIds,
	freshen,
	hiddenSections,
	homeIsDefault,
	hydrate,
	interleave,
	isSaved,
	isSynced,
	markSynced,
	mergeSaved,
	noteRecent,
	noteSections,
	orderLibrary,
	placePick,
	recentItems,
	removePick,
	resetHome,
	seedPick,
	togglePin,
	toggleSaved,
	topArtistIds,
	touchPick,
	unsynced
} from './personal.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const item = (id: string): BrowseItem => ({ kind: 'playlist', id, title: id });
const ids = (list: { id: string }[]) => list.map((x) => x.id);
const range = (n: number, prefix: string) => Array.from({ length: n }, (_, i) => item(`${prefix}${i}`));

// --- the grid holds only what the user adds; eviction drops the stalest tile ---------------------
{
	const p = empty();
	range(MAX_PICKS, 'm').forEach((it, i) => addPick(p, it, 1000 + i));
	ok(p.picks.length === MAX_PICKS, 'the grid fills to capacity');

	addPick(p, item('newest'), 9000);
	ok(p.picks.length === MAX_PICKS, 'an add over capacity keeps the grid at 18');
	ok(!ids(p.picks).includes('m0'), 'the least recently used tile is evicted');
	ok(ids(p.picks).includes('m1') && ids(p.picks).includes('newest'), 'the rest survive');

	// Playing a tile protects it: it is no longer the stalest.
	touchPick(p, 'm1', 9500);
	addPick(p, item('another'), 9600);
	ok(ids(p.picks).includes('m1'), 'a recently played tile is not evicted');
	ok(!ids(p.picks).includes('m2'), 'the next-stalest goes instead');
}

// --- adding an existing tile refreshes it rather than duplicating --------------------------------
{
	const p = empty();
	addPick(p, item('a'), 100);
	addPick(p, item('b'), 200);
	ok(addPick(p, item('a'), 300) === false, 'a repeat add reports "already there"');
	ok(p.picks.length === 2, 'and does not duplicate the tile');
	ok(p.picks.find((x) => x.id === 'a')!.lastUsedAt === 300, 'but does refresh its recency');
}

// --- removal is permanent: nothing refills the grid ----------------------------------------------
{
	const p = empty();
	range(4, 'm').forEach((it, i) => addPick(p, it, 1000 + i));
	removePick(p, 'm2');
	ok(p.picks.length === 3, 'removal takes the tile out');
	ok(!ids(p.picks).includes('m2'), 'and it stays out');
	removePick(p, 'm0');
	removePick(p, 'm1');
	removePick(p, 'm3');
	ok(p.picks.length === 0, 'the grid can be emptied completely');
}

// --- display order is array order: additions append, use never reshuffles ------------------------
{
	const p = empty();
	range(6, 'c').forEach((it, i) => addPick(p, it, 1000 + i));
	ok(ids(p.picks).join() === 'c0,c1,c2,c3,c4,c5', 'tiles appear in the order they were added');
	touchPick(p, 'c4', 99999);
	ok(ids(p.picks).join() === 'c0,c1,c2,c3,c4,c5', 'playing a tile does not move it');
}

// --- placePick: what a drag lands on. Ordering is the user's, so this is the load-bearing bit -----
{
	const p = empty();
	range(4, 'd').forEach((it, i) => addPick(p, it, 1000 + i)); // d0,d1,d2,d3

	// Leftward move: in front of the tile it was dropped on.
	placePick(p, item('d3'), 'd1');
	ok(ids(p.picks).join() === 'd0,d3,d1,d2', 'a tile moves in front of the drop target');

	// Rightward move: also in front of the target — no off-by-one from the tile leaving its old slot.
	placePick(p, item('d0'), 'd2');
	ok(ids(p.picks).join() === 'd3,d1,d0,d2', 'a rightward move lands in front of the target too');

	// Null target = the end of the grid.
	placePick(p, item('d3'), null);
	ok(ids(p.picks).join() === 'd1,d0,d2,d3', 'a drop past the last tile appends');

	// Dropped on itself: a no-op, not a duplicate or a jump to the end.
	placePick(p, item('d0'), 'd0');
	ok(ids(p.picks).join() === 'd1,d0,d2,d3', 'dropping a tile on itself changes nothing');
	ok(p.picks.length === 4, 'and never duplicates it');

	// A tile dragged in from a shelf inserts at the drop point.
	placePick(p, item('fresh'), 'd2', 5000);
	ok(ids(p.picks).join() === 'd1,d0,fresh,d2,d3', 'a new tile inserts where it was dropped');
	ok(p.picks.find((x) => x.id === 'fresh')!.lastUsedAt === 5000, 'and starts out fresh');
}

// --- placePick respects capacity, evicting the stalest rather than the new arrival ----------------
{
	const p = empty();
	range(MAX_PICKS, 'f').forEach((it, i) => addPick(p, it, 1000 + i));
	placePick(p, item('dropped'), 'f5', 9000);
	ok(p.picks.length === MAX_PICKS, 'an insert over capacity keeps the grid at 18');
	ok(ids(p.picks).includes('dropped'), 'the tile just dropped in survives');
	ok(!ids(p.picks).includes('f0'), 'the stalest tile is the one evicted');
	ok(ids(p.picks)[4] === 'dropped', 'and it sits where it was dropped');

	// Rearranging a full grid must not evict anything.
	placePick(p, item('f9'), null);
	ok(p.picks.length === MAX_PICKS, 'moving a tile on a full grid evicts nothing');
	ok(ids(p.picks).at(-1) === 'f9', 'it just moves to the end');
}

// --- pins: capped at 3, order preserved ---------------------------------------------------------
{
	const p = empty();
	ok(togglePin(p, 'a') === 'pinned' && togglePin(p, 'b') === 'pinned', 'first two pins take');
	ok(togglePin(p, 'c') === 'pinned', 'third pin takes');
	ok(togglePin(p, 'd') === 'full', 'a fourth pin is refused');
	ok(p.pins.join() === 'a,b,c', 'pin order is insertion order');
	ok(togglePin(p, 'b') === 'unpinned', 'toggling an existing pin unpins it');
	ok(p.pins.join() === 'a,c', 'unpinning preserves the order of the rest');
	ok(togglePin(p, 'd') === 'pinned', 'a slot freed by unpinning is usable');
}

// --- library ordering: pins first, then last played, no duplicates ------------------------------
{
	const p = empty();
	const items = [item('a'), item('b'), item('c'), item('d')];
	togglePin(p, 'c');
	noteRecent(p, item('b'), 100);
	noteRecent(p, item('d'), 50);
	const ordered = orderLibrary(items, p);
	ok(ids(ordered).join() === 'c,b,d,a', 'pinned first, then most recently played, then untouched');
	ok(new Set(ids(ordered)).size === ordered.length, 'no playlist appears twice');
	ok(ordered.length === items.length, 'nothing is dropped');

	togglePin(p, 'zzz'); // a pin whose playlist is gone
	ok(orderLibrary(items, p).length === items.length, 'a stale pin does not duplicate or crash');

	// A pinned playlist that is also the most recently played must still appear exactly once.
	noteRecent(p, item('c'), 999);
	const dupCheck = orderLibrary(items, p);
	ok(ids(dupCheck).filter((id) => id === 'c').length === 1, 'pinned + recent is not duplicated');
}

// --- recentItems: newest first, capped, empty when nothing played --------------------------------
{
	const p = empty();
	ok(recentItems(p).length === 0, 'no recent activity yields an empty list');

	noteRecent(p, item('a'), 100);
	noteRecent(p, item('b'), 300);
	noteRecent(p, item('c'), 200);
	ok(ids(recentItems(p)).join() === 'b,c,a', 'newest played-from comes first');
	ok(ids(recentItems(p, 2)).join() === 'b,c', 'n caps the result');
}

// --- interleave dedupes across lists ------------------------------------------------------------
{
	const merged = interleave([[item('x'), item('y')], [item('x'), item('z')]], 10);
	ok(ids(merged).join() === 'x,y,z', 'round-robins and drops repeats');
	ok(interleave([range(9, 'a'), range(9, 'b')], 4).length === 4, 'the cap holds');
}

// --- artist credit parsing ----------------------------------------------------------------------
{
	ok(firstArtist('Daft Punk') === 'Daft Punk', 'a lone artist is unchanged');
	ok(firstArtist('Daft Punk, Pharrell Williams') === 'Daft Punk', 'a comma list takes the lead');
	ok(firstArtist('The Weeknd & Ariana Grande') === 'The Weeknd', 'an ampersand pair takes the lead');
	ok(firstArtist('Drake feat. Rihanna') === 'Drake', 'a feature credit is stripped');
}

// --- hydrate survives junk ----------------------------------------------------------------------
{
	ok(hydrate(null).picks.length === 0, 'null hydrates to empty');
	ok(hydrate('nonsense').pins.length === 0, 'a bad blob hydrates to empty');
	ok(hydrate({ pins: ['a', 'b', 'c', 'd', 'e'] }).pins.length === 3, 'an over-long pin list is cut');
	ok(hydrate({ picks: [{ id: 'a' }, {}] }).picks.length === 1, 'malformed tiles are dropped');

	// Migration off the auto-seeding build: `manual: false` tiles were never chosen by the user.
	const migrated = hydrate({
		picks: [{ id: 'kept', manual: true }, { id: 'seeded', manual: false }, { id: 'new' }]
	});
	ok(ids(migrated.picks).join() === 'kept,new', 'auto-seeded tiles from the old build are dropped');
	ok(hydrate({ dismissedSeeds: ['a', 7] }).dismissedSeeds.join() === 'a', 'junk dismissals drop');
}

// --- a seeded tile is a suggestion: never forced, and never suggested twice ----------------------
{
	const p = empty();
	ok(seedPick(p, item('onrepeat')), 'an empty grid takes the suggestion');
	ok(!seedPick(p, item('onrepeat')), 'a suggestion already on the grid is not re-added');

	// The whole point of the dismissal: removing it has to stick across every later attempt.
	removePick(p, 'onrepeat');
	ok(p.picks.length === 0, 'removing the tile takes it off the grid');
	ok(!seedPick(p, item('onrepeat')), 'a removed suggestion never comes back');
	ok(!seedPick(p, item('onrepeat'), 9999), 'not on a later visit either');

	// A dismissal only gates suggestions. The user can still put it back by hand, and removing it
	// again re-arms the dismissal rather than leaving it stuck on.
	addPick(p, item('onrepeat'));
	ok(ids(p.picks).join() === 'onrepeat', 'the user can still add it manually');
	removePick(p, 'onrepeat');
	ok(!seedPick(p, item('onrepeat')), 'and removing it again still blocks the suggestion');
}

// --- a suggestion must never push a hand-picked tile off a full grid -----------------------------
{
	const p = empty();
	range(MAX_PICKS, 'mine').forEach((it, i) => addPick(p, it, 1000 + i));
	ok(!seedPick(p, item('onrepeat')), 'a full grid refuses the suggestion');
	ok(p.picks.length === MAX_PICKS && !ids(p.picks).includes('onrepeat'), 'nothing was evicted');
}

// --- files that vanish off disk take their tiles, pins and recents with them ---------------------
{
	const p = empty();
	addPick(p, item('LOCALALBUM:gone'));
	addPick(p, item('LOCALALBUM:kept'));
	togglePin(p, 'LOCALALBUM:gone');
	noteRecent(p, item('LOCALALBUM:gone'));
	noteRecent(p, item('LOCALALBUM:kept'));

	ok(forgetIds(p, ['LOCALALBUM:gone']) === 1, 'one tile was dropped');
	ok(ids(p.picks).join() === 'LOCALALBUM:kept', 'only the deleted album left the grid');
	ok(p.pins.length === 0, 'its sidebar pin went too');
	ok(!p.recent['LOCALALBUM:gone'] && !!p.recent['LOCALALBUM:kept'], 'and its recency entry');
	// Not a dismissal: the user refused nothing, so re-adding it later behaves normally.
	ok(seedPick(p, item('LOCALALBUM:gone')), 'a forgotten id can be suggested again if it returns');
	ok(forgetIds(p, []) === 0, 'nothing to forget is a no-op');
}

// --- home arrangement: saved order wins, new sections keep the feed's order at the end -----------
{
	const p = empty();
	const secs = (...keys: string[]) => keys.map((key) => ({ key }));
	const keys = (list: { key: string }[]) => list.map((x) => x.key).join();

	ok(keys(arrangeSections(secs('a', 'b', 'c'), p)) === 'a,b,c', 'no saved order leaves the feed alone');

	p.home = { ...p.home, order: ['c', 'a'], hidden: ['a'] };
	// Hidden sections still come back: the Edit modal lists them so they can be offered again.
	ok(keys(arrangeSections(secs('a', 'b', 'c'), p)) === 'c,a,b', 'saved order first, the rest after');
	ok(hiddenSections(p).has('a') && !hiddenSections(p).has('c'), 'hidden is read back as a set');
	// Two unranked neighbours must not compare as NaN, or the sort silently keeps its input order.
	ok(keys(arrangeSections(secs('z', 'y', 'c'), p)) === 'c,z,y', 'unranked sections hold their order');
	ok(keys(arrangeSections([], p)) === '', 'an empty feed is fine');
}

// --- the arrangement survives a round trip, and a corrupt one degrades instead of throwing --------
{
	const p = empty();
	p.home = { ...p.home, order: ['@recent', 'Listen again'], hidden: ['@forgotten'], seen: ['Listen again'] };
	const back = hydrate(JSON.parse(JSON.stringify(p)));
	// '@familiar' and '@shortcuts' are slotted into an order saved before they existed, so neither
	// sinks to the bottom.
	ok(
		back.home.order.join() === '@shortcuts,@recent,@familiar,Listen again',
		'order survives persistence'
	);
	ok(back.home.hidden.join() === '@forgotten', 'hidden survives persistence');
	ok(back.home.seen.join() === 'Listen again', 'so does the list of shelves home has ever shown');
	ok(
		hydrate({ home: { order: ['Listen again'], hidden: [] } }).home.order.join() ===
			'@shortcuts,Listen again,@familiar',
		'no @recent to sit under: the new section goes last'
	);
	ok(
		hydrate(JSON.parse(JSON.stringify(back))).home.order.join() === back.home.order.join(),
		'the slotting happens once, not on every load'
	);
	ok(hydrate({}).home.order.length === 0, 'a blob from before the feature reads as unarranged');
	ok(hydrate({ home: { order: [1, 'a'], hidden: 'nope' } }).home.order.join() === '@shortcuts,a,@familiar', 'junk is dropped');
}

// --- the rest of Edit home: new-section policy, look, reset --------------------------------------
{
	const p = empty();
	ok(homeIsDefault(p), 'a fresh home is the default one');
	ok(!hiddenSections(p).has('Anything'), 'nothing hidden by default');

	// #317: hide every shelf, and the next page of the feed must not bring replacements.
	p.home = { ...p.home, order: ['@shortcuts', 'Quick picks'], hidden: ['Quick picks'], hideNew: true };
	const h = hiddenSections(p);
	ok(h.has('Quick picks') && h.has('Mixed for you'), 'a section the arrangement never ranked is hidden');
	ok(!h.has('@shortcuts'), 'a ranked, shown one is not');
	p.home.hideNew = false;
	ok(!hiddenSections(p).has('Mixed for you'), 'with the policy off it shows as before');
	// Nothing ranked yet means nothing is new relative to anything: the policy can't blank home.
	p.home = { ...p.home, order: [], hidden: [], hideNew: true };
	ok(!hiddenSections(p).has('Mixed for you'), 'no arrangement, no new sections');

	p.home = { ...p.home, order: ['a'], chips: false, backdrop: false, cards: 'large', seen: ['a', 'b'] };
	ok(!homeIsDefault(p), 'any setting counts as arranged');
	const back = hydrate(JSON.parse(JSON.stringify(p)));
	ok(
		back.home.hideNew && !back.home.chips && !back.home.backdrop && back.home.cards === 'large',
		'the look survives persistence'
	);
	resetHome(p);
	ok(homeIsDefault(p), 'reset is the default');
	ok(p.home.seen.join() === 'a,b', 'but keeps the shelves home has seen, which are not a setting');

	const junk = hydrate({ home: { hideNew: 'yes', chips: 0, cards: 'huge' } }).home;
	ok(!junk.hideNew && junk.chips && junk.backdrop && junk.cards === 'medium', 'junk falls back');
}

// --- the saved library: local saves merge with YouTube's without duplicating anything ------------
{
	const p = empty();
	const album = (id: string): BrowseItem => ({ kind: 'album', id, title: id });

	ok(toggleSaved(p, item('VL1')), 'saving reports it saved');
	ok(isSaved(p, 'VL1'), 'and it reads back as saved');
	ok(!toggleSaved(p, item('VL1')), 'saving the same card again unsaves it');
	ok(!isSaved(p, 'VL1'), 'and it is gone');

	toggleSaved(p, item('VL1'));
	toggleSaved(p, album('MPRE1'));
	toggleSaved(p, album('MPRE2'));
	// Newest first, and only the asked-for kind: the Library page's tabs are built from these.
	ok(ids(mergeSaved(p, [], 'album')).join() === 'MPRE2,MPRE1', 'newest saved first, albums only');
	ok(ids(mergeSaved(p, [], 'playlist')).join() === 'VL1', 'playlists are their own tab');
	ok(mergeSaved(p, [], 'artist').length === 0, 'nothing saved of that kind is empty, not an error');

	// An album saved here and also in the YouTube library is one tile, not two.
	const remote = [album('MPRE1'), album('MPRE9')];
	ok(ids(mergeSaved(p, remote, 'album')).join() === 'MPRE2,MPRE1,MPRE9', 'deduped by id');
	ok(mergeSaved(empty(), remote, 'album') === remote, 'no local saves returns the input untouched');

	// Syncing to an account flags the rows, it does not take them out of the local library: dropping
	// them is what left a signed-out user with nothing after they had signed in once.
	markSynced(p, ['MPRE1', 'VL1']);
	ok(ids(p.saved).join() === 'MPRE2,MPRE1,VL1', 'a synced save is still saved here');
	ok(isSynced(p, 'MPRE1') && !isSynced(p, 'MPRE2'), 'only what synced is flagged');
	ok(ids(unsynced(p)).join() === 'MPRE2', 'the sync button only offers what is left');
	// The account has its own copy now, so the two must still read as one card.
	ok(ids(mergeSaved(p, [album('MPRE1')], 'album')).join() === 'MPRE2,MPRE1', 'no duplicate tile');
	// Re-saving a card that was removed starts over: it has to be pushed again.
	toggleSaved(p, album('MPRE1'));
	toggleSaved(p, album('MPRE1'));
	ok(!isSynced(p, 'MPRE1'), 're-saving clears the synced flag');
	markSynced(p, ['MPRE1']);

	// Saves are not account-scoped, so signing in later must not drop them (hydrate is the only
	// thing that ever rebuilds this list).
	const back = hydrate(JSON.parse(JSON.stringify(p)));
	ok(ids(back.saved).join() === 'MPRE1,MPRE2,VL1', 'saves persist');
	ok(isSynced(back, 'MPRE1'), 'and so does the synced flag');
	ok(hydrate({}).saved.length === 0, 'a blob from before the feature reads as nothing saved');
	ok(hydrate({ saved: [{ id: 'x' }, 'junk'] }).saved.length === 0, 'junk rows are dropped');
}

// --- familiar artists: play counts, but only the ones with a channel to open --------------------
{
	const p = empty();
	p.artists = {
		UCa: { name: 'A', count: 3 },
		UCb: { name: 'B', count: 9 },
		'Some Band': { name: 'Some Band', count: 99 }
	};
	ok(topArtistIds(p).join() === 'UCb,UCa', 'ordered by plays, name-keyed entries dropped');
	ok(topArtistIds(p, 1).join() === 'UCb', 'capped at n');
	ok(topArtistIds(empty()).length === 0, 'no listening history is not an error');
}

// --- stored cards follow the live library ------------------------------------------------------
{
	const stale = { ...item('VL1'), subtitle: '14 tracks', thumbnail: 'old.jpg', lastUsedAt: 5 };
	const live: BrowseItem[] = [
		{ kind: 'playlist', id: 'VL1', title: 'Workout', subtitle: '15 tracks', thumbnail: 'new.jpg' }
	];
	const fresh = freshen(stale, live);
	ok(fresh.subtitle === '15 tracks', 'the live track count wins over the snapshot');
	ok(fresh.title === 'Workout' && fresh.thumbnail === 'new.jpg', 'so do the title and the cover');
	ok(fresh.lastUsedAt === 5, 'and the tile keeps everything the library knows nothing about');
	ok(freshen(stale, []) === stale, 'a card the library has no row for is left alone');
	// On Repeat and local albums are drawn from their own store; a row without art must not blank
	// a snapshot that has some.
	const bare: BrowseItem[] = [{ kind: 'playlist', id: 'VL1', title: 'Workout' }];
	ok(freshen(stale, bare).thumbnail === 'old.jpg', 'a live row with no cover keeps the old one');
	ok(freshen(stale, bare).subtitle === '14 tracks', 'same for a row with no subtitle');
}

// --- every shelf home has ever shown, so Edit home isn't bounded by how far you scrolled --------
{
	const p = empty();
	ok(noteSections(p, ['Quick picks', 'Listen again']), 'the first page is all new');
	ok(!noteSections(p, ['Listen again']), 'a page with nothing new writes nothing');
	ok(noteSections(p, ['Listen again', 'Albums for you']), 'a later page adds only what is new');
	ok(p.home.seen.join() === 'Albums for you,Quick picks,Listen again', 'newest first');
	ok(!noteSections(p, ['', '']), 'a shelf YouTube sent untitled is not a section');
	ok(noteSections(p, ['Dup', 'Dup']) && p.home.seen.filter((t) => t === 'Dup').length === 1,
		'one page carrying the same title twice records it once');
	// The cap is what keeps a shelf YouTube stopped sending from living in the modal forever.
	for (let i = 0; i < 60; i++) noteSections(p, [`shelf ${i}`]);
	ok(p.home.seen.length === 40, 'capped');
	ok(p.home.seen[0] === 'shelf 59', 'and it is the stalest that goes, not the newest');
	ok(!p.home.seen.includes('Quick picks'), 'the first shelves ever seen have aged out');
}

console.log('ok');
