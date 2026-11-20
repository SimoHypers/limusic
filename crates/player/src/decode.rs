use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use libmpv2::events::{Event, EventContext};
use libmpv2::Mpv;

/// Decodes any audio file or URL to a WAV file using a separate temporary libmpv instance.
///
/// A separate instance is used because configuring mpv for PCM output (`ao=pcm`) alters
/// global audio parameters and output redirection for that instance. Isolating this in a
/// dedicated `Mpv` object prevents any interference with the player's active playback decks.
pub fn decode_to_wav(
    input: &str,
    out_wav: &Path,
    headers: &std::collections::HashMap<String, String>,
    cancel: &AtomicBool,
    // Stop decoding after this many seconds (mpv's `end`), so a track too long to process never
    // fills the disk with a multi-GB WAV first. `None` decodes everything.
    max_secs: Option<f64>,
) -> anyhow::Result<()> {
    let mpv = Mpv::new().map_err(|e| anyhow::anyhow!("{}", e))?;

    if let Some(ua) = headers.get("User-Agent").or_else(|| headers.get("user-agent")) {
        mpv.set_property("user-agent", ua.as_str()).map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    let fields: String = headers
        .iter()
        .filter(|(k, _)| {
            !k.eq_ignore_ascii_case("user-agent") && !k.eq_ignore_ascii_case("http-proxy")
        })
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(",");
    if !fields.is_empty() {
        mpv.set_property("http-header-fields", fields.as_str())
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    if let Some(proxy) = headers.get("http-proxy").or_else(|| headers.get("HTTP-PROXY")) {
        mpv.set_property("http-proxy", proxy.as_str()).map_err(|e| anyhow::anyhow!("{}", e))?;
    }

    mpv.set_property("vid", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("ao", "pcm").map_err(|e| anyhow::anyhow!("{}", e))?;
    let path_str = out_wav.to_str().ok_or_else(|| anyhow::anyhow!("invalid output path"))?;
    mpv.set_property("ao-pcm-file", path_str).map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("ao-pcm-waveheader", "yes").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("audio-samplerate", 44100_i64).map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("audio-channels", "stereo").map_err(|e| anyhow::anyhow!("{}", e))?;
    if mpv.set_property("audio-format", "floatp").is_err() {
        mpv.set_property("audio-format", "float").map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    mpv.set_property("untimed", "yes").map_err(|e| anyhow::anyhow!("{}", e))?;
    if let Some(secs) = max_secs {
        mpv.set_property("end", format!("{secs:.0}").as_str())
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    mpv.set_property("idle", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("cache", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("audio-display", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("terminal", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("config", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("load-scripts", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("ytdl", "no").map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("volume", 100.0).map_err(|e| anyhow::anyhow!("{}", e))?;
    mpv.set_property("af", "").map_err(|e| anyhow::anyhow!("{}", e))?;

    mpv.command("loadfile", &[&super::quoted(input), "replace"])
        .map_err(|e| anyhow::anyhow!("{}", super::friendly_error(&e)))?;

    let mut ev = EventContext::new(mpv.ctx);
    ev.disable_deprecated_events().ok();

    let mut result = Ok(());
    loop {
        if cancel.load(Ordering::Relaxed) {
            result = Err(anyhow::anyhow!("cancelled"));
            break;
        }
        match ev.wait_event(0.1) {
            Some(Ok(Event::EndFile(reason))) => {
                match reason as i32 {
                    0 => break, // EOF
                    other if other == 4 => {
                        result =
                            Err(anyhow::anyhow!("mpv decoding failed with error reason {other}"));
                        break;
                    }
                    other => {
                        if other != 0 {
                            result = Err(anyhow::anyhow!("mpv decoding ended with reason {other}"));
                        }
                        break;
                    }
                }
            }
            Some(Err(e)) => {
                result = Err(anyhow::anyhow!("{}", super::friendly_error(&e)));
                break;
            }
            _ => {}
        }
    }

    drop(mpv); // finalize WAV file header before returning

    if cancel.load(Ordering::Relaxed) || result.is_err() {
        let _ = std::fs::remove_file(out_wav);
        if cancel.load(Ordering::Relaxed) && result.is_ok() {
            return Err(anyhow::anyhow!("cancelled"));
        }
    }

    result
}
