use super::{Candidate, ImportTrack, MatchStatus};
use innertube::SongItem;
use regex::Regex;
use std::collections::HashSet;

const WEIGHT_TITLE: f32 = 0.5;
const WEIGHT_ARTIST: f32 = 0.3;
const WEIGHT_DURATION: f32 = 0.2;

const MATCH_THRESHOLD: f32 = 0.80;
const REVIEW_THRESHOLD: f32 = 0.45;

const DURATION_EXACT_SECS: i32 = 2;
const DURATION_MAX_SECS: i32 = 15;
const DURATION_MATCH_SECS: i32 = 5;

const VARIANT_PENALTY: f32 = 0.25;
const MAX_VARIANT_PENALTY: f32 = 0.5;
const MIN_SCORE_GAP: f32 = 0.05;
const MAX_CANDIDATES: usize = 5;

const VARIANT_TAGS: &[&str] = &[
    "live",
    "remix",
    "cover",
    "karaoke",
    "instrumental",
    "acoustic",
    "slowed",
    "sped up",
    "nightcore",
    "reverb",
];

const REMASTER_SUFFIXES: &[&str] =
    &["remaster", "radio edit", "single version", "mono", "stereo", "deluxe"];

pub fn normalize(s: &str) -> String {
    let s_lower = s.to_lowercase();

    let mut depth = 0u32;
    let mut unbracketed = String::new();
    for c in s_lower.chars() {
        match c {
            '(' | '[' | '{' | '<' | '【' | '（' => depth += 1,
            ')' | ']' | '}' | '>' | '】' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => unbracketed.push(c),
            _ => {}
        }
    }

    let parts: Vec<&str> = unbracketed.split(" - ").collect();
    let mut base = parts.first().copied().unwrap_or(&unbracketed).to_string();
    if parts.len() > 1 {
        let mut kept = vec![parts[0]];
        for part in &parts[1..] {
            let p_trimmed = part.trim();
            let is_remaster_suffix = REMASTER_SUFFIXES.iter().any(|&s| p_trimmed.contains(s));
            if !is_remaster_suffix {
                kept.push(part);
            }
        }
        base = kept.join(" - ");
    }

    let cleaned: String = base.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();

    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn variant_tags(s: &str) -> Vec<&'static str> {
    let s_lower = s.to_lowercase();
    let mut found = Vec::new();
    for &tag in VARIANT_TAGS {
        let matched = if tag == "sped up" {
            s_lower.contains("sped up") || s_lower.contains("sped-up") || s_lower.contains("spedup")
        } else {
            let pattern = format!(r"\b{}\b", regex::escape(tag));
            if let Ok(re) = Regex::new(&pattern) {
                re.is_match(&s_lower)
            } else {
                s_lower.contains(tag)
            }
        };
        if matched && !found.contains(&tag) {
            found.push(tag);
        }
    }
    found
}

pub fn score_candidate(src: &ImportTrack, cand: &SongItem) -> Candidate {
    let src_title_norm = normalize(&src.title);
    let cand_title_norm = normalize(&cand.title);

    let title_sim = if src_title_norm == cand_title_norm && !src_title_norm.is_empty() {
        1.0
    } else {
        let src_tokens: HashSet<&str> = src_title_norm.split_whitespace().collect();
        let cand_tokens: HashSet<&str> = cand_title_norm.split_whitespace().collect();
        if src_tokens.is_empty() && cand_tokens.is_empty() {
            1.0
        } else if src_tokens.is_empty() || cand_tokens.is_empty() {
            0.0
        } else {
            let intersection = src_tokens.intersection(&cand_tokens).count() as f32;
            let union = src_tokens.union(&cand_tokens).count() as f32;
            if union > 0.0 {
                intersection / union
            } else {
                0.0
            }
        }
    };

    let artist_sim = if src.artists.is_empty() {
        1.0
    } else {
        let cand_artists_norm = normalize(&cand.artists);
        let cand_artist_tokens: HashSet<&str> = cand_artists_norm.split_whitespace().collect();
        let mut found_count = 0;
        for sa in &src.artists {
            let sa_norm = normalize(sa);
            let sa_tokens: HashSet<&str> = sa_norm.split_whitespace().collect();
            if !sa_tokens.is_empty() && sa_tokens.is_subset(&cand_artist_tokens) {
                found_count += 1;
            } else if sa_tokens.is_empty() {
                found_count += 1;
            }
        }
        (found_count as f32) / (src.artists.len() as f32)
    };

    let src_secs = src.duration_secs;
    let cand_secs = cand
        .duration
        .as_deref()
        .and_then(crate::lyrics::duration_str_secs)
        .map(|d| d.round() as i32);

    let (duration_score, duration_diff_secs, has_duration) = match (src_secs, cand_secs) {
        (Some(s), Some(c)) => {
            let diff = c - s as i32;
            let diff_abs = diff.abs();
            let score = if diff_abs <= DURATION_EXACT_SECS {
                1.0
            } else if diff_abs <= DURATION_MAX_SECS {
                1.0 - (diff_abs - DURATION_EXACT_SECS) as f32
                    / (DURATION_MAX_SECS - DURATION_EXACT_SECS) as f32
            } else {
                0.0
            };
            (Some(score), Some(diff), true)
        }
        _ => (None, None, false),
    };

    let final_score = match (duration_score, has_duration) {
        (Some(d_score), true) => {
            WEIGHT_TITLE * title_sim + WEIGHT_ARTIST * artist_sim + WEIGHT_DURATION * d_score
        }
        _ => {
            let sum_weights = WEIGHT_TITLE + WEIGHT_ARTIST;
            if sum_weights > 0.0 {
                (WEIGHT_TITLE * title_sim + WEIGHT_ARTIST * artist_sim) / sum_weights
            } else {
                0.0
            }
        }
    };

    let src_tags = variant_tags(&src.title);
    let cand_tags = variant_tags(&cand.title);
    let extra_tags = cand_tags.iter().filter(|t| !src_tags.contains(t)).count();
    let penalty = (extra_tags as f32 * VARIANT_PENALTY).min(MAX_VARIANT_PENALTY);

    let clamped_score = (final_score - penalty).clamp(0.0, 1.0);

    Candidate { song: cand.clone(), score: clamped_score, duration_diff_secs }
}

pub fn classify(cands: &[Candidate], _src: &ImportTrack) -> MatchStatus {
    let Some(best) = cands.first() else {
        return MatchStatus::NotFound;
    };

    let best_score = best.score;
    let second_score = cands.get(1).map_or(0.0, |c| c.score);
    let diff_beats_second = (best_score - second_score) >= MIN_SCORE_GAP;
    let duration_diff_small =
        best.duration_diff_secs.map_or(true, |d| d.abs() <= DURATION_EXACT_SECS);
    let duration_diff_moderate =
        best.duration_diff_secs.map_or(true, |d| d.abs() <= DURATION_MATCH_SECS);

    if best_score >= MATCH_THRESHOLD
        && duration_diff_moderate
        && (diff_beats_second || duration_diff_small)
    {
        MatchStatus::Matched
    } else if best_score >= REVIEW_THRESHOLD {
        MatchStatus::Review
    } else {
        MatchStatus::NotFound
    }
}

pub fn rank(src: &ImportTrack, items: Vec<SongItem>) -> Vec<Candidate> {
    let mut cands: Vec<Candidate> = items.iter().map(|item| score_candidate(src, item)).collect();
    cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    cands.truncate(MAX_CANDIDATES);
    cands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(title: &str, artists: Vec<&str>, duration_secs: Option<u32>) -> ImportTrack {
        ImportTrack {
            title: title.into(),
            artists: artists.into_iter().map(Into::into).collect(),
            album: None,
            duration_secs,
            isrc: None,
        }
    }

    fn make_song(title: &str, artists: &str, duration: Option<&str>) -> SongItem {
        SongItem {
            video_id: "test_id".into(),
            title: title.into(),
            artists: artists.into(),
            duration: duration.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Song Name (feat. Artist X)"), "song name");
        assert_eq!(normalize("Song Title [Remastered 2011]"), "song title");
        assert_eq!(normalize("My Song - Remastered 2011"), "my song");
        assert_eq!(normalize("Привет Мир"), "привет мир");
        assert_eq!(normalize("Hello World 123!"), "hello world 123");
    }

    #[test]
    fn test_variant_tags() {
        assert_eq!(variant_tags("Live in Tokyo (Remix)"), vec!["live", "remix"]);
        assert_eq!(variant_tags("Normal Song"), Vec::<&str>::new());
        assert_eq!(variant_tags("Alive and Well"), Vec::<&str>::new()); // should not match "live" inside "alive"
    }

    #[test]
    fn test_same_song_different_duration_live_loses() {
        let src = make_track("Song", vec!["Artist"], Some(200));
        let normal = make_song("Song", "Artist", Some("3:20")); // 200s
        let live = make_song("Song (Live)", "Artist", Some("4:03")); // 243s (+43s)

        let ranked = rank(&src, vec![live, normal]);
        assert_eq!(ranked[0].song.title, "Song");
        assert_eq!(classify(&ranked, &src), MatchStatus::Matched);
    }

    #[test]
    fn test_remaster_matches() {
        let src = make_track("Bohemian Rhapsody", vec!["Queen"], Some(354));
        let cand = make_song("Bohemian Rhapsody - Remastered 2011", "Queen", Some("5:54")); // 354s

        let ranked = rank(&src, vec![cand]);
        assert_eq!(ranked[0].score, 1.0);
        assert_eq!(classify(&ranked, &src), MatchStatus::Matched);
    }

    #[test]
    fn test_feat_in_brackets_ignored() {
        let src = make_track("Song", vec!["Artist A"], Some(180));
        let cand = make_song("Song (feat. Artist B)", "Artist A, Artist B", Some("3:00"));

        let ranked = rank(&src, vec![cand]);
        assert!(ranked[0].score >= 0.8);
        assert_eq!(classify(&ranked, &src), MatchStatus::Matched);
    }

    #[test]
    fn test_cyrillic_title_matches() {
        let src = make_track("Кукушка", vec!["Кино"], Some(300));
        let cand = make_song("Кукушка", "Кино", Some("5:00"));

        let ranked = rank(&src, vec![cand]);
        assert_eq!(ranked[0].score, 1.0);
        assert_eq!(classify(&ranked, &src), MatchStatus::Matched);
    }

    #[test]
    fn test_unrelated_song_not_found() {
        let src = make_track("Good Song", vec!["Good Artist"], Some(200));
        let cand = make_song("Totally Different Track", "Other Artist", Some("4:15"));

        let ranked = rank(&src, vec![cand]);
        assert_eq!(classify(&ranked, &src), MatchStatus::NotFound);
    }

    #[test]
    fn test_empty_candidate_list() {
        let src = make_track("Song", vec!["Artist"], Some(200));
        let ranked = rank(&src, vec![]);
        assert!(ranked.is_empty());
        assert_eq!(classify(&ranked, &src), MatchStatus::NotFound);
    }
}
