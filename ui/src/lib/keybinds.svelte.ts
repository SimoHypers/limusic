// The app's own keyboard shortcuts, made editable (Settings > the shortcuts list). Separate from
// `hotkeys.svelte.ts`, which owns the *global* shortcuts registered with the OS: these fire only
// while Limusic has focus, so they can be anything, bare keys included.
//
// Only overrides are kept, in localStorage (same place as `zoom.svelte.ts`), so the defaults stay
// in code where they can be updated, and a machine that never touched them carries no state.
import { toast } from '$lib/player.svelte';
import { t } from '$lib/i18n.svelte';
import { formatCombo, isLocalBindable, normalizeCombo } from '$lib/keycombo';
import { KEYBIND_ACTIONS, sanitizeOverrides } from '$lib/keybinddefs';

const STORAGE_KEY = 'keybinds';

class KeybindsStore {
	loaded = $state(false);
	/** The action being rebound right now, if any. The global key handlers stand down while it is
	 *  set, so the key being recorded cannot also trigger itself. */
	recording = $state<string | null>(null);
	/** actionId → accepted combos, for the actions the user has changed. */
	overrides = $state<Record<string, string[]>>({});

	/** Read the stored overrides once, on mount (`initShortcuts` calls this). */
	load() {
		if (this.loaded) return;
		this.loaded = true;
		if (typeof localStorage === 'undefined') return;
		try {
			const raw = localStorage.getItem(STORAGE_KEY);
			if (raw) this.overrides = sanitizeOverrides(JSON.parse(raw));
		} catch {
			// Corrupt JSON is not worth a toast; the defaults are the useful fallback.
		}
	}

	/** The combos an action answers to: the override if there is one, the defaults otherwise. */
	combos(id: string): string[] {
		return this.overrides[id] ?? KEYBIND_ACTIONS.find((a) => a.id === id)?.defaults ?? [];
	}

	/** Is this action still on its defaults? Drives the per-row reset affordance. */
	isDefault(id: string): boolean {
		return !(id in this.overrides);
	}

	/** How the action's keys read in a hint or a row. */
	label(id: string): string {
		return this.combos(id).map(formatCombo).join(' or ');
	}

	/** The action a key event maps to, or `null`. */
	actionFor(combo: string): string | null {
		const want = normalizeCombo(combo);
		for (const a of KEYBIND_ACTIONS) {
			if (this.combos(a.id).some((c) => normalizeCombo(c) === want)) return a.id;
		}
		return null;
	}

	/** Rebind an action, refusing a combo the dispatcher would drop or another action already uses.
	 *  Returns whether it stuck. */
	set(id: string, combo: string): boolean {
		if (!isLocalBindable(combo)) {
			toast.error(t('settings.keybinds.unbindable'));
			return false;
		}
		const want = normalizeCombo(combo);
		for (const a of KEYBIND_ACTIONS) {
			if (a.id === id) continue;
			if (this.combos(a.id).some((c) => normalizeCombo(c) === want)) {
				toast.error(t('settings.keybinds.conflict', { action: t(a.titleKey) }));
				return false;
			}
		}
		this.overrides[id] = [combo];
		this.persist();
		return true;
	}

	reset(id: string) {
		delete this.overrides[id];
		// The restored default can now collide with an override another action took meanwhile
		// (`refresh` back to F5 while `next` holds F5). Re-check the survivors the same way a load
		// does, so the reset can't leave two actions on one combo.
		this.overrides = sanitizeOverrides($state.snapshot(this.overrides));
		this.persist();
	}

	resetAll() {
		this.overrides = {};
		this.persist();
	}

	persist() {
		try {
			localStorage.setItem(STORAGE_KEY, JSON.stringify($state.snapshot(this.overrides)));
		} catch {
			// Private mode or a full quota: the shortcut still works for this session.
		}
	}
}

export const keybinds = new KeybindsStore();
