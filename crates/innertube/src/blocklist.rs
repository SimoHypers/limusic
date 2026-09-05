//! The user's blocked-artist list, applied at the fetch boundary next to "hide music videos".
//!
//! Lives in this crate because the filter runs where responses are parsed (`endpoints.rs`), the
//! same place and for the same reason as `hide_videos`. The crate holds the predicate; the app
//! owns the data and pushes it in with `InnerTube::set_blocked`.

use std::collections::HashSet;

use crate::models::browse::BrowseItem;
use crate::models::metadata::SongItem;

/// Channel ids and artist names the user never wants recommended.
///
/// Both keys, because neither is sufficient on its own: the channel id survives a rename, and the
/// name survives a re-upload under a fresh channel, which is exactly what the AI-slop channels the
/// feature exists for keep doing.
#[derive(Debug, Default, Clone)]
pub struct BlockList {
    ids: HashSet<String>,
    /// Normalized with [`norm`] on the way in, so lookups can be plain `contains`.
    names: HashSet<String>,
}

/// Trimmed and lowercased. Nothing cleverer: a fuzzy match here is a false positive on a real
/// artist, which is a worse bug than a leak.
fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

impl BlockList {
    pub fn new(entries: impl IntoIterator<Item = (Option<String>, String)>) -> Self {
        let mut bl = BlockList::default();
        for (id, name) in entries {
            if let Some(id) = id.filter(|i| !i.is_empty()) {
                bl.ids.insert(id);
            }
            let name = norm(&name);
            if !name.is_empty() {
                bl.names.insert(name);
            }
        }
        bl
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.names.is_empty()
    }

    /// ponytail: an unlinked collab byline ("Foo & Bar" as one run with no channel) only matches
    /// when the whole string matches, so blocking "Bar" alone misses it. Splitting on separators
    /// would over-match real names; revisit only if that leak is actually reported.
    pub fn blocks_song(&self, s: &SongItem) -> bool {
        if self.is_empty() {
            return false;
        }
        if s.artist_id.as_deref().is_some_and(|id| self.ids.contains(id)) {
            return true;
        }
        if s.artist_runs.iter().any(|r| self.matches_run(r.id.as_deref(), &r.text)) {
            return true;
        }
        s.artist_runs.is_empty() && self.names.contains(&norm(&s.artists))
    }

    /// Cards carry less. An artist card is identified by its own browseId; a song card by its
    /// `artist_runs`. `subtitle` is deliberately not matched: on album and playlist cards it is a
    /// composed line ("Album • Foo • 2024"), and substring-matching it is how you make unrelated
    /// records disappear.
    pub fn blocks_card(&self, c: &BrowseItem) -> bool {
        if self.is_empty() {
            return false;
        }
        if c.kind == "artist" && (self.ids.contains(&c.id) || self.names.contains(&norm(&c.title)))
        {
            return true;
        }
        c.artist_runs.iter().any(|r| self.matches_run(r.id.as_deref(), &r.text))
    }

    fn matches_run(&self, id: Option<&str>, text: &str) -> bool {
        id.is_some_and(|i| self.ids.contains(i)) || self.names.contains(&norm(text))
    }
}

/// Drop blocked songs, always keeping `keep` (the requested video: a radio must still open on the
/// song you started it from, and the track already playing is never yanked out from under you).
pub fn retain_songs(items: &mut Vec<SongItem>, bl: &BlockList, keep: Option<&str>) {
    if bl.is_empty() {
        return;
    }
    items.retain(|i| Some(i.video_id.as_str()) == keep || !bl.blocks_song(i));
}

pub fn retain_cards(items: &mut Vec<BrowseItem>, bl: &BlockList) {
    if bl.is_empty() {
        return;
    }
    items.retain(|i| !bl.blocks_card(i));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::metadata::ArtistRun;

    fn list(entries: &[(Option<&str>, &str)]) -> BlockList {
        BlockList::new(entries.iter().map(|(id, name)| (id.map(str::to_owned), (*name).to_owned())))
    }

    fn run(text: &str, id: Option<&str>) -> ArtistRun {
        ArtistRun { text: text.into(), id: id.map(str::to_owned) }
    }

    fn song(video_id: &str, artists: &str, runs: Vec<ArtistRun>) -> SongItem {
        SongItem {
            video_id: video_id.into(),
            title: "t".into(),
            artists: artists.into(),
            artist_id: runs.iter().find_map(|r| r.id.clone()),
            artist_runs: runs,
            ..Default::default()
        }
    }

    fn card(kind: &'static str, id: &str, title: &str, runs: Vec<ArtistRun>) -> BrowseItem {
        BrowseItem {
            kind,
            id: id.into(),
            title: title.into(),
            subtitle: None,
            thumbnail: None,
            duration: None,
            artist_runs: runs,
            play_count: None,
            is_video: false,
            is_upload: false,
            explicit: false,
        }
    }

    #[test]
    fn an_empty_list_blocks_nothing() {
        let bl = BlockList::default();
        assert!(bl.is_empty());
        assert!(!bl.blocks_song(&song("v", "Foo", vec![run("Foo", Some("UCfoo"))])));
        assert!(!bl.blocks_card(&card("artist", "UCfoo", "Foo", vec![])));
    }

    #[test]
    fn a_channel_id_matches_through_the_runs() {
        let bl = list(&[(Some("UCfoo"), "Whatever the name was then")]);
        // Renamed since it was blocked: the id is what still matches.
        assert!(bl.blocks_song(&song("v", "New Name", vec![run("New Name", Some("UCfoo"))])));
    }

    #[test]
    fn a_name_matches_regardless_of_case_and_padding() {
        let bl = list(&[(None, "  FOO Bar ")]);
        assert!(bl.blocks_song(&song("v", "Foo Bar", vec![run("foo bar", None)])));
    }

    #[test]
    fn a_collab_is_blocked_when_any_run_is() {
        let bl = list(&[(Some("UCbar"), "Bar")]);
        let s = song(
            "v",
            "Foo & Bar",
            vec![run("Foo", Some("UCfoo")), run(" & ", None), run("Bar", Some("UCbar"))],
        );
        assert!(bl.blocks_song(&s));
    }

    #[test]
    fn a_row_with_no_runs_matches_on_the_whole_artists_string() {
        let bl = list(&[(None, "Foo")]);
        assert!(bl.blocks_song(&song("v", "Foo", vec![])));
        // Whole string only: an unlinked collab does not match a single blocked name.
        assert!(!bl.blocks_song(&song("v", "Foo & Bar", vec![])));
    }

    #[test]
    fn the_artists_string_is_only_a_fallback() {
        let bl = list(&[(None, "Foo")]);
        // The row links its artists, none of them blocked. The plain string happening to read
        // "Foo" must not resurrect the fallback.
        let s = song("v", "Foo", vec![run("Bar", Some("UCbar"))]);
        assert!(!bl.blocks_song(&s));
    }

    #[test]
    fn retain_songs_keeps_the_seed_even_when_it_is_blocked() {
        let bl = list(&[(Some("UCfoo"), "Foo")]);
        let mut items = vec![
            song("seed", "Foo", vec![run("Foo", Some("UCfoo"))]),
            song("other", "Foo", vec![run("Foo", Some("UCfoo"))]),
            song("keeper", "Bar", vec![run("Bar", Some("UCbar"))]),
        ];
        retain_songs(&mut items, &bl, Some("seed"));
        assert_eq!(
            items.iter().map(|i| i.video_id.as_str()).collect::<Vec<_>>(),
            ["seed", "keeper"]
        );
    }

    #[test]
    fn cards_match_by_artist_browse_id_and_by_run() {
        let bl = list(&[(Some("UCfoo"), "Foo")]);
        assert!(bl.blocks_card(&card("artist", "UCfoo", "Renamed", vec![])));
        assert!(bl.blocks_card(&card("artist", "UCother", "  foo ", vec![])));
        assert!(bl.blocks_card(&card(
            "song",
            "vid",
            "Some track",
            vec![run("Foo", Some("UCfoo"))]
        )));
        // An album card whose subtitle links the artist is blocked through that run.
        assert!(bl.blocks_card(&card(
            "album",
            "MPREx",
            "Some Album",
            vec![run("Foo", Some("UCfoo"))]
        )));
        // One that links nobody survives: matching the composed `subtitle` text is what this
        // deliberately does not do.
        assert!(!bl.blocks_card(&card("album", "MPREy", "Foo", vec![])));
    }

    #[test]
    fn retaining_against_an_empty_list_changes_nothing() {
        let bl = BlockList::default();
        let mut items = vec![song("a", "Foo", vec![]), song("b", "Bar", vec![])];
        retain_songs(&mut items, &bl, None);
        assert_eq!(items.len(), 2);
        let mut cards = vec![card("artist", "UCfoo", "Foo", vec![])];
        retain_cards(&mut cards, &bl);
        assert_eq!(cards.len(), 1);
    }
}
