use crate::importer::ImportTrack;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpotifyPlaylist {
    pub name: String,
    pub owner: Option<String>,
    pub tracks: Vec<ImportTrack>,
}

/// Parses a Spotify playlist link or URI in various supported formats into a 22-character base62 ID.
/// Returns None if the input is not a valid Spotify playlist link (e.g. album, track, artist).
pub fn parse_playlist_id(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let id_string: String = if input.starts_with("spotify:playlist:") {
        input["spotify:playlist:".len()..].to_string()
    } else if input.starts_with("http://") || input.starts_with("https://") {
        let Ok(parsed) = reqwest::Url::parse(input) else {
            return None;
        };
        if parsed.host_str() != Some("open.spotify.com") {
            return None;
        }
        let segments: Vec<String> = parsed
            .path_segments()
            .map(|s| s.map(|seg| seg.to_string()).collect())
            .unwrap_or_default();
        let mut candidate_id = None;
        for i in 0..segments.len() {
            if segments[i] == "playlist" && i + 1 < segments.len() {
                candidate_id = Some(segments[i + 1].clone());
                break;
            }
        }
        candidate_id?
    } else {
        input.to_string()
    };

    let id = id_string.split('?').next()?.split('#').next()?.split('/').next()?.to_string();
    if id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(id)
    } else {
        None
    }
}

/// Parses raw HTML from a Spotify embed page, extracting playlist metadata and tracks from __NEXT_DATA__.
pub fn parse_embed_html(html: &str) -> Result<SpotifyPlaylist, String> {
    let marker = "<script id=\"__NEXT_DATA__\"";
    let start_idx = html.find(marker).ok_or_else(|| "spotify_changed".to_string())?;
    let script_content_start =
        html[start_idx..].find('>').ok_or_else(|| "spotify_changed".to_string())? + start_idx + 1;
    let end_marker = "</script>";
    let end_idx = html[script_content_start..]
        .find(end_marker)
        .ok_or_else(|| "spotify_changed".to_string())?
        + script_content_start;

    let json_str = &html[script_content_start..end_idx];
    let v: serde_json::Value =
        serde_json::from_str(json_str).map_err(|_| "spotify_changed".to_string())?;

    let entity = v
        .pointer("/props/pageProps/state/data/entity")
        .ok_or_else(|| "spotify_unavailable".to_string())?;

    let name =
        entity.get("name").and_then(|n| n.as_str()).unwrap_or("Untitled Playlist").to_string();

    let owner = entity.get("subtitle").and_then(|s| s.as_str()).map(|s| s.to_string());

    let track_list = entity
        .get("trackList")
        .and_then(|t| t.as_array())
        .ok_or_else(|| "spotify_empty".to_string())?;

    if track_list.is_empty() {
        return Err("spotify_empty".to_string());
    }

    let mut tracks = Vec::new();
    for item in track_list {
        let entity_type = item.get("entityType").and_then(|e| e.as_str()).unwrap_or("");
        if entity_type != "track" {
            continue;
        }
        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }

        let subtitle = item.get("subtitle").and_then(|s| s.as_str()).unwrap_or("");
        let artists: Vec<String> = if subtitle.is_empty() {
            vec![]
        } else {
            subtitle
                .split(",\u{00A0}")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        let duration_ms = item.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
        let duration_secs = if duration_ms > 0 {
            u32::try_from(duration_ms.saturating_add(500) / 1000).ok()
        } else {
            None
        };

        tracks.push(ImportTrack { title, artists, album: None, duration_secs, isrc: None });
    }

    if tracks.is_empty() {
        return Err("spotify_empty".to_string());
    }

    Ok(SpotifyPlaylist { name, owner, tracks })
}

/// Selects the best successful playlist from multiple attempts (most tracks, first wins ties).
/// If all attempts failed, returns the last error.
pub fn pick_best(results: Vec<Result<SpotifyPlaylist, String>>) -> Result<SpotifyPlaylist, String> {
    let mut best: Option<SpotifyPlaylist> = None;
    let mut last_err: Option<String> = None;
    for res in results {
        match res {
            Ok(playlist) => match &best {
                None => best = Some(playlist),
                Some(b) => {
                    if playlist.tracks.len() > b.tracks.len() {
                        best = Some(playlist);
                    }
                }
            },
            Err(e) => {
                last_err = Some(e);
            }
        }
    }
    match best {
        Some(b) => Ok(b),
        None => Err(last_err.unwrap_or_else(|| "spotify_network".to_string())),
    }
}

const FULL_PAGE: usize = 100;
const LIKELY_COMPLETE_BELOW: usize = 90;

/// Determines whether to stop fetching playlist attempts early.
pub fn should_stop(counts: &[usize]) -> bool {
    if let Some(&last) = counts.last() {
        if last >= FULL_PAGE {
            return true;
        }
    }
    if counts.len() >= 2 {
        let last = counts[counts.len() - 1];
        let prev = counts[counts.len() - 2];
        if last == prev && last < LIKELY_COMPLETE_BELOW {
            return true;
        }
    }
    false
}

/// Fetches a Spotify playlist by URL or ID and parses its embed page, keeping the longest result.
pub async fn fetch_spotify(url: &str) -> Result<SpotifyPlaylist, String> {
    let playlist_id = parse_playlist_id(url).ok_or_else(|| "spotify_invalid_link".to_string())?;
    let fetch_url = format!("https://open.spotify.com/embed/playlist/{}", playlist_id);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
    let mut results = Vec::new();

    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        let attempt_fut = async {
            let resp = crate::http::client()
                .get(&fetch_url)
                .header("User-Agent", crate::http::WEB_UA)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .map_err(|e| {
                    tracing::warn!("Failed to fetch Spotify playlist embed URL: {}", e);
                    "spotify_network".to_string()
                })?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Err("spotify_unavailable".to_string());
            }

            if !resp.status().is_success() {
                tracing::warn!("Spotify embed returned status: {}", resp.status());
                return Err("spotify_network".to_string());
            }

            let html = resp.text().await.map_err(|e| {
                tracing::warn!("Failed to read Spotify embed response text: {}", e);
                "spotify_network".to_string()
            })?;

            parse_embed_html(&html)
        };

        let res = match tokio::time::timeout_at(deadline, attempt_fut).await {
            Ok(res) => res,
            Err(_) => {
                break;
            }
        };

        let successful_counts: Vec<usize> = results
            .iter()
            .filter_map(|r: &Result<SpotifyPlaylist, String>| {
                r.as_ref().ok().map(|p| p.tracks.len())
            })
            .collect();

        match &res {
            Ok(_) => {
                let mut counts = successful_counts;
                if let Ok(p) = &res {
                    counts.push(p.tracks.len());
                }
                results.push(res);
                if should_stop(&counts) {
                    break;
                }
            }
            Err(e) => {
                let is_terminal = e == "spotify_unavailable"
                    || e == "spotify_invalid_link"
                    || e == "spotify_empty";
                if is_terminal && successful_counts.is_empty() {
                    return Err(e.clone());
                } else if is_terminal {
                    results.push(res);
                    break;
                } else {
                    results.push(res);
                }
            }
        }
    }

    pick_best(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_playlist_id() {
        assert_eq!(
            parse_playlist_id("https://open.spotify.com/playlist/AbCdEfGhIjKlMnOpQrStUv?si=abc"),
            Some("AbCdEfGhIjKlMnOpQrStUv".to_string())
        );
        assert_eq!(
            parse_playlist_id("https://open.spotify.com/intl-uk/playlist/AbCdEfGhIjKlMnOpQrStUv"),
            Some("AbCdEfGhIjKlMnOpQrStUv".to_string())
        );
        assert_eq!(
            parse_playlist_id("https://open.spotify.com/embed/playlist/AbCdEfGhIjKlMnOpQrStUv"),
            Some("AbCdEfGhIjKlMnOpQrStUv".to_string())
        );
        assert_eq!(
            parse_playlist_id("spotify:playlist:AbCdEfGhIjKlMnOpQrStUv"),
            Some("AbCdEfGhIjKlMnOpQrStUv".to_string())
        );
        assert_eq!(
            parse_playlist_id("AbCdEfGhIjKlMnOpQrStUv"),
            Some("AbCdEfGhIjKlMnOpQrStUv".to_string())
        );

        assert_eq!(
            parse_playlist_id("https://open.spotify.com/album/AbCdEfGhIjKlMnOpQrStUv"),
            None
        );
        assert_eq!(
            parse_playlist_id("https://open.spotify.com/track/AbCdEfGhIjKlMnOpQrStUv"),
            None
        );
        assert_eq!(
            parse_playlist_id("https://open.spotify.com/artist/AbCdEfGhIjKlMnOpQrStUv"),
            None
        );
        assert_eq!(parse_playlist_id("invalid_id"), None);
        assert_eq!(parse_playlist_id(""), None);
    }

    #[test]
    fn test_parse_embed_html_sample() {
        let html = include_str!("testdata/spotify_embed_sample.html");
        let playlist = parse_embed_html(html).expect("Should parse sample HTML successfully");

        assert_eq!(playlist.name, "Test Playlist");
        assert_eq!(playlist.owner, Some("Test".to_string()));
        assert_eq!(playlist.tracks.len(), 6);

        let track_2 = &playlist.tracks[1];
        assert_eq!(track_2.title, "Just the Two of Us");
        assert_eq!(
            track_2.artists,
            vec!["Grover Washington, Jr.".to_string(), "Bill Withers".to_string()]
        );
        assert_eq!(track_2.duration_secs, Some(444));

        let track_5 = &playlist.tracks[4];
        assert_eq!(track_5.title, "Крузак");
        assert_eq!(track_5.artists, vec!["SKOFKA".to_string()]);
    }

    #[test]
    fn test_parse_embed_html_errors() {
        let html_no_script = "<html><body>No data here</body></html>";
        let res1 = parse_embed_html(html_no_script);
        assert!(res1.is_err());
        assert_eq!(res1.unwrap_err(), "spotify_changed");

        let html_empty_tracks = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props": {"pageProps": {"state": {"data": {"entity": {"type": "playlist", "name": "Empty", "trackList": []}}}}}}</script></body></html>"#;
        let res2 = parse_embed_html(html_empty_tracks);
        assert!(res2.is_err());
        assert_eq!(res2.unwrap_err(), "spotify_empty");
    }

    #[test]
    fn test_parse_embed_html_duration_overflow() {
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props": {"pageProps": {"state": {"data": {"entity": {"type": "playlist", "name": "Overflow", "trackList": [{"uri": "spotify:track:1234567890123456789012", "title": "Too Long", "subtitle": "Artist", "duration": 18446744073709551615, "entityType": "track"}]}}}}}}</script></body></html>"#;
        let playlist = parse_embed_html(html).unwrap();
        assert_eq!(playlist.tracks.len(), 1);
        assert_eq!(playlist.tracks[0].duration_secs, None);
    }

    #[test]
    fn test_should_stop() {
        assert!(should_stop(&[100]));
        assert!(!should_stop(&[94]));
        assert!(!should_stop(&[60]));
        assert!(should_stop(&[60, 60]));
        assert!(!should_stop(&[94, 94]));
        assert!(should_stop(&[94, 98, 100]));
    }

    #[test]
    fn test_pick_best() {
        fn make_playlist(count: usize) -> SpotifyPlaylist {
            SpotifyPlaylist {
                name: "Test".to_string(),
                owner: None,
                tracks: vec![
                    ImportTrack {
                        title: "Track".to_string(),
                        artists: vec![],
                        album: None,
                        duration_secs: None,
                        isrc: None,
                    };
                    count
                ],
            }
        }

        // [94, 100, 98] -> 100
        let r1 =
            pick_best(vec![Ok(make_playlist(94)), Ok(make_playlist(100)), Ok(make_playlist(98))]);
        assert_eq!(r1.unwrap().tracks.len(), 100);

        // [Err, 98] -> 98
        let r2 = pick_best(vec![Err("spotify_network".to_string()), Ok(make_playlist(98))]);
        assert_eq!(r2.unwrap().tracks.len(), 98);

        // [Err, Err] -> last Err
        let r3 = pick_best(vec![
            Err("spotify_network".to_string()),
            Err("spotify_unavailable".to_string()),
        ]);
        assert_eq!(r3.unwrap_err(), "spotify_unavailable");

        // ties keep the first
        let r4 = pick_best(vec![Ok(make_playlist(100)), Ok(make_playlist(100))]);
        assert_eq!(r4.unwrap().tracks.len(), 100);
    }
}
