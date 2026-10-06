// What the editable shortcuts *are*: the action list and their defaults. Kept pure (no runes) so
// `keybinddefs.check.ts` can read it in plain node; the reactive store in `keybinds.svelte.ts` wraps it.
import type { TranslationKey } from './i18n.svelte';
import { isLocalBindable, normalizeCombo, IS_MAC, PRIMARY } from './keycombo.ts';

export type KeybindGroup = 'playback' | 'general';

export interface KeybindActionDef {
	id: string;
	titleKey: TranslationKey;
	group: KeybindGroup;
	/** Combos accepted out of the box, in display order. Two only where there have always been two
	 *  (play/pause answers to both Space and `;`). */
	defaults: string[];
}

/**
 * Every editable shortcut. The ids are the `switch` cases in `initShortcuts`, so an action added
 * here without a handler simply never fires.
 */
export const KEYBIND_ACTIONS: KeybindActionDef[] = [
	{ id: 'play_pause', titleKey: 'dialogs.shortcuts.play_pause', group: 'playback', defaults: ['Space', ';'] },
	{ id: 'next', titleKey: 'dialogs.shortcuts.next_song', group: 'playback', defaults: [`${PRIMARY}+F`] },
	{ id: 'prev', titleKey: 'dialogs.shortcuts.previous_song', group: 'playback', defaults: [`${PRIMARY}+D`] },
	{ id: 'shuffle', titleKey: 'dialogs.shortcuts.shuffle_queue', group: 'playback', defaults: [`${PRIMARY}+S`] },
	{ id: 'repeat', titleKey: 'dialogs.shortcuts.toggle_repeat', group: 'playback', defaults: [`${PRIMARY}+R`] },
	// macOS keeps ⌘M for minimize, so mute asks for Shift on top there.
	{ id: 'mute', titleKey: 'dialogs.shortcuts.mute_unmute', group: 'playback', defaults: [IS_MAC ? `${PRIMARY}+Shift+M` : `${PRIMARY}+M`] },
	{ id: 'volume_up', titleKey: 'dialogs.shortcuts.volume_up', group: 'playback', defaults: [`${PRIMARY}+.`] },
	{ id: 'volume_down', titleKey: 'dialogs.shortcuts.volume_down', group: 'playback', defaults: [`${PRIMARY}+,`] },
	{ id: 'refresh', titleKey: 'dialogs.shortcuts.refresh_page', group: 'general', defaults: ['F5'] },
	{ id: 'search', titleKey: 'dialogs.shortcuts.search_anywhere', group: 'general', defaults: [`${PRIMARY}+K`] },
	{ id: 'now_playing', titleKey: 'dialogs.shortcuts.toggle_now_playing', group: 'general', defaults: [`${PRIMARY}+E`] },
	{ id: 'zoom_in', titleKey: 'dialogs.shortcuts.zoom_in', group: 'general', defaults: [`${PRIMARY}+=`] },
	{ id: 'zoom_out', titleKey: 'dialogs.shortcuts.zoom_out', group: 'general', defaults: [`${PRIMARY}+-`] },
	{ id: 'reset_zoom', titleKey: 'dialogs.shortcuts.reset_zoom', group: 'general', defaults: [`${PRIMARY}+0`] },
	// macOS keeps ⌘H for "hide window", so the list answers to ⌘/ there.
	{ id: 'show_list', titleKey: 'dialogs.shortcuts.show_this_list', group: 'general', defaults: [IS_MAC ? `${PRIMARY}+/` : `${PRIMARY}+H`] }
];

/**
 * Keep only the persisted overrides that `set()` could have left behind: a known action, bindable
 * combos, and none that another action ends up answering to. localStorage is user-writable, and a
 * value from a hand-edit (or an older build) must not leave an action on a combo the dispatcher
 * drops, or shadow the action that really owns it. Rejected actions stay on their defaults.
 *
 * Overlaps are resolved against the *final* effective bindings, not the defaults of actions not
 * yet visited: a user can rebind `refresh` from F5 to F9 and then give `next` the freed F5, and
 * both are legitimate together — comparing `next` against `refresh`'s stale F5 default would throw
 * it away. Each pass computes what every action answers to right now (its candidate override, or
 * its defaults once the candidate is emptied) and drops any candidate combo another action holds;
 * emptying a candidate restores a default, which can clash in turn, so this repeats to a fixed
 * point. Each pass only removes, so it terminates.
 */
export function sanitizeOverrides(raw: unknown): Record<string, string[]> {
	const parsed = (typeof raw === 'object' && raw !== null ? raw : {}) as Record<string, unknown>;
	const candidates: Record<string, string[]> = {};
	for (const a of KEYBIND_ACTIONS) {
		const stored = parsed[a.id];
		if (!Array.isArray(stored)) continue;
		const combos: string[] = [];
		for (const value of stored) {
			if (typeof value !== 'string' || !isLocalBindable(value)) continue;
			const norm = normalizeCombo(value);
			if (combos.some((c) => normalizeCombo(c) === norm)) continue;
			combos.push(value);
		}
		if (combos.length) candidates[a.id] = combos;
	}
	const effective = (id: string) =>
		candidates[id] ?? KEYBIND_ACTIONS.find((a) => a.id === id)?.defaults ?? [];
	for (let changed = true; changed; ) {
		changed = false;
		for (const a of KEYBIND_ACTIONS) {
			const combos = candidates[a.id];
			if (!combos) continue;
			const kept = combos.filter((c) => {
				const norm = normalizeCombo(c);
				return !KEYBIND_ACTIONS.some(
					(x) => x.id !== a.id && effective(x.id).some((k) => normalizeCombo(k) === norm)
				);
			});
			if (kept.length === combos.length) continue;
			changed = true;
			if (kept.length) candidates[a.id] = kept;
			else delete candidates[a.id];
		}
	}
	return candidates;
}
