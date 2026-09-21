import * as api from '$lib/api';
import { toast } from '$lib/player.svelte';
import { t, type TranslationKey } from '$lib/i18n.svelte';

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
	enabled = $state(true);
	bindings = $state<Record<string, string>>({});
	errors = $state<Record<string, string>>({});
	recordingAction = $state<string | null>(null);

	async load() {
		try {
			const res = await api.getGlobalHotkeys();
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
				const errorCount = Object.keys(res.errors).length;
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
			toast.success(t('common.done'));
		} catch (e) {
			toast.error(String(e));
		} finally {
			this.saving = false;
		}
	}
}

export const hotkeys = new HotkeysStore();

/**
 * Format a keyboard event into a normalized shortcut string.
 * Returns null if only modifier keys are pressed.
 */
export function eventToShortcut(e: KeyboardEvent): string | null {
	// Modifiers only: ignore
	if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
		return null;
	}

	const parts: string[] = [];
	if (e.ctrlKey) parts.push('Ctrl');
	if (e.altKey) parts.push('Alt');
	if (e.shiftKey) parts.push('Shift');
	if (e.metaKey) parts.push('Super');

	let key = e.key;

	// F-keys
	if (/^F([1-9]|1[0-9]|2[0-4])$/i.test(key)) {
		key = key.toUpperCase();
	} else if (key === ' ') {
		key = 'Space';
	} else if (key === 'ArrowUp') {
		key = 'ArrowUp';
	} else if (key === 'ArrowDown') {
		key = 'ArrowDown';
	} else if (key === 'ArrowLeft') {
		key = 'ArrowLeft';
	} else if (key === 'ArrowRight') {
		key = 'ArrowRight';
	} else if (key === 'PageUp') {
		key = 'PageUp';
	} else if (key === 'PageDown') {
		key = 'PageDown';
	} else if (key === 'Home') {
		key = 'Home';
	} else if (key === 'End') {
		key = 'End';
	} else if (key === 'Insert') {
		key = 'Insert';
	} else if (key === 'Delete') {
		key = 'Delete';
	} else if (key === 'Escape') {
		key = 'Escape';
	} else if (key === 'Enter') {
		key = 'Enter';
	} else if (key === 'Tab') {
		key = 'Tab';
	} else if (key === 'Backspace') {
		key = 'Backspace';
	} else if (key.length === 1) {
		key = key.toUpperCase();
	} else {
		// Capitalize first letter
		key = key.charAt(0).toUpperCase() + key.slice(1);
	}

	parts.push(key);
	return parts.join('+');
}

/** Split shortcut string into individual badges */
export function splitShortcut(combo: string): string[] {
	return combo
		.split('+')
		.map((s) => s.trim())
		.filter(Boolean);
}
