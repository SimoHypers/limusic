// Settings search (#624): every row of the Settings dialog by its translation keys, so a query can
// list it and the dialog can take the user to it.
//
// Add a row here when you add one to the dialog. `settings-search.check.ts` (run by `pnpm test`)
// fails when a `row({ title: t('…') })` in a settings component has no entry. The global hotkeys
// are not listed: the dialog adds them from `HOTKEY_ACTIONS`, so they can't drift.
//
// Pure, so the check can run it under plain Node.
import type { TranslationKey } from './i18n.svelte';
import type { Indexed } from './localsearch.ts';

export type SettingsTab =
	| 'general'
	| 'themes'
	| 'playback'
	| 'contributing'
	| 'hotkeys'
	| 'discord'
	| 'scrobbling'
	| 'devices'
	| 'data'
	| 'about';

export interface SettingEntry {
	tab: SettingsTab;
	section?: TranslationKey;
	title: TranslationKey;
	desc?: TranslationKey;
	/** The switch this row only shows under. The result says so, and lands there while it is off. */
	under?: TranslationKey;
}

type K = TranslationKey;
type Row = [title: K, desc?: K, under?: K];

// [tab, section label or null, rows], in the order the dialog shows them.
const TABLE: [SettingsTab, K | null, Row[]][] = [
	// One entry per tab, so a word from a tab's hint ("phone") finds the tab itself.
	['general', null, [['settings.tabs.general', 'settings.tabs.general_hint']]],
	['themes', null, [['settings.tabs.themes', 'settings.tabs.themes_hint']]],
	['playback', null, [['settings.tabs.playback', 'settings.tabs.playback_hint']]],
	['contributing', null, [['settings.tabs.contributing', 'settings.tabs.contributing_hint']]],
	['hotkeys', null, [['settings.tabs.hotkeys', 'settings.tabs.hotkeys_hint']]],
	['discord', null, [['settings.tabs.discord', 'settings.tabs.discord_hint']]],
	['scrobbling', null, [['settings.tabs.scrobbling', 'settings.tabs.scrobbling_hint']]],
	['devices', null, [['settings.tabs.devices', 'settings.tabs.devices_hint']]],
	['data', null, [['settings.tabs.data', 'settings.tabs.data_hint']]],
	['about', null, [['settings.tabs.about', 'settings.tabs.about_hint']]],

	['general', 'settings.sections.language', [['settings.general.language', 'settings.general.language_hint']]],
	['general', 'settings.sections.activity', [['player.history', 'settings.playback.play_history_hint']]],
	[
		'general',
		'settings.sections.system',
		[
			['settings.general.close_to_tray', 'settings.general.close_to_tray_hint'],
			['settings.general.track_notifications', 'settings.general.track_notifications_hint'],
			[
				'settings.general.track_notifications_focused',
				'settings.general.track_notifications_focused_hint',
				'settings.general.track_notifications'
			],
			['settings.general.play_on_start', 'settings.general.play_on_start_hint'],
			['settings.general.autostart', 'settings.general.autostart_hint'],
			['settings.general.start_minimized', 'settings.general.start_minimized_hint', 'settings.general.autostart'],
			['settings.general.system_titlebar', 'settings.general.system_titlebar_hint']
		]
	],

	['themes', 'settings.sections.theme', [['settings.themes.mode', 'settings.themes.mode_hint']]],
	[
		'themes',
		'settings.sections.colors',
		[
			['settings.themes.primary_color', 'settings.themes.accent_hint'],
			['settings.themes.background_color', 'settings.themes.tint_hint'],
			['settings.themes.artwork_accent', 'settings.themes.artwork_accent_hint'],
			['settings.themes.matugen', 'settings.themes.matugen_hint']
		]
	],
	[
		'themes',
		'settings.sections.interface',
		[
			['settings.themes.roundness', 'settings.themes.roundness_hint'],
			['settings.themes.zoom', 'settings.themes.zoom_hint'],
			['settings.themes.app_icon', 'settings.themes.app_icon_hint'],
			['settings.themes.blur_playlist_header', 'settings.themes.blur_playlist_header_hint'],
			['settings.themes.now_playing_pill', 'settings.themes.now_playing_pill_hint'],
			['settings.themes.lite_mode', 'settings.themes.lite_mode_hint'],
			['settings.themes.lite_auto', 'settings.themes.lite_auto_hint']
		]
	],
	[
		'themes',
		'settings.sections.typography',
		[
			['settings.themes.interface_font_label', 'settings.themes.interface_font_short_hint'],
			['settings.themes.heading_font_label', 'settings.themes.heading_font_short_hint'],
			['settings.themes.load_font_file', 'settings.themes.load_font_file_hint']
		]
	],
	[
		'themes',
		'settings.sections.player_view',
		[
			['settings.themes.classic_player', 'settings.themes.classic_player_hint'],
			['settings.themes.cover_lyric', 'settings.themes.cover_lyric_hint'],
			['settings.themes.cover_lyric_romanized', 'settings.themes.cover_lyric_romanized_hint'],
			['settings.themes.open_player', 'settings.themes.open_player_hint'],
			['settings.themes.empty_click_player', 'settings.themes.empty_click_player_hint'],
			['settings.themes.tabbed_player', 'settings.themes.tabbed_player_hint'],
			['settings.themes.artwork_background', 'settings.themes.artwork_background_hint'],
			['settings.themes.theater_clock', 'settings.themes.theater_clock_hint'],
			['settings.themes.theater_clock_side', undefined, 'settings.themes.theater_clock'],
			['settings.themes.theater_clock_format', undefined, 'settings.themes.theater_clock']
		]
	],

	[
		'playback',
		'settings.sections.audio',
		[
			['settings.playback.audio_quality', 'settings.playback.audio_quality_hint'],
			['settings.playback.normalize_volume', 'settings.playback.normalize_volume_hint'],
			['settings.playback.autoplay', 'settings.playback.autoplay_hint'],
			['settings.playback.crossfade', 'settings.playback.crossfade_hint'],
			['settings.playback.crossfade_duration', 'settings.playback.crossfade_duration_hint', 'settings.playback.crossfade'],
			['settings.playback.prevent_duplicates', 'settings.playback.prevent_duplicates_hint'],
			['settings.playback.auto_pause_other_audio', 'settings.playback.auto_pause_other_audio_hint'],
			['settings.playback.sticky_shuffle', 'settings.playback.sticky_shuffle_hint'],
			['settings.playback.shuffle_whole_queue', 'settings.playback.shuffle_whole_queue_hint']
		]
	],
	[
		'playback',
		'settings.sections.video',
		[
			['settings.playback.music_videos', 'settings.playback.music_videos_hint'],
			['settings.playback.videos_first', 'settings.playback.videos_first_hint', 'settings.playback.music_videos'],
			['settings.playback.ambient_light', 'settings.playback.ambient_light_hint', 'settings.playback.music_videos'],
			['settings.playback.hide_videos', 'settings.playback.hide_videos_hint']
		]
	],
	['playback', 'settings.sections.blocked', [['settings.playback.blocked_artists', 'settings.playback.blocked_artists_hint']]],
	[
		'playback',
		'settings.sections.lyrics',
		[
			['settings.themes.enhanced_lyrics', 'settings.themes.enhanced_lyrics_hint'],
			[
				'settings.themes.enhanced_lyrics_line_sync',
				'settings.themes.enhanced_lyrics_line_sync_hint',
				'settings.themes.enhanced_lyrics'
			],
			['settings.themes.lyrics_anim', 'settings.themes.lyrics_anim_hint', 'settings.themes.enhanced_lyrics'],
			['settings.themes.lyrics_style', 'settings.themes.lyrics_style_hint', 'settings.themes.enhanced_lyrics'],
			[
				'settings.themes.lyrics_anim_strength',
				'settings.themes.lyrics_anim_strength_hint',
				'settings.themes.enhanced_lyrics'
			],
			['settings.themes.lyrics_glow', 'settings.themes.lyrics_glow_hint', 'settings.themes.enhanced_lyrics'],
			['settings.playback.lyrics_sources', 'settings.playback.lyrics_sources_hint'],
			['settings.playback.lyrics_prefer_words', 'settings.playback.lyrics_prefer_words_hint']
		]
	],
	[
		'playback',
		'karaoke.settings.title',
		[
			['karaoke.settings.model', 'karaoke.settings.model_desc'],
			['karaoke.settings.cache_limit', 'karaoke.settings.cache_limit_desc'],
			['karaoke.settings.cache_used', 'karaoke.settings.cache_used_desc'],
			['karaoke.settings.device', 'karaoke.settings.device_desc_opt_in'],
			['karaoke.settings.status', 'karaoke.settings.status_desc'],
			['karaoke.settings.credit', 'karaoke.consent.credit']
		]
	],
	['playback', 'settings.sections.advanced', [['settings.general.stream_clients']]],

	['hotkeys', null, [['settings.hotkeys.enable', 'settings.hotkeys.description']]],

	[
		'discord',
		null,
		[
			['settings.discord.enable', 'settings.discord.enable_hint'],
			['settings.discord.show_paused', 'settings.discord.show_paused_hint'],
			['settings.discord.hide_details', 'settings.discord.hide_details_hint']
		]
	],
	// Everything below is gone under "Hide details": no `under`, since the hint would have to say
	// "off". The search lands on the tab, where that switch is the third row.
	[
		'discord',
		'settings.sections.discord_status',
		[
			['settings.discord.status_line', 'settings.discord.status_line_hint'],
			['settings.discord.app_name', 'settings.discord.app_name_hint']
		]
	],
	[
		'discord',
		'settings.sections.discord_card',
		[
			['settings.discord.line1', 'settings.discord.line1_hint'],
			['settings.discord.line2', 'settings.discord.line2_hint'],
			['settings.discord.cover', 'settings.discord.cover_hint'],
			['settings.discord.badge', 'settings.discord.badge_hint', 'settings.discord.cover'],
			['settings.discord.line3', 'settings.discord.line3_hint', 'settings.discord.cover'],
			['settings.discord.timestamps', 'settings.discord.timestamps_hint']
		]
	],
	[
		'discord',
		'settings.sections.discord_links',
		[
			['settings.discord.link_line1'],
			['settings.discord.link_line2'],
			['settings.discord.link_cover', undefined, 'settings.discord.cover']
		]
	],
	[
		'discord',
		'settings.sections.discord_buttons',
		[['settings.discord.button1'], ['settings.discord.button2']]
	],

	[
		'scrobbling',
		null,
		[
			// The account cards have no title of their own; these read as one in a result.
			['integrations.lastfm_scrobble_to', 'settings.scrobbling.account_disconnected'],
			['settings.scrobbling.lb_token_placeholder', 'settings.scrobbling.lb_disconnected'],
			['settings.scrobbling.enable', 'settings.scrobbling.enable_hint'],
			['settings.scrobbling.now_playing', 'settings.scrobbling.now_playing_hint']
		]
	],
	[
		'scrobbling',
		'settings.scrobbling.section_timing',
		[
			['settings.scrobbling.percent', 'settings.scrobbling.percent_hint'],
			['settings.scrobbling.minutes', 'settings.scrobbling.minutes_hint']
		]
	],
	[
		'scrobbling',
		'settings.scrobbling.section_metadata',
		[
			['settings.scrobbling.split_titles', 'settings.scrobbling.split_titles_hint'],
			['settings.playback.lastfm_primary_artist', 'settings.playback.lastfm_primary_artist_hint'],
			[
				'settings.playback.lastfm_primary_strict',
				'settings.playback.lastfm_primary_strict_hint',
				'settings.playback.lastfm_primary_artist'
			]
		]
	],
	[
		'scrobbling',
		null,
		[
			['settings.scrobbling.section_rules', 'settings.scrobbling.rules_hint'],
			['settings.scrobbling.section_edits', 'settings.scrobbling.edits_hint']
		]
	],

	['data', 'settings.sections.network', [['settings.general.proxy', 'settings.general.proxy_hint']]],
	[
		'data',
		'settings.sections.storage',
		[
			['settings.data.used', 'settings.data.used_hint'],
			['settings.data.library', 'settings.data.library_hint'],
			['settings.data.audio', 'settings.data.audio_hint'],
			['settings.data.karaoke', 'karaoke.settings.cache_used_desc'],
			['settings.data.models', 'settings.data.models_hint'],
			['settings.data.web', 'settings.data.web_hint'],
			['settings.data.other', 'settings.data.other_hint']
		]
	],

	[
		'about',
		'settings.sections.support',
		[
			['settings.about.discord', 'settings.about.discord_hint'],
			['settings.about.kofi', 'settings.about.kofi_hint']
		]
	],
	[
		'about',
		'settings.sections.updates',
		[
			['settings.about.check_updates'],
			['settings.general.update_banner', 'settings.general.update_banner_hint'],
			['settings.about.beta', 'settings.about.beta_hint']
		]
	],
	[
		'about',
		'settings.sections.report',
		[
			['settings.about.diagnostics', 'settings.about.diagnostics_hint'],
			['settings.about.diagnostics_save', 'settings.about.diagnostics_save_hint'],
			['settings.about.report_issue', 'settings.about.report_issue_hint']
		]
	],
	['about', 'settings.sections.whats_new', [['settings.about.changelog']]]
];

export const SETTINGS: SettingEntry[] = TABLE.flatMap(([tab, section, rows]) =>
	rows.map(([title, desc, under]) => ({ tab, section: section ?? undefined, title, desc, under }))
);

/**
 * The entries ready for `match` (`localsearch.ts`): ranked on the title in the active language, matched against title,
 * description and section in it and in English, so a word from a guide or an issue works too.
 */
export function indexSettings(
	entries: SettingEntry[],
	tr: (key: K) => string,
	en: (key: K) => string
): Indexed<SettingEntry> {
	const name: string[] = [];
	const hay: string[] = [];
	for (const e of entries) {
		const keys = [e.title, e.desc, e.section].filter((k): k is K => !!k);
		const local = keys.map(tr).join(' ');
		const english = keys.map(en).join(' ');
		name.push(tr(e.title).toLowerCase());
		hay.push((local === english ? local : `${local} ${english}`).toLowerCase());
	}
	return { items: entries, name, hay };
}
