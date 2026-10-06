import * as api from '$lib/api';
import { toast } from '$lib/player.svelte';
import { t, type TranslationKey } from '$lib/i18n.svelte';
import { IS_MAC } from '$lib/shortcuts';
import { comboFromEvent } from '$lib/keycombo';

export interface HotkeyActionDef {
	id: string;
	titleKey: TranslationKey;
	hintKey: TranslationKey;
	section: 'playback' | 'audio' | 'window';
}

export const HOTKEY_ACTIONS: HotkeyActionDef[] = [
	// Playback
	{
		id: 'play_pause',
		titleKey: 'settings.hotkeys.action_play_pause',
		hintKey: 'settings.hotkeys.action_play_pause_hint',
		section: 'playback'
	},
	{
		id: 'next_track',
		titleKey: 'settings.hotkeys.action_next_track',
		hintKey: 'settings.hotkeys.action_next_track_hint',
		section: 'playback'
	},
	{
		id: 'prev_track',
		titleKey: 'settings.hotkeys.action_prev_track',
		hintKey: 'settings.hotkeys.action_prev_track_hint',
		section: 'playback'
	},
	{
		id: 'seek_forward',
		titleKey: 'settings.hotkeys.action_seek_forward',
		hintKey: 'settings.hotkeys.action_seek_forward_hint',
		section: 'playback'
	},
	{
		id: 'seek_backward',
		titleKey: 'settings.hotkeys.action_seek_backward',
		hintKey: 'settings.hotkeys.action_seek_backward_hint',
		section: 'playback'
	},
	{
		id: 'toggle_shuffle',
		titleKey: 'settings.hotkeys.action_toggle_shuffle',
		hintKey: 'settings.hotkeys.action_toggle_shuffle_hint',
		section: 'playback'
	},
	{
		id: 'toggle_repeat',
		titleKey: 'settings.hotkeys.action_toggle_repeat',
		hintKey: 'settings.hotkeys.action_toggle_repeat_hint',
		section: 'playback'
	},
	// Audio
	{
		id: 'volume_up',
		titleKey: 'settings.hotkeys.action_volume_up',
		hintKey: 'settings.hotkeys.action_volume_up_hint',
		section: 'audio'
	},
	{
		id: 'volume_down',
		titleKey: 'settings.hotkeys.action_volume_down',
		hintKey: 'settings.hotkeys.action_volume_down_hint',
		section: 'audio'
	},
	{
		id: 'mute_toggle',
		titleKey: 'settings.hotkeys.action_mute_toggle',
		hintKey: 'settings.hotkeys.action_mute_toggle_hint',
		section: 'audio'
	},
	// Window
	{
		id: 'show_app',
		titleKey: 'settings.hotkeys.action_show_app',
		hintKey: 'settings.hotkeys.action_show_app_hint',
		section: 'window'
	}
];

class HotkeysStore {
	loaded = $state(false);
	saving = $state(false);
	enabled = $state(false);
	wayland = $state(false);
	bindings = $state<Record<string, string>>({});
	errors = $state<Record<string, string>>({});
	recordingAction = $state<string | null>(null);

	async load() {
		try {
			const [res, wayland] = await Promise.all([
				api.getGlobalHotkeys(),
				api.globalHotkeysOnWayland()
			]);
			this.wayland = wayland;
			this.enabled = res.enabled;
			this.bindings = res.bindings || {};
			this.loaded = true;
		} catch (e) {
			console.error('Failed to load global hotkeys', e);
		}
	}

	async save() {
		this.saving = true;
		try {
			const res = await api.setGlobalHotkeys({
				enabled: this.enabled,
				bindings: $state.snapshot(this.bindings)
			});
			this.errors = res.errors || {};
			if (!res.success) {
				if (Object.keys(res.errors || {}).length === 0) {
					this.enabled = res.config.enabled;
					this.bindings = res.config.bindings || {};
				}
				const errorCount = Object.keys(res.errors || {}).length;
				toast.error(
					t('settings.hotkeys.failed_register') + (errorCount > 0 ? ` (${errorCount})` : '')
				);
			}
		} catch (e) {
			toast.error(String(e));
		} finally {
			this.saving = false;
		}
	}

	async toggleEnabled(val: boolean) {
		this.enabled = val;
		await this.save();
	}

	async setBinding(actionId: string, combo: string) {
		// Check for conflict
		for (const [act, c] of Object.entries(this.bindings)) {
			if (act !== actionId && c.toLowerCase() === combo.toLowerCase()) {
				const actDef = HOTKEY_ACTIONS.find((a) => a.id === act);
				const actName = actDef ? t(actDef.titleKey) : act;
				toast.error(t('settings.hotkeys.conflict', { action: actName }));
				return;
			}
		}
		this.bindings[actionId] = combo;
		// Clear recording
		this.recordingAction = null;
		await this.save();
	}

	async clearBinding(actionId: string) {
		delete this.bindings[actionId];
		delete this.errors[actionId];
		await this.save();
	}

	async resetDefaults() {
		this.saving = true;
		try {
			const res = await api.resetGlobalHotkeys();
			this.enabled = res.config.enabled;
			this.bindings = res.config.bindings || {};
			this.errors = res.errors || {};
			if (res.success) {
				toast.success(t('common.done'));
			} else {
				const errorCount = Object.keys(res.errors || {}).length;
				toast.error(
					t('settings.hotkeys.failed_register') + (errorCount > 0 ? ` (${errorCount})` : '')
				);
			}
		} catch (e) {
			toast.error(String(e));
		} finally {
			this.saving = false;
		}
	}
}

export const hotkeys = new HotkeysStore();

/**
 * Format a keyboard event into a normalized shortcut string using canonical physical codes.
 * Returns null for modifiers alone, an unsupported key, or a key with no Ctrl, Alt or Super.
 */
export function eventToShortcut(e: KeyboardEvent): string | null {
	// `true`: a global grab needs a modifier (or an F-key), unlike the app's own shortcuts.
	return comboFromEvent(e, true);
}

const MAC_KEYS: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' };

/** Split shortcut string into individual badges, with macOS's modifier symbols there. */
export function splitShortcut(combo: string): string[] {
	return combo
		.split('+')
		.map((s) => s.trim())
		.filter(Boolean)
		.map((k) => (IS_MAC && MAC_KEYS[k]) || k);
}
