//! The downloads contract at the serialization boundary.
//!
//! The UI worker builds against these names; a rename here is a silent break, so it is pinned.

use app_lib::DownloadItem;

#[test]
fn download_item_serializes_with_the_contract_keys_and_nulls() {
    let item = DownloadItem {
        id: "dQw4w9WgXcQ".into(),
        video_id: "dQw4w9WgXcQ".into(),
        title: "Never Gonna Give You Up".into(),
        artists: "Rick Astley".into(),
        album: None,
        thumbnail: None,
        state: "queued".into(),
        itag: None,
        mime: None,
        quality_label: "HIGH".into(),
        path: None,
        bytes_done: 0,
        bytes_total: None,
        error: None,
        added_at: 1_700_000_000,
        updated_at: 1_700_000_001,
    };
    let value = serde_json::to_value(&item).unwrap();
    let object = value.as_object().unwrap();

    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "added_at",
            "album",
            "artists",
            "bytes_done",
            "bytes_total",
            "error",
            "id",
            "itag",
            "mime",
            "path",
            "quality_label",
            "state",
            "thumbnail",
            "title",
            "updated_at",
            "video_id"
        ],
        "the DownloadItem field names are the contract"
    );
    for nullable in ["album", "itag", "mime", "path", "bytes_total", "error"] {
        assert!(object[nullable].is_null(), "{nullable} must serialize as null, not be dropped");
    }
    assert_eq!(object["state"], "queued");
    assert_eq!(object["quality_label"], "HIGH");
}

#[test]
fn a_completed_item_carries_its_path_and_sizes() {
    let item = DownloadItem {
        id: "abc123".into(),
        video_id: "abc123".into(),
        title: "Song".into(),
        artists: "Artist".into(),
        album: Some("Album".into()),
        thumbnail: Some("https://example.invalid/t.jpg".into()),
        state: "done".into(),
        itag: Some(251),
        mime: Some("audio/webm; codecs=opus".into()),
        quality_label: "160 kbps Opus".into(),
        path: Some("/data/downloads/Song [abc123].webm".into()),
        bytes_done: 4096,
        bytes_total: Some(4096),
        error: None,
        added_at: 1,
        updated_at: 2,
    };
    let value = serde_json::to_value(&item).unwrap();
    assert_eq!(value["itag"], 251);
    assert_eq!(value["bytes_total"], 4096);
    assert_eq!(value["path"], "/data/downloads/Song [abc123].webm");
}
