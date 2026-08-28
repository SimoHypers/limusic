// Keyed in-memory cache for browse pages: show instantly on revisit, revalidate in background.
// Data is copied into each page's own $state, so no reactivity is needed here.
const TTL_MS = 5 * 60_000; // YouTube browse data is stable on this horizon
// Community cards each cache the playlist they preview, so a scrolled shelf alone can fill 20 slots.
const MAX_ENTRIES = 80;

const store = new Map<string, { data: unknown; at: number }>();

export function getCached<T>(key: string): T | null {
	const e = store.get(key);
	if (!e) return null;
	if (Date.now() - e.at > TTL_MS) {
		store.delete(key);
		return null;
	}
	return e.data as T;
}

export function putCached(key: string, data: unknown): void {
	if (store.size >= MAX_ENTRIES && !store.has(key)) {
		// Map iterates in insertion order — evict the oldest entry.
		const oldest = store.keys().next().value;
		if (oldest !== undefined) store.delete(oldest);
	}
	store.delete(key); // re-insert so revalidated entries move to the back
	store.set(key, { data, at: Date.now() });
}

/** Library ▸ Songs, shared: every write that changes what's in that list has to drop it. */
export const LIBRARY_SONGS_KEY = 'library:songs';

export function invalidateCached(key: string): void {
	store.delete(key);
}

/** Drop everything — browse data is per-account, so sign-in/out makes all of it stale. */
export function clearCached(): void {
	store.clear();
}
