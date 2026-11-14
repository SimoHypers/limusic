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
    // tauri-build watches tauri.conf.json but not the icons it embeds, so editing a PNG here
    // leaves `generate_context!` emitting the old `default_window_icon` (window, tray, taskbar).
    println!("cargo:rerun-if-changed=icons");

    // Monterey may keep an older WebKit in /System even after Safari updates. Like Safari's
    // own executable, prefer Apple's newer staged frameworks (dyld compares versions and
    // otherwise keeps the system copy). The older engine rejects decimal OKLCH/color-mix,
    // leaving theme backgrounds transparent. No bundled runtime or higher OS target needed.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for kind in ["FRAMEWORK", "LIBRARY"] {
            println!(
                "cargo:rustc-link-arg-bins=-Wl,-dyld_env,DYLD_VERSIONED_{kind}_PATH=/Library/Apple/System/Library/StagedFrameworks/Safari"
            );
        }
    }

    tauri_build::build()
}
