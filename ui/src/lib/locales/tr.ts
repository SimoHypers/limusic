import type { Translations } from './en';

export const tr: Translations = {
	common: {
		loading: 'Yükleniyor...',
		retry: 'Tekrar Dene',
		save: 'Kaydet',
		cancel: 'İptal',
		delete: 'Sil',
		close: 'Kapat',
		search: 'Ara',
		search_placeholder: 'Şarkı, albüm, sanatçı veya çalma listesi arayın...',
		done: 'Bitti',
		remove: 'Kaldır',
		add: 'Ekle',
		create: 'Oluştur',
		back: 'Geri',
		forward: 'İleri',
		custom: 'Özel',
		reset: 'Sıfırla',
		unknown_artist: 'Bilinmeyen Sanatçı',
		songs: 'Şarkılar',
		albums: 'Albümler',
		artists: 'Sanatçılar',
		playlists: 'Çalma Listeleri',
		song_singular: 'Şarkı',
		album_singular: 'Albüm',
		artist_singular: 'Sanatçı',
		playlist_singular: 'Çalma Listesi',
		all: 'Hepsi',
		top_results: 'En iyi sonuçlar',
		results: 'Sonuçlar',
		searching: 'Aranıyor...',
		shuffle: 'Karıştır',
		radio: 'Radyo',
		search_prompt: 'Bir şarkı, albüm, sanatçı veya çalma listesi arayın.',
		no_results: '“{query}” için sonuç bulunamadı.',
		show_more: 'Daha fazla göster',
		results_for: '“{query}” için sonuçlar',
		nothing_found: 'Sonuç bulunamadı.',
		see_all: 'Tümünü gör',
		more: 'Daha fazla',
		less: 'Daha az',
		like: 'Beğen',
		dislike: 'Beğenme',
		minimize: 'Simge durumuna küçült',
		maximize: 'Ekranı kapla',
		try_again: 'Try again', // TODO(tr)
		sign_in_google: 'Sign in with Google', // TODO(tr)
		public: 'Public', // TODO(tr)
		sorting: 'Sorting…', // TODO(tr)
		type_to_search: 'Type to search.', // TODO(tr)
		nothing_quick: 'Nothing quick for that.', // TODO(tr)
		nothing_here: 'Nothing here.', // TODO(tr)
		nothing_matches: 'Nothing matches that.', // TODO(tr)
		search_this_list: 'Search this list', // TODO(tr)
		search_this_album: 'Search this album', // TODO(tr)
		search_this_playlist: 'Search this playlist', // TODO(tr)
		search_your_songs: 'Search your songs', // TODO(tr)
		search_your_music: 'Search your music', // TODO(tr)
		filter_library: 'Filter your library…', // TODO(tr)
		command_description: 'Search songs, albums, artists and playlists', // TODO(tr)
		shuffle_all: 'Shuffle all', // TODO(tr)
		play_all: 'Play all', // TODO(tr)
	},
	nav: {
		home: 'Ana Sayfa',
		search: 'Arama',
		library: 'Kitaplık',
		settings: 'Ayarlar',
		listen_together: 'Birlikte Dinle',
		new_playlist: 'Yeni Çalma Listesi',
		account: 'Hesap',
		sign_in: 'Oturum aç',
		sign_out: 'Çıkış yap',
		switch_channel: 'Kanal değiştir',
		choose_channel: 'Bir YouTube kanalı seçin',
		choose_channel_desc: 'Kitaplık, beğeniler ve çalma listeleri bu kanalı kullanacak. Daha sonra tekrar değiştirebilirsiniz.',
		cancel_sign_in: 'Oturum açmayı iptal et',
		sign_in_hint: 'Sign in with your Google account to reach your YouTube Music library and playlists.', // TODO(tr)
	},
	player: {
		play: 'Oynat',
		pause: 'Duraklat',
		next: 'Sonraki',
		previous: 'Önceki',
		shuffle: 'Karışık',
		repeat_off: 'Tekrar Kapalı',
		repeat_all: 'Tümünü Tekrarla',
		repeat_one: 'Tekrarla (1)',
		mute: 'Sesi Kapat',
		unmute: 'Sesi Aç',
		volume: 'Ses Seviyesi',
		queue: 'Sıra',
		lyrics: 'Şarkı Sözleri',
		mini_player: 'Mini Oynatıcı',
		now_playing: 'Şu An Çalıyor',
		autoplay_notice: 'Otomatik oynatma açık',
		clear_queue: 'Sırayı Temizle',
		empty_queue: 'Sıra boş',
		play_next: 'Sıradakine ekle',
		add_to_queue: 'Sıraya ekle',
		remove_from_queue: 'Sıradan kaldır',
		save_to_playlist: 'Çalma listesine kaydet',
		remove_from_playlist: 'Çalma listesinden kaldır',
		go_to_artist: 'Sanatçıya git',
		go_to_album: 'Albüme git',
		start_radio: 'Radyoyu başlat',
		share: 'Paylaş',
		not_playing: 'Çalmıyor',
		history: 'Geçmiş',
		hide_history: 'Geçmişi gizle',
		show_history: 'Geçmişi göster',
		repeat_state: 'Tekrar: {state}',
		seek: 'Konum',
		open_player: 'Oynatıcıyı aç',
		minimize_player: 'Mini oynatıcıyı küçült',
		remove_from_liked: 'Beğenilen şarkılardan çıkar',
		save_to_liked: 'Beğenilen şarkılara kaydet',
		enlarge_lyrics: 'Şarkı sözlerini büyüt',
		shrink_lyrics: 'Şarkı sözlerini küçült',
		remove_rating: 'Puanı kaldır',
		shuffle_play: 'Shuffle play', // TODO(tr)
		add_to_shortcuts: 'Add to shortcuts', // TODO(tr)
		edit_playlist: 'Edit playlist', // TODO(tr)
		delete_playlist: 'Delete playlist', // TODO(tr),
		remove_dislike: 'Remove dislike', // TODO(tr),
		add_to_playlist: 'Add to playlist', // TODO(tr)
	},
	home: {
		good_morning: 'Günaydın',
		good_afternoon: 'Tünaydın',
		good_evening: 'İyi akşamlar',
		good_night: 'İyi geceler',
		forgotten_favourites: 'Unutulan favoriler',
		familiar_artists: 'Sevdiğiniz sanatçılara benzer',
		shortcuts: 'Kısayollar',
		edit_home: 'Sayfayı Düzenle',
		add_shortcut: 'Kısayol ekle',
		remove_shortcut: 'Remove from shortcuts', // TODO(tr)
		shortcuts_desc: 'En çok dinlediğiniz içerikler ana sayfadan tek tık uzağınızda. Kartları buraya sürükleyin veya kitaplığınızdan seçin.',
		jump_back_in: 'Yeniden dinleyin',
		show_section: '{title} bölümünü göster',
		hide_section: '{title} bölümünü gizle',
		feed_empty: 'Your home feed came back empty this time.', // TODO(tr)
		signed_out_hint: 'Sign in and home fills up with mixes and playlists built from what you listen to.', // TODO(tr)
	},
	artist: {
		subscribe: 'Abone ol',
		subscribed: 'Abone olundu',
		top_songs: 'En popüler şarkılar'
	},
	library: {
		title: 'Kitaplık',
		songs_tab: 'Şarkılar',
		playlists_tab: 'Çalma Listeleri',
		albums_tab: 'Albümler',
		artists_tab: 'Sanatçılar',
		local_tab: 'Yerel',
		matching_songs: '{count} eşleşen',
		matching_songs_so_far: 'şu ana kadar {count} eşleşen',
		sync_idle_tooltip: 'Bu cihazda kayıtlı {count} parçayı YouTube Music kitaplığınıza ekleyin',
		save_to_library: 'Kitaplığa kaydet',
		in_library: 'Kitaplıkta',
		no_matching_tracks: '“{query}” ile eşleşen parça bulunamadı.',
		empty_album: 'Bu albüm boş.',
		play_count: '{count} dinlenme',
		added_by: '{user} tarafından eklendi',
		no_playlists_create: 'No playlists yet — create one in your Library.', // TODO(tr)
		no_playlists_hint: 'No playlists yet. Save one from its page, or create one in your Library.', // TODO(tr)
		empty_playlist: 'This playlist is empty.', // TODO(tr)
		collab: 'Collab', // TODO(tr)
		collab_tooltip: 'Others can add to this playlist', // TODO(tr)
		delete_playlist_confirm: 'Delete this playlist?', // TODO(tr)
		remove_from_library: 'Remove from library', // TODO(tr)
		no_tracks_match_loading: 'No tracks match “{query}”{loading}.', // TODO(tr)
		still_loading: ' yet, still loading', // TODO(tr)
		saved_to_library: 'Saved to library', // TODO(tr)
		removed_from_library: 'Removed from library', // TODO(tr),
		every_song_saved: 'Every song you’ve saved, in one list', // TODO(tr)
	},
	local: {
		scanning: 'Klasör taranıyor...',
		rescan: 'Yeniden tara',
		nothing_matches_device: 'Nothing on this device matches “{query}”.', // TODO(tr)
		albums_count: 'Albums ({count})', // TODO(tr)
		artists_count: 'Artists ({count})', // TODO(tr)
		songs_count: 'Songs ({count})', // TODO(tr),
		add_folder: 'Add folder', // TODO(tr)
		pick_folder_dialog: 'Add a music folder', // TODO(tr)
	},
	settings: {
		title: 'Ayarlar',
		tabs: {
			general: 'Genel',
			general_hint: 'Geçmiş, entegrasyonlar ve uygulamanın nasıl başladığı.',
			themes: 'Görünüm',
			themes_hint: 'Renkler, yazı tipleri ve oynatıcı görünümü.',
			playback: 'Oynatma',
			playback_hint: 'Kalite, sıra davranışı ve şarkı sözleri.',
			data: 'Veri ve depolama',
			data_hint: 'Ağ ve önbellek dosyaları.',
			about: 'Hakkında',
			about_hint: 'Sürüm, güncellemeler ve değişiklikler.'
		},
		general: {
			language: 'Dil (Language)',
			language_hint: 'Arayüzün görüntüleneceği dili seçin.',
			autostart: 'Başlangıçta çalıştır',
			autostart_hint: 'Sistem açıldığında Limusic uygulamasını otomatik başlatın.',
			close_to_tray: 'Sistem tepsisine küçült',
			close_to_tray_hint: 'Pencere kapatıldığında arka planda çalışmaya devam etsin.',
			discord_rpc: 'Discord Rich Presence',
			discord_rpc_hint: 'Çalan şarkıyı Discord profilinizde gösterin.',
			update_banner: 'Güncelleme bildirimleri',
			update_banner_hint: 'Yeni bir Limusic sürümü çıktığında bildirim çubuğu gösterilsin.',
			proxy: 'HTTP/HTTPS Proxy',
			proxy_hint: 'Ağ isteklerini özel bir vekil sunucu üzerinden yönlendirin.',
			proxy_placeholder: 'örn. http://127.0.0.1:7890',
			stream_clients: 'Akış istemcileri',
			stream_clients_hint: 'Ses ayrıştırma için alternatif akış istemcilerini açıp kapatın.'
		},
		themes: {
			accent_themes: 'Vurgu Temaları',
			interface_font: 'Arayüz yazı tipi',
			load_font_file: 'Dosyadan yazı tipi yükle',
			custom_colors: 'Özel Renkler',
			primary_color: 'Ana Vurgu Rengi',
			background_color: 'Arka Plan',
			reset_theme: 'Varsayılan temaya sıfırla',
			accent_colors: 'Accent colors', // TODO(tr)
			palettes: 'Palettes', // TODO(tr)
			your_fonts: 'Your fonts', // TODO(tr)
			custom_font: 'Custom…', // TODO(tr)
			font_placeholder: 'Font installed on this computer, e.g. Inter', // TODO(tr)
			font_aria: '{label} family name', // TODO(tr)
			font_not_installed: 'Not installed — install the font, then reopen settings.', // TODO(tr)
			add_font: 'Add font…', // TODO(tr),
			tint_palette_hint: 'Only shades the default palette, {theme} brings its own colors.', // TODO(tr)
			tint_hint: 'Shades the greys: surfaces, borders and secondary text.', // TODO(tr)
			roundness: 'Roundness', // TODO(tr)
			roundness_hint: 'Corner radius of cards, buttons and artwork.', // TODO(tr)
			load_font_file_hint: 'Load a .ttf, .otf or .woff from anywhere on this computer. It joins both dropdowns above.', // TODO(tr)
			open_player: 'Open the player when you press play', // TODO(tr)
			open_player_hint: 'On, playing a song, album or playlist brings up the full player view. Off, it starts playing and leaves you on the page you were browsing.', // TODO(tr)
			tabbed_player: 'Queue and lyrics in the player view', // TODO(tr)
			tabbed_player_hint: 'On, the player view carries them as tabs and the bar\'s two buttons switch between them. Off, those buttons only ever open the side panels, which stay open over the player view so you can see both at once.', // TODO(tr)
			artwork_background: 'Artwork background', // TODO(tr)
			artwork_background_hint: 'Tint the player view with the playing track\'s cover, blurred. Off leaves it plain.', // TODO(tr)
			artwork_accent: 'Adapt colors to artwork', // TODO(tr)
			artwork_accent_hint: 'Recolor the app from the playing track\'s cover: accent, surfaces and borders, fading between tracks. Off keeps the selected theme\'s own colors.', // TODO(tr)
			reset_theme_hint: 'Drop the color, roundness and font overrides. Keeps the preset.', // TODO(tr)
			interface_font_label: 'Interface font', // TODO(tr)
			interface_font_short_hint: 'Everything except headings.', // TODO(tr)
			heading_font_label: 'Heading font', // TODO(tr)
			heading_font_short_hint: 'Page and section titles.', // TODO(tr)
			load_font_dialog: 'Load a font', // TODO(tr)
			font_filter: 'Fonts', // TODO(tr),
			experimental: 'Experimental', // TODO(tr)
		},
		playback: {
			audio_quality: 'Ses kalitesi',
			audio_quality_hint: 'Daha yüksek kalite daha fazla internet verisi kullanır.',
			quality_low: 'Düşük',
			quality_auto: 'Otomatik',
			quality_high: 'Yüksek',
			autoplay: 'Benzer şarkıları otomatik çal',
			autoplay_hint: 'Sıradaki parçalar bittiğinde önerilen şarkıları çalmaya devam eder.',
			prevent_duplicates: 'Sırada yinelenen şarkıları engelle',
			prevent_duplicates_hint: 'Zaten sırada bulunan şarkıların tekrar eklenmesini önler.',
			play_history_hint: 'Kişiselleştirilmiş öneriler için son dinlenen parçaları hatırlar.',
			music_videos: 'Müzik videolarını etkinleştir',
			music_videos_hint: 'Yalnızca ses yerine uygun olduğunda video akışını gösterir.',
			hide_videos: 'Aramada videoları gizle',
			hide_videos_hint: 'Arama sonuçlarında yalnızca şarkıları ve resmi albümleri gösterir.',
			lyrics_provider: 'Senkronize şarkı sözleri (Boidu/LRCLIB)',
			lyrics_provider_hint: 'Çalan şarkı için topluluk destekli senkronize sözleri çeker.'
		},
		data: {
			clear_cache: 'Görsel ve akış önbelleğini temizle',
			clear_cache_hint: 'Önbelleğe alınan küçük resimleri ve ses parçalarını silerek disk alanı açar.',
			clear_cache_button: 'Önbelleği şimdi temizle'
		},
		about: {
			description: 'Tauri ve Svelte ile geliştirilmiş şık, hızlı ve hafif masaüstü müzik çalar.',
			version: 'Sürüm {version}',
			check_updates: 'Güncellemeleri denetle',
			checking_updates: 'Güncellemeler denetleniyor...',
			up_to_date: 'En güncel sürümü kullanıyorsunuz.',
			update_available: '{version} sürümü mevcut!',
			install_update: 'Yükle ve Yeniden Başlat',
			download_page: 'İndirme sayfasını aç',
			changelog: 'Değişiklik Günlüğü ve Sürüm Notları',
		}
	},
	dialogs: {
		edit_playlist: {
			title: 'Çalma Listesini Düzenle',
			new_title: 'Çalma Listesi Oluştur',
			new_desc: 'Şarkılarınızı düzenlemek için yeni bir çalma listesi oluşturun.',
			name_label: 'Çalma Listesi Adı',
			name_placeholder: 'Favori Parçalarım',
			desc_label: 'Açıklama',
			desc_placeholder: 'Çalma listeniz için bir açıklama yazın...',
			change_cover: 'Görseli değiştir',
			remove_cover: 'Görseli kaldır',
			save_btn: 'Çalma Listesini Kaydet',
			public_on: 'Anyone can find this playlist on YouTube Music.', // TODO(tr)
			public_off: 'Only you can see this playlist.', // TODO(tr)
			artwork_note: 'Artwork applies here at once and uploads to YouTube Music in the background. Square JPEG or PNG works best.', // TODO(tr),
			pick_artwork: 'Choose playlist artwork', // TODO(tr)
			image_filter: 'Images', // TODO(tr)
		},
		share: {
			title: 'Paylaş',
			copy_link: 'Bağlantıyı Kopyala',
			public_link_on: 'Anyone with the link can open it.', // TODO(tr)
			public_link_off: 'Turn on to make the link work for everyone.', // TODO(tr)
			private_note: 'This playlist is private. Anyone you send the link to will get an error.', // TODO(tr)
		},
		shortcuts: {
			title: 'Klavye Kısayolları',
			reopen_hint: '{mod}H brings this back at any time.', // TODO(tr)
			group_search: 'Arama',
			group_playback: 'Oynatma',
			group_window: 'Window', // TODO(tr)
			search_anywhere: 'Search from anywhere', // TODO(tr)
			toggle_now_playing: 'Show or hide the now-playing view', // TODO(tr)
			volume_up: 'Sesi artır',
			volume_down: 'Sesi azalt',
			zoom_in: 'Zoom in', // TODO(tr)
			zoom_out: 'Zoom out', // TODO(tr)
			reset_zoom: 'Reset zoom', // TODO(tr)
			show_this_list: 'Show this list', // TODO(tr)
		},
		tempo_pitch: {
			title: 'Tempo ve Perde Kontrolleri',
			desc: 'Değiştirilene veya uygulama yeniden başlatılana kadar tüm çalınan parçalara uygulanır.',
			tempo: 'Tempo (Hız)',
			pitch: 'Perde (Ton)'
		},
		link: {
			title: 'Bağlantı Aç',
			desc: 'Şarkı, çalma listesi, albüm veya sanatçıya ait bir YouTube Music bağlantısı yapıştırın.',
			open: 'Aç',
			invalid_link: 'Bu geçerli bir YouTube Music bağlantısı değil'
		},
		listen_together: {
			title: 'Birlikte Dinle',
			desc: 'Senkronize dinleme oturumu',
			connecting: 'Bağlanılıyor…',
			waiting_for_host: 'Ev sahibinin sizi içeri alması bekleniyor…',
			join_tab: 'Katıl',
			host_tab: 'Kur',
			invite_code: 'Davet kodu',
			invite_placeholder: 'Arkadaşınızın gönderdiği daveti yapıştırın',
			invite_hint: 'Davet kodu sunucu adresini içerir, başka bir ayara gerek yoktur.',
			your_name: 'Adınız',
			your_name_placeholder: 'Adınız',
			join_button: 'Oturuma katıl',
			sync_server: 'Senkronizasyon sunucusu',
			sync_server_placeholder: 'wss://your-machine.ts.net/ws',
			sync_server_hint: 'Kendi sunucunuz (örn. Tailscale Funnel adresi). Sonrası için kaydedilir.',
			start_button: 'Oturum başlat',
			hosting: 'Sunucu',
			listening: 'Dinleyici',
			copy_invite: 'Daveti kopyala',
			invite_copied: 'Davet kopyalandı, arkadaşınıza gönderin',
			invite_copy_failed: 'Davet kopyalanamadı',
			join_requests: 'Katılma istekleri',
			in_room: 'Odadakiler ({count})',
			you: '(siz)',
			connected: 'Bağlı',
			disconnected: 'Bağlantı kesildi',
			make_host: 'Ev sahibi yap',
			remove: 'Çıkar',
			suggestions: 'Öneriler',
			from_user: '{user} tarafından',
			resync: 'Yeniden senkronize et',
			leave: 'Ayrıl',
			status_connecting: 'Bağlanıyor',
			status_connected: 'Bağlandı',
			status_disconnected: 'Bağlantı kesildi',
			err_enter_name: 'Önce adınızı girin',
			err_enter_server: 'Senkronizasyon sunucu adresini girin',
			err_paste_code: 'Arkadaşınızın gönderdiği davet kodunu yapıştırın',
			err_paste_full_invite: 'Ev sahibinin gönderdiği tam davet kodunu yapıştırın'
		}
	},
	integrations: {
		discord_on: 'Discord durumu açık',
		discord_off: 'Discord durumu kapalı',
		discord_tooltip_on: 'Discord durumu açık — kapatmak için tıklayın',
		discord_tooltip_off: 'Çaldığınız parçayı Discord\'da gösterin',
		lastfm_scrobbling_as: '{user} olarak Scrobble ediliyor',
		lastfm_disconnected: 'Last.fm bağlantısı kesildi',
		lastfm_approve_in_browser: 'Tarayıcınızda Limusic\'e izin verin',
		lastfm_connecting: 'Last.fm\'e bağlanılıyor — iptal etmek için tıklayın',
		lastfm_scrobble_to: 'Last.fm\'e scrobble et',
		disconnect: 'Bağlantıyı kes'
	},
	a11y: {
		close: 'Close', // TODO(tr)
		close_menu: 'Close menu', // TODO(tr)
		close_queue: 'Close queue', // TODO(tr)
		close_lyrics: 'Close lyrics', // TODO(tr)
		play: 'Play', // TODO(tr)
		play_item: 'Play {title}', // TODO(tr)
		play_pause: 'Play/pause', // TODO(tr)
		previous: 'Previous', // TODO(tr)
		next: 'Next', // TODO(tr)
		seek: 'Seek', // TODO(tr)
		volume: 'Volume', // TODO(tr)
		track_options: 'Track options', // TODO(tr)
		playlist_options: 'Playlist options', // TODO(tr)
		clear_search: 'Clear search', // TODO(tr)
		search_preview: 'Search preview', // TODO(tr)
		scroll_left: 'Scroll left', // TODO(tr)
		scroll_right: 'Scroll right', // TODO(tr)
		back_to_top: 'Back to top', // TODO(tr)
		toggle_mini: 'Mini oynatıcıyı aç / kapat',
		toggle_theme: 'Toggle theme', // TODO(tr)
		explicit: 'Explicit', // TODO(tr)
		shrink_lyrics: 'Shrink lyrics', // TODO(tr)
		expand_lyrics: 'Expand lyrics', // TODO(tr)
		public_playlist: 'Public playlist', // TODO(tr)
		remove_folder: 'Remove folder', // TODO(tr)
		remove_font: 'Remove {name}', // TODO(tr)
		sync_to_ytm: 'Sync {count} saved items to YouTube Music', // TODO(tr)
		listen_together: 'Listen Together', // TODO(tr)
		saturation_brightness: 'Saturation and brightness', // TODO(tr)
		hue: 'Hue', // TODO(tr)
		pick_colour: 'Pick a colour from the screen', // TODO(tr)
		hex_colour: 'Hex colour', // TODO(tr)
		theme: 'Theme', // TODO(tr)
		choose_accent: 'Choose accent color', // TODO(tr)
		background_tint: 'Background tint', // TODO(tr)
		roundness: 'Roundness', // TODO(tr)
		more_options: 'More options', // TODO(tr),
		expand_sidebar: 'Expand sidebar', // TODO(tr)
		collapse_sidebar: 'Collapse sidebar', // TODO(tr),
		show_artwork: 'Show artwork', // TODO(tr)
		show_video: 'Show video', // TODO(tr)
	},
	lyrics: {
		title: 'Lyrics', // TODO(tr)
		instrumental: 'Instrumental', // TODO(tr)
		none_found: 'No lyrics found for this track.', // TODO(tr)
	},
	queue: {
		title: 'Queue', // TODO(tr)
	},
	sort: {
		label: 'Sort', // TODO(tr)
		default: 'Default', // TODO(tr)
		newest: 'Newest first', // TODO(tr)
		oldest: 'Oldest first', // TODO(tr)
		title: 'Title', // TODO(tr)
		artist: 'Artist', // TODO(tr)
		album: 'Album', // TODO(tr)
		plays: 'Most played', // TODO(tr)
		direction: 'Sort direction: {dir}', // TODO(tr)
		ascending: 'ascending', // TODO(tr)
		descending: 'descending', // TODO(tr)
		top: 'Top voted', // TODO(tr)
	},
	changelog: {
		version: 'Version {version}', // TODO(tr)
		installed: 'installed', // TODO(tr)
		no_releases: 'No releases yet.', // TODO(tr)
		load_failed: 'Couldn\'t load the changelog ({error}).', // TODO(tr)
	},
	toasts: {
		could_not_play: 'Could not play — try opening it instead', // TODO(tr)
		could_not_queue: 'Could not queue that — try opening it instead', // TODO(tr)
		could_not_load_playlist: 'Could not load that playlist — try opening it instead', // TODO(tr)
		could_not_load_more: 'Could not load more', // TODO(tr)
		could_not_copy_link: 'Could not copy the link. Select it and press Ctrl+C.', // TODO(tr)
		browser_failed: 'Couldn\'t open the browser: {error}', // TODO(tr)
		update_failed: 'Update failed: {error}', // TODO(tr)
		shortcuts_dropped_one: 'Removed 1 shortcut for deleted music', // TODO(tr)
		shortcuts_dropped: 'Removed {count} shortcuts for deleted music', // TODO(tr)
		pins_full: 'Unpin one first — {max} pins max', // TODO(tr)
		starting_radio: 'Starting radio…', // TODO(tr)
		signed_in: 'Signed in', // TODO(tr)
		folder_removed: 'Folder removed from your local library', // TODO(tr)
		playlist_updated: 'Playlist updated', // TODO(tr)
		playlist_created: 'Created "{title}"', // TODO(tr)
		playlist_deleted: 'Playlist deleted', // TODO(tr)
		removed_from_library: 'Removed from library', // TODO(tr)
		removed_from_liked: 'Removed from Liked Music', // TODO(tr)
		removed_from_playlist: 'Removed from playlist', // TODO(tr)
		font_loaded: '{name} loaded — pick it above', // TODO(tr)
		quality_updated: 'Audio quality updated', // TODO(tr)
		proxy_saved: 'Proxy saved — restart to apply', // TODO(tr)
		caches_cleared: 'Caches cleared', // TODO(tr)
		synced_partial: 'Synced {synced} of {total}. {failed} failed, still saved here.', // TODO(tr)
		synced_none: 'Nothing synced. {failed} failed, still saved here.', // TODO(tr)
		synced_all: 'Synced {count} to YouTube Music', // TODO(tr)
		added_to_playlist_dupes: 'Added {count} to {playlist} ({dupes} already there)', // TODO(tr)
		partial_playlist_queued: 'Couldn\'t load all of this playlist, so only what loaded was queued.', // TODO(tr)
		partial_playlist_added: 'Couldn\'t load all of this playlist, so only what loaded was added.', // TODO(tr)
		sort_not_saved: 'Sorted, but couldn\'t save it to YouTube: {error}', // TODO(tr)
		sort_failed: 'Couldn\'t sort this playlist: {error}', // TODO(tr),
		already_in_all: 'All {count} are already in {playlist}', // TODO(tr)
		already_in: 'Already in {playlist}', // TODO(tr)
		added_songs: 'Added {count} songs to {playlist}', // TODO(tr)
		added_one: 'Added to {playlist}', // TODO(tr)
	}
};
