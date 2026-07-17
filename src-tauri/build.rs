fn main() {
  // Last.fm API credentials live in a gitignored `lastfm.keys` next to this file (the repo is
  // public — they must never be tracked). Format: `LIMUSIC_LASTFM_API_KEY=…` and
  // `LIMUSIC_LASTFM_API_SECRET=…`, one per line. Exported as compile-time env for `option_env!`
  // in lastfm.rs; missing file just means the scrobbler reports "not configured".
  println!("cargo:rerun-if-changed=lastfm.keys");
  if let Ok(keys) = std::fs::read_to_string("lastfm.keys") {
    for line in keys.lines() {
      if let Some((k, v)) = line.split_once('=') {
        let (k, v) = (k.trim(), v.trim());
        if k == "LIMUSIC_LASTFM_API_KEY" || k == "LIMUSIC_LASTFM_API_SECRET" {
          println!("cargo:rustc-env={k}={v}");
        }
      }
    }
  }
  tauri_build::build()
}
