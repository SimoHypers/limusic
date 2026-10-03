use super::ImportTrack;

pub fn parse_csv(content: &str) -> Result<Vec<ImportTrack>, String> {
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(content.as_bytes());

    let headers = reader.headers().map_err(|e| format!("Failed to read CSV headers: {e}"))?;

    let mut title_idx = None;
    let mut artists_idx = None;
    let mut album_idx = None;
    let mut duration_ms_idx = None;
    let mut duration_text_idx = None;
    let mut isrc_idx = None;

    for (i, header) in headers.iter().enumerate() {
        let h = header.trim().to_lowercase();
        match h.as_str() {
            "track name" | "title" | "name" | "song" => title_idx = Some(i),
            "artist name(s)" | "artist name" | "artists" | "artist" => artists_idx = Some(i),
            "album name" | "album" => album_idx = Some(i),
            "duration (ms)" | "duration_ms" | "duration ms" => duration_ms_idx = Some(i),
            "duration" => duration_text_idx = Some(i),
            "isrc" => isrc_idx = Some(i),
            _ => {}
        }
    }

    let title_idx = title_idx.ok_or_else(|| "Missing title column".to_string())?;

    let mut tracks = Vec::new();

    for result in reader.records() {
        if tracks.len() >= 5000 {
            break;
        }
        let record = match result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let title_val = record.get(title_idx).unwrap_or("").trim();
        if title_val.is_empty() {
            continue;
        }

        let artists = if let Some(idx) = artists_idx {
            let val = record.get(idx).unwrap_or("").trim();
            if val.is_empty() {
                Vec::new()
            } else if val.contains(';') {
                val.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
            } else {
                vec![val.to_string()]
            }
        } else {
            Vec::new()
        };

        let album = album_idx
            .and_then(|idx| record.get(idx))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let duration_secs = {
            let mut dur = None;
            if let Some(idx) = duration_ms_idx {
                if let Some(ms_str) = record.get(idx).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ms) = ms_str.parse::<u64>() {
                        dur = Some((ms / 1000) as u32);
                    }
                }
            }
            if dur.is_none() {
                if let Some(idx) = duration_text_idx {
                    if let Some(text) = record.get(idx).map(|s| s.trim()).filter(|s| !s.is_empty())
                    {
                        dur = parse_duration_text(text);
                    }
                }
            }
            dur
        };

        let isrc = isrc_idx
            .and_then(|idx| record.get(idx))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from);

        tracks.push(ImportTrack {
            title: title_val.to_string(),
            artists,
            album,
            duration_secs,
            isrc,
        });
    }

    if tracks.is_empty() {
        return Err("No tracks found in CSV".to_string());
    }

    Ok(tracks)
}

fn parse_duration_text(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.split(':').collect();
    match parts.len() {
        1 => parts[0].trim().parse::<u32>().ok(),
        2 => {
            let m = parts[0].trim().parse::<u32>().ok()?;
            let s = parts[1].trim().parse::<u32>().ok()?;
            m.checked_mul(60)?.checked_add(s)
        }
        3 => {
            let h = parts[0].trim().parse::<u32>().ok()?;
            let m = parts[1].trim().parse::<u32>().ok()?;
            let s = parts[2].trim().parse::<u32>().ok()?;
            h.checked_mul(3600)?.checked_add(m.checked_mul(60)?)?.checked_add(s)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_csv_exportify_style() {
        let csv = "Track Name,Artist Name(s),Album Name,Duration (ms),ISRC\n\"Hello, World\",Artist One; Artist Two,Album Alpha,201500,USUM71703649";
        let res = parse_csv(csv).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Hello, World");
        assert_eq!(res[0].artists, vec!["Artist One", "Artist Two"]);
        assert_eq!(res[0].album.as_deref(), Some("Album Alpha"));
        assert_eq!(res[0].duration_secs, Some(201));
        assert_eq!(res[0].isrc.as_deref(), Some("USUM71703649"));
    }

    #[test]
    fn test_parse_csv_duration_text_and_bom() {
        let csv = "\u{FEFF}title,artist,duration\nSong Name,Artist Name,3:21";
        let res = parse_csv(csv).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Song Name");
        assert_eq!(res[0].artists, vec!["Artist Name"]);
        assert_eq!(res[0].duration_secs, Some(201));
        assert!(res[0].album.is_none());
        assert!(res[0].isrc.is_none());
    }

    #[test]
    fn test_parse_csv_missing_optional_columns() {
        let csv = "name\nOnly Title Here";
        let res = parse_csv(csv).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Only Title Here");
        assert!(res[0].artists.is_empty());
        assert!(res[0].album.is_none());
        assert!(res[0].duration_secs.is_none());
        assert!(res[0].isrc.is_none());
    }

    #[test]
    fn test_parse_csv_missing_title_column() {
        let csv = "artist,album\nSome Artist,Some Album";
        let res = parse_csv(csv);
        assert!(res.is_err());
    }

    #[test]
    fn test_parse_duration_three_parts() {
        assert_eq!(parse_duration_text("1:02:03"), Some(3723));
        assert_eq!(parse_duration_text("3:21"), Some(201));
        assert_eq!(parse_duration_text("42"), Some(42));
    }

    #[test]
    fn test_parse_duration_overflow() {
        assert_eq!(parse_duration_text("99999999:00"), None);
        assert_eq!(parse_duration_text("4294967295:59"), None);
    }
}
