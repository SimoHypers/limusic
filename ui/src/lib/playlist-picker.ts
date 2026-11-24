// Pure helpers for remembering and pinning the last-used playlist targets in `AddToPlaylist`.
export const LAST_PLAYLISTS_KEY = 'last_playlist_ids';

/** Parse a stored JSON array of playlist ids, deduplicating and dropping invalid entries. */
export function parseLastPlaylists(raw: string | null): string[] {
	if (!raw) return [];
	let parsed: unknown;
	try {
		parsed = JSON.parse(raw);
	} catch {
		return [];
	}
	if (!Array.isArray(parsed)) return [];
	const out: string[] = [];
	const seen = new Set<string>();
	for (const entry of parsed) {
		if (typeof entry !== 'string') continue;
		const id = entry.trim();
		if (!id || seen.has(id)) continue;
		seen.add(id);
		out.push(id);
	}
	return out;
}

/** Keep only remembered ids that are valid targets in the current picker context. */
export function pickRemembered(remembered: string[], targetIds: string[]): string[] {
	const available = new Set(targetIds);
	return remembered.filter((id) => available.has(id));
}

/** Pin remembered targets to the top while preserving relative order in each group. */
export function orderWithRemembered<T extends { id: string }>(items: T[], remembered: string[]): T[] {
	if (!remembered.length) return items;
	const pinned = new Set(remembered);
	const top: T[] = [];
	const rest: T[] = [];
	for (const item of items) {
		if (pinned.has(item.id)) top.push(item);
		else rest.push(item);
	}
	return top.length ? [...top, ...rest] : items;
}
