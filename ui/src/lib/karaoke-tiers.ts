import type { KaraokeModelId } from './api';
import { karaokeModels } from './player.svelte';

/** The quality tiers, fastest first. `key` names their en.json strings (`karaoke.settings.model_<key>`
 *  and `_desc`); `level` is how many bars the pickers draw for it. */
export const KARAOKE_TIERS: { id: KaraokeModelId; key: 'light' | 'balanced' | 'best'; level: number }[] =
	[
		{ id: 'kara', key: 'light', level: 1 },
		{ id: 'kara_2', key: 'balanced', level: 2 },
		{ id: 'inst_hq_3', key: 'best', level: 3 }
	];

/** A tier's download size (the model, plus the runtime while that isn't installed), or '' while
 *  it isn't known yet. */
export function tierMB(id: KaraokeModelId): string {
	const total = karaokeModels[id]?.total ?? 0;
	return total > 0 ? `${(total / 1048576).toFixed(1)} MB` : '';
}
