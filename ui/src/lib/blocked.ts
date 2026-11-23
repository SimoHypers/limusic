// Pure helpers for exporting, validating, and merging the blocked-artists JSON list so
// `blocked.check.ts` can verify them under Node with no Svelte or Tauri runtime.
import type { BlockedArtist } from './api';

/** Parse a JSON blocked-artists export (`[{ id?, name }, ...]`). Returns `null` if the payload
 *  is malformed or contains any invalid entry, failing closed rather than importing partial junk. */
export function parseBlockedArtists(raw: string): BlockedArtist[] | null {
	let parsed: unknown;
	try {
		parsed = JSON.parse(raw);
	} catch {
		return null;
	}
	if (!Array.isArray(parsed)) return null;

	const out: BlockedArtist[] = [];
	const seenIds = new Set<string>();
	const seenNames = new Set<string>();

	for (const item of parsed) {
		if (!item || typeof item !== 'object' || Array.isArray(item)) return null;
		const rec = item as Record<string, unknown>;
		if (typeof rec.name !== 'string') return null;
		const name = rec.name.trim();
		if (!name) return null;

		let id: string | undefined;
		if (rec.id !== undefined && rec.id !== null) {
			if (typeof rec.id !== 'string') return null;
			const trimmed = rec.id.trim();
			if (trimmed) id = trimmed;
		}

		const normName = name.toLowerCase();
		if ((id && seenIds.has(id)) || seenNames.has(normName)) continue;
		if (id) seenIds.add(id);
		seenNames.add(normName);

		out.push(id ? { id, name } : { name });
	}
	return out;
}

/** Return the subset of `incoming` not already present in `existing` (matching by `id` or
 *  case-insensitive `name`). */
export function newBlockedArtists(
	incoming: BlockedArtist[],
	existing: BlockedArtist[]
): BlockedArtist[] {
	const existingIds = new Set(existing.map((b) => b.id).filter((id): id is string => !!id));
	const existingNames = new Set(existing.map((b) => b.name.trim().toLowerCase()));
	return incoming.filter(
		(entry) =>
			(!entry.id || !existingIds.has(entry.id)) &&
			!existingNames.has(entry.name.trim().toLowerCase())
	);
}
