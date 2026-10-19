//! Listening stats (#396): the stats page's numbers, shaped from what [`Db::listening`] reads.
//! Worked out on every visit rather than kept up to date as songs play: a year of heavy listening is
//! tens of thousands of rows, which SQLite groups in a few milliseconds.
//!
//! [`Db::listening`]: crate::db::Db::listening

use std::collections::HashMap;

use innertube::SongItem;
use serde::Serialize;

use crate::db::Listening;
use crate::local;

/// Past this length a video is a stream, a VOD, a podcast episode or an hour-long mix, not a song.
/// It still counts as time listened, but never as a song, an artist or an album: an evening of a
/// streamer's VODs made the streamer a top artist, and a podcast's byline ("Sep 14") another.
/// Audio tracks and files are songs at any length (an album's twenty-minute closer is one).
const LONGEST_VIDEO_SONG_SECS: i64 = 15 * 60;

/// How much of each top list the page shows.
const TOP_SONGS: usize = 10;
const TOP_ARTISTS: usize = 8;
const TOP_ALBUMS: usize = 6;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Seconds heard in the window.
    pub secs: i64,
    /// The same, for the window of the same length just before it. `None` for all time, which has
    /// nothing before it.
    pub prev_secs: Option<i64>,
    pub plays: i64,
    pub songs: i64,
    /// Songs played for the first time ever in the window.
    pub new_songs: i64,
    pub artists: i64,
    /// Seconds per local day (`2026-10-17`), oldest first, days with nothing left out.
    pub days: Vec<(String, i64)>,
    /// Seconds per local weekday (0 = Sunday) and hour: `hours[day * 24 + hour]`.
    pub hours: Vec<i64>,
    pub top_songs: Vec<TopSong>,
    pub top_artists: Vec<TopArtist>,
    pub top_albums: Vec<TopAlbum>,
    /// The oldest play on record. `None` means nothing has been recorded at all.
    pub first_at: Option<i64>,
    /// Days in a row with a play, up to today (or yesterday: today isn't over), and the longest
    /// run ever. Whole history, whatever the window.
    pub streak: i64,
    pub best_streak: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSong {
    pub song: SongItem,
    pub plays: i64,
    pub secs: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopArtist {
    pub name: String,
    /// Their channel (`UC…`), or a local artist's page (`LOCALARTIST:…`). `None` when neither.
    pub id: Option<String>,
    /// The cover of their most-heard song in the window, for when there's no photo to show.
    pub thumbnail: Option<String>,
    pub plays: i64,
    pub secs: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub thumbnail: Option<String>,
    pub plays: i64,
    pub secs: i64,
}

/// `all_time`: the window reaches back past the first play, so there's no previous one to compare.
pub fn shape(raw: Listening, all_time: bool) -> Stats {
    let since = raw.since;
    let mut parsed = parse(raw.songs);
    let secs = parsed.iter().map(|(s, _)| s.secs).sum();
    let plays = parsed.iter().map(|(s, _)| s.plays).sum();
    parsed.retain(|(s, _)| is_song(&s.song));
    let new_songs = parsed.iter().filter(|(_, first)| *first >= since).count() as i64;
    let mut songs: Vec<TopSong> = parsed.into_iter().map(|(s, _)| s).collect();
    // Most heard first, so each artist and album takes its picture from the song heard most.
    songs.sort_by(|a, b| b.secs.cmp(&a.secs).then(b.plays.cmp(&a.plays)));

    let mut artists = tally_artists(&songs);
    let mut albums: HashMap<&str, TopAlbum> = HashMap::new();
    for s in &songs {
        // ponytail: only albums YouTube links. A local file has no album id on its row (see
        // `local::to_song`), so a library of files has no top albums. Rebuild its `LOCALALBUM:`
        // key from the stored track if that is ever missed.
        if let (Some(id), Some(title)) = (s.song.album_id.as_deref(), s.song.album.as_ref()) {
            let al = albums.entry(id).or_insert_with(|| TopAlbum {
                id: id.to_owned(),
                title: title.clone(),
                artist: credits(&s.song).into_iter().next().map(|(_, n)| n).unwrap_or_default(),
                thumbnail: s.song.thumbnail.clone(),
                plays: 0,
                secs: 0,
            });
            al.plays += s.plays;
            al.secs += s.secs;
        }
    }

    let mut hours = vec![0; 7 * 24];
    for (day, hour, secs) in raw.hours {
        if let Some(slot) = usize::try_from(day * 24 + hour).ok().and_then(|i| hours.get_mut(i)) {
            *slot = secs;
        }
    }
    let (streak, best_streak) = streaks(&raw.day_numbers, raw.today);
    let artist_count = artists.len() as i64;
    artists.sort_by(|a, b| b.secs.cmp(&a.secs).then(b.plays.cmp(&a.plays)));
    artists.truncate(TOP_ARTISTS);
    let mut albums: Vec<TopAlbum> = albums.into_values().collect();
    albums.sort_by(|a, b| b.secs.cmp(&a.secs).then(b.plays.cmp(&a.plays)));
    albums.truncate(TOP_ALBUMS);

    let stats = Stats {
        secs,
        prev_secs: (!all_time).then_some(raw.prev_secs),
        plays,
        songs: songs.len() as i64,
        new_songs,
        artists: artist_count,
        days: raw.days,
        hours,
        top_songs: Vec::new(),
        top_artists: artists,
        top_albums: albums,
        first_at: raw.first_at,
        streak,
        best_streak,
    };
    // Songs rank by plays, the way a chart counts them; artists and albums by time heard, so an
    // album of long tracks isn't beaten by a pile of short ones.
    songs.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.secs.cmp(&a.secs)));
    songs.truncate(TOP_SONGS);
    Stats { top_songs: songs, ..stats }
}

/// Home's Familiar Artists (#497): the `n` most played artists in `songs` (the rows of
/// [`Listening::songs`]), counted exactly as the stats page counts them. Home used to keep a tally
/// of its own, bumped as each song started, so skips and every launch counted there and not here.
pub fn top_artists(songs: Vec<(String, i64, i64, i64)>, n: usize) -> Vec<TopArtist> {
    let mut songs: Vec<TopSong> =
        parse(songs).into_iter().map(|(s, _)| s).filter(|s| is_song(&s.song)).collect();
    songs.sort_by(|a, b| b.secs.cmp(&a.secs).then(b.plays.cmp(&a.plays)));
    let mut artists = tally_artists(&songs);
    // By plays, where the stats page goes by time: a poster shows only its play count.
    artists.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.secs.cmp(&a.secs)));
    artists.truncate(n);
    artists
}

/// The rows whose song still parses, each with its first play ever.
fn parse(rows: Vec<(String, i64, i64, i64)>) -> Vec<(TopSong, i64)> {
    rows.into_iter()
        .filter_map(|(json, plays, secs, first)| {
            Some((TopSong { song: serde_json::from_str(&json).ok()?, plays, secs }, first))
        })
        .collect()
}

/// Every artist credited on `songs`. One row per lowercased name, so the same artist from a YouTube
/// song and from a file is one row, and per id, so a channel credited under two names (a renamed
/// or localized credit) is one too. `songs` come most heard first: each artist takes its picture
/// from the first.
fn tally_artists(songs: &[TopSong]) -> Vec<TopArtist> {
    let mut out: Vec<TopArtist> = Vec::new();
    // Names lowercased, ids as they are: a lowercased name can't pass for a `UC…` id.
    let mut row: HashMap<String, usize> = HashMap::new();
    for s in songs {
        for (id, name) in credits(&s.song) {
            let key = name.to_lowercase();
            let i = match id.as_ref().and_then(|id| row.get(id)).or_else(|| row.get(&key)) {
                Some(&i) => i,
                None => {
                    out.push(TopArtist {
                        name,
                        id: None,
                        thumbnail: s.song.thumbnail.clone(),
                        plays: 0,
                        secs: 0,
                    });
                    out.len() - 1
                }
            };
            let a = &mut out[i];
            a.plays += s.plays;
            a.secs += s.secs;
            if a.id.is_none() {
                a.id = id;
            }
            row.insert(key, i);
            if let Some(id) = &a.id {
                row.insert(id.clone(), i);
            }
        }
    }
    out
}

/// Whether a play counts as a song. See [`LONGEST_VIDEO_SONG_SECS`]. A video of unknown length is
/// given the benefit of the doubt.
fn is_song(s: &SongItem) -> bool {
    !s.is_video
        || crate::state::parse_duration_ms(s.duration.as_deref()) <= LONGEST_VIDEO_SONG_SECS * 1000
}

/// Who a song counts toward: every artist its credit links (a collab counts for each of them), or
/// else the lead name of the plain credit, pointed at a local file's artist page where it is one.
fn credits(s: &SongItem) -> Vec<(Option<String>, String)> {
    let linked: Vec<_> = s
        .artist_runs
        .iter()
        .filter_map(|r| Some((Some(r.id.clone()?), r.text.trim().to_owned())))
        .collect();
    if !linked.is_empty() {
        return linked;
    }
    let name = local::primary_artist(&s.artists);
    if name.is_empty() || name == local::UNKNOWN_ARTIST {
        return Vec::new();
    }
    let id = if local::is_local_song(&s.video_id) {
        Some(format!("{}{name}", local::ARTIST_PREFIX))
    } else {
        s.artist_id.clone()
    };
    vec![(id, name.to_owned())]
}

/// `(current, longest)` run of consecutive days in `days` (ascending day numbers). The current run
/// survives until a whole day passes without a play: today isn't over until midnight.
fn streaks(days: &[i64], today: i64) -> (i64, i64) {
    let (mut run, mut best, mut last) = (0, 0, None);
    for &d in days {
        run = if last == Some(d - 1) { run + 1 } else { 1 };
        best = best.max(run);
        last = Some(d);
    }
    let current = if last.is_some_and(|d| d >= today - 1) { run } else { 0 };
    (current, best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use innertube::models::metadata::ArtistRun;

    fn song(id: &str, artists: &str, runs: &[(&str, Option<&str>)]) -> String {
        serde_json::to_string(&SongItem {
            video_id: id.into(),
            title: id.into(),
            artists: artists.into(),
            artist_runs: runs
                .iter()
                .map(|(t, id)| ArtistRun { text: (*t).into(), id: id.map(Into::into) })
                .collect(),
            // Like `local::to_song`: a file names its album but links none.
            album: Some("Album".into()),
            album_id: (!local::is_local_song(id)).then(|| "MPREalbum".into()),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn a_collab_counts_for_each_artist_and_a_file_for_its_lead() {
        let raw = Listening {
            since: 100,
            songs: vec![
                (
                    song("a", "X & Y", &[("X", Some("UCx")), (" & ", None), ("Y", Some("UCy"))]),
                    2,
                    400,
                    50,
                ),
                (song("b", "X", &[("X", Some("UCx"))]), 5, 100, 100),
                (song("LOCAL:/m/c.mp3", "x feat. Z", &[]), 1, 50, 150),
                (song("LOCAL:/m/d.mp3", local::UNKNOWN_ARTIST, &[]), 1, 999, 150),
                ("not a song".into(), 9, 9, 150),
            ],
            ..Default::default()
        };
        let s = shape(raw, false);
        assert_eq!((s.plays, s.secs, s.songs), (9, 1549, 4), "the unreadable row is skipped");
        assert_eq!(s.new_songs, 3, "first played at or after `since`");
        let x = &s.top_artists[0];
        assert_eq!((x.name.as_str(), x.id.as_deref()), ("X", Some("UCx")));
        assert_eq!((x.plays, x.secs), (8, 550), "a, b and the file (same name, any case)");
        assert_eq!(s.top_artists[1].name, "Y");
        assert_eq!(s.artists, 2, "\"Unknown artist\" is nobody");
        assert_eq!(s.top_songs[0].song.video_id, "b", "songs rank by plays");
        assert_eq!(s.top_albums[0].secs, 500, "only the two linked songs: files have no album id");
        assert_eq!(s.prev_secs, Some(0));
        assert_eq!(shape(Listening::default(), true).prev_secs, None);
    }

    /// Home's posters (#497) rank by plays and carry the stats page's counts, collabs included.
    #[test]
    fn familiar_artists_rank_by_plays() {
        let songs = vec![
            (
                song("a", "X & Y", &[("X", Some("UCx")), (" & ", None), ("Y", Some("UCy"))]),
                2,
                900,
                0,
            ),
            (song("b", "Y", &[("Y", Some("UCy"))]), 5, 100, 0),
            ("not a song".into(), 9, 9, 0),
        ];
        let top: Vec<_> =
            top_artists(songs.clone(), 5).into_iter().map(|a| (a.name, a.plays)).collect();
        assert_eq!(top, [("Y".to_owned(), 7), ("X".to_owned(), 2)], "X has the time, Y the plays");
        assert_eq!(top_artists(songs, 1).len(), 1, "capped at n");

        let renamed = vec![
            (song("c", "Ye", &[("Ye", Some("UCk"))]), 3, 30, 0),
            (song("d", "Kanye West", &[("Kanye West", Some("UCk"))]), 2, 20, 0),
        ];
        let top = top_artists(renamed, 5);
        assert_eq!((top.len(), top[0].plays), (1, 5), "one channel under two names is one artist");
    }

    /// A streamer's VODs and a podcast episode are listening time, not songs or artists.
    #[test]
    fn a_long_video_is_time_listened_but_not_a_song() {
        let item = |id: &str, artist: &str, video: bool, duration: &str| {
            serde_json::to_string(&SongItem {
                video_id: id.into(),
                artists: artist.into(),
                artist_runs: vec![ArtistRun { text: artist.into(), id: Some(format!("UC{id}")) }],
                is_video: video,
                duration: Some(duration.into()),
                ..Default::default()
            })
            .unwrap()
        };
        let raw = Listening {
            songs: vec![
                (item("vod", "erobb221 unmuted VODs", true, "3:12:40"), 3, 30_000, 0),
                (item("pod", "Sep 14", true, "58:02"), 1, 3_000, 0),
                (item("mv", "Band", true, "4:31"), 2, 500, 0),
                (item("closer", "Prog", false, "21:40"), 1, 1_300, 0),
            ],
            ..Default::default()
        };
        let s = shape(raw, false);
        assert_eq!((s.plays, s.secs), (7, 34_800), "every second still counts");
        let names: Vec<_> = s.top_artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Prog", "Band"], "a music video and a long audio track do count");
        assert_eq!((s.songs, s.artists, s.top_songs.len()), (2, 2, 2));
    }

    #[test]
    fn a_streak_lives_until_a_whole_day_is_missed() {
        assert_eq!(streaks(&[], 100), (0, 0));
        assert_eq!(streaks(&[1, 2, 3, 10, 11], 11), (2, 3), "today counts");
        assert_eq!(streaks(&[1, 2, 3, 10, 11], 12), (2, 3), "today hasn't been played yet");
        assert_eq!(streaks(&[1, 2, 3, 10, 11], 13), (0, 3), "yesterday was missed");
    }

    #[test]
    fn hours_land_in_their_weekday_row() {
        let s = shape(
            Listening { hours: vec![(0, 0, 5), (6, 23, 7), (9, 0, 1)], ..Default::default() },
            true,
        );
        assert_eq!((s.hours[0], s.hours[6 * 24 + 23], s.hours.len()), (5, 7, 168));
    }
}
