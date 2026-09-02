//! Live-YouTube extraction smoke test (context/17). NOT in the default run:
//!   cargo test -p innertube --features integration-tests -- --nocapture
#![cfg(feature = "integration-tests")]

use innertube::{
    find_format, AudioQuality, Clients, InnerTube, PlaylistSort, Session, STREAM_FALLBACK_ORDER,
};

const VIDEO_ID: &str = "xl9cFAOKg_Y"; // the id from the user's failing run

/// GET the first KB with the given UA — what mpv effectively does on load.
async fn probe(url: &str, ua: Option<&str>) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let mut req = client.get(url).header("Range", "bytes=0-1023");
    if let Some(ua) = ua {
        req = req.header("User-Agent", ua);
    }
    req.send().await.expect("probe request").status()
}

#[tokio::test]
async fn direct_clients_resolve_and_stream() {
    let it = InnerTube::new(Session::default(), None).unwrap();
    let vd = it.fetch_visitor_data().await.ok();
    let it = InnerTube::new(Session { visitor_data: vd, ..Session::default() }, None).unwrap();
    let clients = Clients::bundled();

    let mut any_ok = false;
    for key in STREAM_FALLBACK_ORDER {
        let client = clients.get(key).unwrap();
        let resp = match it.player(client, VIDEO_ID, None, None, None).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{key}: /player failed: {e}");
                continue;
            }
        };
        if !resp.playability_status.is_ok() {
            eprintln!("{key}: status {}", resp.playability_status.status);
            continue;
        }
        let sd = resp.streaming_data.as_ref().expect("streamingData");
        assert!(sd.expires_in_seconds.is_some(), "{key}: expiry must parse");
        let Some(format) = find_format(sd, AudioQuality::High) else {
            eprintln!("{key}: no audio format");
            continue;
        };
        let Some(url) = format.direct_url() else {
            eprintln!("{key}: itag {} cipher-only", format.itag);
            continue;
        };
        let status = probe(url, Some(&client.user_agent)).await;
        eprintln!("{key}: itag {} -> HTTP {status}", format.itag);
        if status.is_success() {
            any_ok = true;
        }
    }
    assert!(any_ok, "no direct client produced a playable (HTTP 2xx) stream URL");
}

/// Live regression for the "load more duplicates tracks" bug: an owned playlist's continuation
/// embeds a nested duplicate renderer per row. Self-skips unless a real session is supplied:
///   LIMUSIC_COOKIE=… LIMUSIC_VISITOR=… cargo test -p innertube --features integration-tests owned_continuation_not_doubled -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn owned_continuation_not_doubled() {
    let Some(cookie) = std::env::var("LIMUSIC_COOKIE").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipped: set LIMUSIC_COOKIE (+LIMUSIC_VISITOR) to run");
        return;
    };
    let visitor = std::env::var("LIMUSIC_VISITOR").ok().filter(|s| !s.is_empty());
    let it = InnerTube::new(
        Session { cookie: Some(cookie), visitor_data: visitor, ..Session::default() },
        None,
    )
    .unwrap();
    let clients = Clients::bundled();
    let client = clients.get("WEB_REMIX").expect("WEB_REMIX client");

    let libs = it.library_playlists(client).await.expect("library playlists");
    let mut checked = 0;
    for c in &libs {
        let Ok(page) = it.playlist(client, &c.id, None).await else { continue };
        let Some(tok) = page.continuation.clone() else { continue };
        let cont = it.playlist_continuation(client, &tok).await.expect("continuation");
        if cont.items.is_empty() {
            continue; // a suggestions carousel, not more tracks
        }
        checked += 1;
        let mut seen = std::collections::HashSet::new();
        for i in &cont.items {
            assert!(
                seen.insert(i.video_id.clone()),
                "playlist '{}' owned={} continuation doubled video {}",
                page.title.clone().unwrap_or_default(),
                page.owned,
                i.video_id
            );
        }
    }
    assert!(checked > 0, "no playlist with a track continuation found to verify");
    eprintln!("verified {checked} track continuations, no doubling");
}

/// Issue #72: the library grids come back ~25 at a time, so a single browse truncated any bigger
/// library. Asserts the grids page past one response, on an account that has more than one page.
///   LIMUSIC_COOKIE=… LIMUSIC_VISITOR=… cargo test -p innertube --features integration-tests library_grids_page -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn library_grids_page_past_the_first_response() {
    let Some(cookie) = std::env::var("LIMUSIC_COOKIE").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipped: set LIMUSIC_COOKIE (+LIMUSIC_VISITOR) to run");
        return;
    };
    let visitor = std::env::var("LIMUSIC_VISITOR").ok().filter(|s| !s.is_empty());
    let it = InnerTube::new(
        Session { cookie: Some(cookie), visitor_data: visitor, ..Session::default() },
        None,
    )
    .unwrap();
    let clients = Clients::bundled();
    let client = clients.get("WEB_REMIX").expect("WEB_REMIX client");

    let mut paged = 0;
    for (name, items) in [
        ("playlists", it.library_playlists(client).await.expect("library playlists")),
        ("albums", it.library_albums(client).await.expect("library albums")),
        ("artists", it.library_artists(client).await.expect("library artists")),
    ] {
        eprintln!("{name}: {} items", items.len());
        let mut seen = std::collections::HashSet::new();
        for i in &items {
            assert!(seen.insert(i.id.clone()), "{name} grid repeated {} across pages", i.id);
        }
        if items.len() > 25 {
            paged += 1;
        }
    }
    assert!(paged > 0, "no library grid exceeded one page — test this on a fuller account");
}

/// The playlist sort is YouTube's, not ours: the app asks for an order and renders what comes back
/// (`PlaylistSort::params`). Those params are a protobuf literal copied out of YouTube's own menu,
/// so this pins the half that moves under us — that they still sort, and that the menu still says
/// which order the list is in and whether the choice can be written back.
///
/// Read-only on purpose. The write side (`playlist_set_sort`) changes a real playlist for every
/// client on the account, so it is not something a test suite should do behind your back.
///   LIMUSIC_COOKIE=… LIMUSIC_VISITOR=… cargo test -p innertube --features integration-tests playlist_sort -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn playlist_sort_params_still_order_the_server_side_list() {
    let Some(cookie) = std::env::var("LIMUSIC_COOKIE").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipped: set LIMUSIC_COOKIE (+LIMUSIC_VISITOR) to run");
        return;
    };
    let visitor = std::env::var("LIMUSIC_VISITOR").ok().filter(|s| !s.is_empty());
    let it = InnerTube::new(
        Session { cookie: Some(cookie), visitor_data: visitor, ..Session::default() },
        None,
    )
    .unwrap();
    let clients = Clients::bundled();
    let client = clients.get("WEB_REMIX").expect("WEB_REMIX client");

    // Liked Music is the one list every account has, and YouTube sorts it without being asked to
    // store anything on a playlist.
    let plain = it.playlist(client, "VLLM", None).await.expect("liked music");
    let menu = plain.sort_menu.clone().expect("Liked Music still offers a sort menu");
    assert!(!menu.editable, "Liked Music sorts through browse params, not a playlist edit");
    assert!(plain.items.len() > 1, "need more than one track to tell an order from another");

    let titles = |p: &innertube::PlaylistPage| -> Vec<String> {
        p.items.iter().map(|i| i.title.clone()).collect()
    };
    let asc = it.playlist(client, "VLLM", Some((PlaylistSort::Title, false))).await.expect("A-Z");
    let desc = it.playlist(client, "VLLM", Some((PlaylistSort::Title, true))).await.expect("Z-A");
    assert_eq!(
        asc.sort_menu.as_ref().and_then(|m| m.selected),
        Some(PlaylistSort::Title),
        "the menu has to report back the order we asked for"
    );
    assert_ne!(titles(&asc), titles(&desc), "descending params must not return the same page");
    assert_ne!(titles(&asc), titles(&plain), "a title sort must not return the stored order");

    // Put Liked Music back: asking for an order is what persists it on this one list.
    let restored = it.playlist(client, "VLLM", Some((PlaylistSort::Default, false))).await;
    assert!(restored.is_ok(), "failed to restore Liked Music to its stored order");
    eprintln!("title sort verified against {} tracks, Liked Music restored", plain.items.len());
}

/// Every surface has to hand the queue an artist that is a *name*. That string is the player bar,
/// the OS media widget, Discord, and the Last.fm scrobble, so the two bad shapes are: nothing at all
/// (YouTube ships the per-track artist column empty on single-artist albums, `"text": {}`, because
/// the header names the artist) and a whole display subtitle ("Aqua • 1.7B views" off a song card).
/// The unit tests pin how we parse a fixture; this pins the shape YouTube actually sends, which is
/// the half that moves under us.
///   cargo test -p innertube --features integration-tests every_surface -- --nocapture
#[tokio::test]
async fn every_surface_yields_a_scrobbleable_artist() {
    /// Mirrors `lastfm::scrobbleable` plus the "•" tell: a real artist line separates collabs with
    /// "&" or ",", never a bullet, so a bullet means a display string leaked into the field.
    fn bad(artist: &str) -> Option<&'static str> {
        match artist {
            a if a.trim().is_empty() => Some("no artist (would never scrobble)"),
            a if a.contains('•') => {
                Some("display subtitle, not an artist (would scrobble wrong)")
            }
            _ => None,
        }
    }

    let it = InnerTube::new(Session::default(), None).unwrap();
    let vd = it.fetch_visitor_data().await.ok();
    let it = InnerTube::new(Session { visitor_data: vd, ..Session::default() }, None).unwrap();
    let clients = Clients::bundled();
    let c = clients.get(innertube::METADATA_CLIENT).expect("metadata client");

    let mut problems: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let note = |problems: &mut Vec<String>, surface: &str, title: &str, artist: &str| {
        if let Some(why) = bad(artist) {
            problems.push(format!("{surface}: {title:?} → {artist:?} ({why})"));
        }
    };

    // Single-artist albums: the case that shipped broken. Compilations keep a per-row artist, so
    // these are deliberately all one-artist records.
    for q in ["Rumours Fleetwood Mac", "Midnights Taylor Swift", "IGOR Tyler The Creator"] {
        let cards = it.search_cards(c, q, "albums").await.expect("album search");
        let Some(card) = cards.iter().find(|b| b.kind == "album") else {
            problems.push(format!("album search {q:?} returned no album card"));
            continue;
        };
        let album = it.album(c, &card.id).await.expect("album page");
        assert!(!album.items.is_empty(), "album {q:?} parsed with no tracks");
        for t in &album.items {
            checked += 1;
            note(&mut problems, &format!("album {:?}", card.title), &t.title, &t.artists);
        }
    }

    // Search rows, the up-next queue, and an artist page (top songs + every song card in its
    // carousels, which is where the "• 1.7B views" strings came from).
    let songs = it.search_songs(c, "Barbie Girl Aqua").await.expect("search");
    for t in songs.items.iter().take(5) {
        checked += 1;
        note(&mut problems, "search", &t.title, &t.artists);
    }
    if let Some(first) = songs.items.first() {
        for t in it.next(c, Some(&first.video_id), None).await.expect("next").items.iter().take(5) {
            checked += 1;
            note(&mut problems, "queue", &t.title, &t.artists);
        }
    }
    let found = it.search_all(c, "Aqua").await.expect("search_all");
    if let Some(artist) = found.artists.first() {
        let page = it.artist(c, &artist.id).await.expect("artist page");
        for t in &page.top_songs {
            checked += 1;
            note(&mut problems, "artist top songs", &t.title, &t.artists);
        }
        for carousel in &page.sections {
            for card in carousel.items.iter().filter(|i| i.kind == "song") {
                checked += 1;
                let sub = card.subtitle.clone().unwrap_or_default();
                note(&mut problems, &format!("card {:?}", carousel.title), &card.title, &sub);
            }
        }
    }

    assert!(
        checked > 40,
        "only {checked} tracks reached the check, so the surfaces came back empty"
    );
    assert!(
        problems.is_empty(),
        "{checked} tracks checked, {} unscrobbleable:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
    eprintln!("{checked} tracks across album / search / queue / artist surfaces, all with a usable artist");
}

#[tokio::test]
async fn rustypipe_url_is_fetchable() {
    let c =
        innertube::rustypipe_fallback::resolve(VIDEO_ID, true).await.expect("rustypipe resolve");
    let bare = probe(&c.url, None).await;
    eprintln!("rustypipe itag {}: no-UA -> HTTP {bare}", c.itag);
    // mpv sends its own libmpv UA by default; also probe with a browser-ish UA for comparison.
    let browser = probe(&c.url, Some("Mozilla/5.0 (X11; Linux x86_64)")).await;
    eprintln!("rustypipe itag {}: browser-UA -> HTTP {browser}", c.itag);
    assert!(
        bare.is_success() || browser.is_success(),
        "rustypipe URL not fetchable (Raw(-13) root cause)"
    );
}

/// Radio (context/08) is a prefix convention plus one `/next` call, and every part of that is an
/// assumption about YouTube's wire behaviour rather than something a fixture can prove. This
/// checks the three the feature stands on:
///   1. `RDAMVM<videoId>` returns a real queue, not just the seed song.
///   2. `/next` accepts a radio playlist with **no** videoId (how artist radio has to be asked for).
///   3. An artist page still carries a start-radio button to get that playlist id from.
#[tokio::test]
async fn radio_seeds_resolve() {
    let it = InnerTube::new(Session::default(), None).unwrap();
    let vd = it.fetch_visitor_data().await.ok();
    let it = InnerTube::new(Session { visitor_data: vd, ..Session::default() }, None).unwrap();
    let client = Clients::bundled().get(innertube::METADATA_CLIENT).unwrap().clone();

    // 1. Song radio.
    let song = it
        .next(&client, Some(VIDEO_ID), Some(&format!("RDAMVM{VIDEO_ID}")))
        .await
        .expect("song radio /next");
    eprintln!("RDAMVM{VIDEO_ID}: {} tracks", song.items.len());
    assert!(song.items.len() > 1, "song radio came back with only the seed");

    // 2. + 3. Artist radio: the id off the page, then a videoId-less /next with it.
    let artist_id = song.items[0].artist_id.clone().expect("radio rows link their artist");
    let page = it.artist(&client, &artist_id).await.expect("artist page");
    let radio = page.radio_playlist_id.expect("artist header has no start-radio button");
    eprintln!("artist {:?} radio: {radio}", page.name);
    let artist_radio = it.next(&client, None, Some(&radio)).await.expect("artist radio /next");
    eprintln!("{radio}: {} tracks", artist_radio.items.len());
    assert!(artist_radio.items.len() > 1, "playlist-only /next returned nothing");

    // 4. Album radio: `RDAMPL` over the album's *audio* playlist, again with no videoId.
    let album_id =
        song.items.iter().find_map(|i| i.album_id.clone()).expect("a radio row with an album");
    let album = it.album(&client, &album_id).await.expect("album page");
    // Issue #105: the rows themselves never carry it, so `album()` stamps its own id on them.
    // Without that, a track played off a release page has no "Go to album" in its menu.
    assert!(
        album.items.iter().all(|i| i.album_id.as_deref() == Some(album_id.as_str())),
        "album rows reached the queue without the album's id"
    );
    let pl = album.playlist_id.expect("album has no audio playlist");
    let album_radio =
        it.next(&client, None, Some(&format!("RDAMPL{pl}"))).await.expect("album radio /next");
    eprintln!("RDAMPL{pl}: {} tracks", album_radio.items.len());
    assert!(album_radio.items.len() > 1, "album radio came back empty");
}

/// Issue #73: the Library's Songs tab is a plain browse of `FEmusic_liked_videos` read back through
/// the playlist parser (that browseId is YouTube's own Library ▸ Songs, despite the name). Nothing
/// else in the app touches it, so this is what says whether YouTube still answers it with a track
/// shelf and a paging token.
///   LIMUSIC_COOKIE=… LIMUSIC_VISITOR=… cargo test -p innertube --features integration-tests library_songs -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn library_songs_browse_returns_tracks() {
    let Some(cookie) = std::env::var("LIMUSIC_COOKIE").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipped: set LIMUSIC_COOKIE (+LIMUSIC_VISITOR) to run");
        return;
    };
    let visitor = std::env::var("LIMUSIC_VISITOR").ok().filter(|s| !s.is_empty());
    let it = InnerTube::new(
        Session { cookie: Some(cookie), visitor_data: visitor, ..Session::default() },
        None,
    )
    .unwrap();
    let clients = Clients::bundled();
    let client = clients.get("WEB_REMIX").expect("WEB_REMIX client");

    let page = it.playlist(client, "FEmusic_liked_videos", None).await.expect("library songs");
    eprintln!(
        "{} songs, continuation: {}, sort menu: {:?}, title: {:?}, subtitle: {:?}",
        page.items.len(),
        page.continuation.is_some(),
        page.sort_menu,
        page.title,
        page.subtitle
    );
    assert!(!page.items.is_empty(), "no tracks came back — is the account's library empty?");
    for i in &page.items {
        assert!(!i.video_id.is_empty(), "row {:?} has no videoId", i.title);
    }
    let mut seen = std::collections::HashSet::new();
    for i in &page.items {
        assert!(seen.insert(i.video_id.clone()), "first page repeated {}", i.video_id);
    }
    if let Some(t) = &page.continuation {
        let more = it.playlist_continuation(client, t).await.expect("continuation");
        eprintln!("continuation: {} more songs", more.items.len());
        assert!(!more.items.is_empty(), "the paging token resolved to nothing");
    }
}

/// "Add to library" on a song is a `feedbackEndpoint` token minted on the row itself, not a rating,
/// so the whole feature stands on rows still arriving with that menu. It is invisible if it breaks:
/// no tokens simply means the menu never offers the action. Reports what each surface carried, and
/// dumps a row's actual menu when nothing matched, since the shape is the thing in question.
///   LIMUSIC_COOKIE=… LIMUSIC_VISITOR=… cargo test -p innertube --features integration-tests library_tokens -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn song_rows_carry_library_tokens() {
    let Some(cookie) = std::env::var("LIMUSIC_COOKIE").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipped: set LIMUSIC_COOKIE (+LIMUSIC_VISITOR) to run");
        return;
    };
    let visitor = std::env::var("LIMUSIC_VISITOR").ok().filter(|s| !s.is_empty());
    let it = InnerTube::new(
        Session { cookie: Some(cookie), visitor_data: visitor, ..Session::default() },
        None,
    )
    .unwrap();
    let clients = Clients::bundled();
    let client = clients.get("WEB_REMIX").expect("WEB_REMIX client");

    // Every menu entry of the first song row, as YouTube sends it: renderer name, icon, and which
    // kind of endpoint hangs off it. That is exactly the information the parser matches on.
    fn dump_menu(label: &str, row: &serde_json::Value) {
        let Some(menu) = row.get("menu") else {
            let keys: Vec<&str> =
                row.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
            eprintln!("  {label}: no menu on this row. keys: {keys:?}");
            return;
        };
        eprintln!("  {label} menu:");
        for item in
            menu.pointer("/menuRenderer/items").and_then(|i| i.as_array()).unwrap_or(&vec![])
        {
            let Some((renderer, body)) = item.as_object().and_then(|o| o.iter().next()) else {
                continue;
            };
            let icon = |k: &str| {
                body.pointer(&format!("/{k}/iconType")).and_then(|v| v.as_str()).unwrap_or("-")
            };
            let text = body
                .pointer("/text/runs/0/text")
                .or_else(|| body.pointer("/defaultText/runs/0/text"))
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let endpoints: Vec<&str> = body
                .as_object()
                .map(|o| o.keys().filter(|k| k.contains("Endpoint")).map(String::as_str).collect())
                .unwrap_or_default();
            eprintln!(
                "    {renderer}: text={text:?} icon={} default={} toggled={} endpoints={endpoints:?} feedback={}",
                icon("icon"),
                icon("defaultIcon"),
                icon("toggledIcon"),
                serde_json::to_string(body).unwrap_or_default().contains("feedbackEndpoint")
            );
        }
    }

    let raw = it.search_json(client, "daft punk", Some("EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D")).await;
    let search = it.search_songs(client, "daft punk").await.expect("search");
    let with_tokens = search.items.iter().filter(|s| s.library.is_some()).count();
    eprintln!("search: {}/{} rows carry a library action", with_tokens, search.items.len());
    if with_tokens == 0 {
        if let Ok(raw) = &raw {
            let rows = find_rows(raw, "musicResponsiveListItemRenderer");
            eprintln!(
                "  {} raw search rows; feedbackToken anywhere in the response: {}",
                rows.len(),
                serde_json::to_string(raw).unwrap_or_default().contains("feedbackToken")
            );
            if let Some(row) = rows.iter().find(|r| r.get("menu").is_some()).or(rows.first()) {
                dump_menu("search row", row);
            }
        }
    }

    // Search is the surface this feature lives on: every row there carried the toggle when this was
    // written (20/20, 2026-08-28). Zero means the menu shape moved again, and the dump above says how.
    assert!(with_tokens > 0, "no search row carried a library action — the menu shape changed");

    // Library ▸ Songs rows come from a browse, a different response with its own menus. Most of
    // them ship without a menu at all, so this half only reports: it is not a guard.
    // Note the rows that do carry one report `in_library: false` — that list is your *liked*
    // songs, and liking is not the same write as saving to the library.
    let page = it.playlist(client, "FEmusic_liked_videos", None).await.expect("library songs");
    let saved = page.items.iter().filter(|s| s.library.is_some()).count();
    let already =
        page.items.iter().filter(|s| s.library.as_ref().is_some_and(|l| l.in_library)).count();
    eprintln!(
        "library songs: {}/{} carry an action, {} report in_library",
        saved,
        page.items.len(),
        already
    );
    if saved == 0 {
        if let Ok(raw) = it.browse_json(client, "FEmusic_liked_videos").await {
            let rows = find_rows(&raw, "musicResponsiveListItemRenderer");
            eprintln!("  {} raw library rows", rows.len());
            if let Some(row) = rows.iter().find(|r| r.get("menu").is_some()).or(rows.first()) {
                dump_menu("library row", row);
            }
        }
    }
    if let Some(song) = page.items.iter().find(|s| s.library.is_some()) {
        let lib = song.library.as_ref().unwrap();
        eprintln!(
            "  {:?}: in_library={} add={:?} remove={:?}",
            song.title,
            lib.in_library,
            lib.add_token.as_deref().map(|t| &t[..t.len().min(12)]),
            lib.remove_token.as_deref().map(|t| &t[..t.len().min(12)])
        );
    }
}

/// Every `key` node in the tree, in document order (the crate keeps its own walker private).
fn find_rows<'a>(root: &'a serde_json::Value, key: &str) -> Vec<&'a serde_json::Value> {
    let mut out = Vec::new();
    fn walk<'a>(v: &'a serde_json::Value, key: &str, out: &mut Vec<&'a serde_json::Value>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k == key {
                        out.push(val);
                    }
                    walk(val, key, out);
                }
            }
            serde_json::Value::Array(arr) => arr.iter().for_each(|e| walk(e, key, out)),
            _ => {}
        }
    }
    walk(root, key, &mut out);
    out
}
