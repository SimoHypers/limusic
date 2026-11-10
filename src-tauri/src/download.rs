//! Audio downloads through the `yt-dlp` binary. The UI passes a track's identity and its chosen
//! container/quality; this module decides the filename (`Artist - Title.<ext>` under
//! `~/Downloads`, or `~/Downloads/<Playlist>/` for a playlist download), embeds the cover and the
//! tags, and reports where the file landed.
//!
//! The account's YouTube cookies are handed over as a private Netscape jar for each run: a signed-in
//! session is what stops YouTube from answering "Sign in to confirm you're not a bot".
//!
//! The argument builder, the cookie jar and the naming rules are pure and unit-tested (`mod tests`
//! below); only `run`/`version` touch the process.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Every progress line yt-dlp writes starts with this, so the final `--print` path and any other
/// chatter on stdout can be told apart without guessing.
const PROGRESS_PREFIX: &str = "LIMUSIC-PROGRESS ";

/// Containers yt-dlp can extract audio into here. `wav` is the one with no tag/cover container.
const FORMATS: [&str; 5] = ["mp3", "m4a", "flac", "opus", "wav"];
const QUALITIES: [&str; 5] = ["best", "320", "192", "128", "64"];
/// Longest filename stem we hand to yt-dlp, in bytes. Filesystems cap a component at 255 bytes and
/// the extension/template suffix needs headroom; cutting on a char boundary keeps UTF-8 titles valid.
const MAX_STEM_BYTES: usize = 180;

/// Windows refuses these as a whole name, with or without an extension.
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Everything one download needs. `playlist` only changes the folder the file lands in, never the
/// file name.
pub struct TrackDownload<'a> {
    pub video_id: &'a str,
    pub artist: &'a str,
    pub title: &'a str,
    pub format: &'a str,
    pub quality: &'a str,
    pub playlist: Option<&'a str>,
}

/// Where the file went, and whether it was already there (yt-dlp skips an existing file).
#[derive(Debug, PartialEq, Serialize)]
pub struct DownloadOutcome {
    pub path: String,
    pub skipped: bool,
}

/// Containers yt-dlp is allowed to extract into.
pub fn validate_format(format: &str) -> Result<(), String> {
    if FORMATS.contains(&format) {
        Ok(())
    } else {
        Err(format!("Unsupported format: {format}"))
    }
}

/// Bitrate hints accepted by the download command.
pub fn validate_quality(quality: &str) -> Result<(), String> {
    if QUALITIES.contains(&quality) {
        Ok(())
    } else {
        Err(format!("Unsupported quality: {quality}"))
    }
}

/// The id is interpolated into the URL, so anything that isn't the YouTube alphabet is refused
/// rather than handed to yt-dlp.
pub fn validate_video_id(video_id: &str) -> Result<(), String> {
    let ok = !video_id.is_empty()
        && video_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("Unsupported video id: {video_id}"))
    }
}

/// A filesystem-safe name component: separators and the characters Windows rejects become `_`,
/// control characters too, whitespace is collapsed, leading/trailing dots and spaces are dropped
/// (Windows can't create them), reserved device names get a prefix, and the result is capped on a
/// char boundary. Never empty.
pub fn sanitize_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_space = false;
    for c in raw.chars() {
        let c = match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        };
        if c.is_whitespace() {
            if last_space {
                continue;
            }
            out.push(' ');
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    let trimmed = out.trim_matches(|c: char| c.is_whitespace() || c == '.');
    let mut name = truncate_bytes(trimmed.to_owned(), MAX_STEM_BYTES);
    if name.is_empty() {
        return "Unknown".into();
    }
    let stem = name.split('.').next().unwrap_or(&name).to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    name
}

/// `Artist - Title`, falling back to whichever half exists so a nameless row still produces a file.
pub fn file_stem(artist: &str, title: &str) -> String {
    let artist = artist.trim();
    let title = title.trim();
    let stem = match (artist.is_empty(), title.is_empty()) {
        (false, false) => format!("{} - {}", sanitize_component(artist), sanitize_component(title)),
        (false, true) => sanitize_component(artist),
        (true, false) => sanitize_component(title),
        (true, true) => "Unknown track".to_owned(),
    };
    truncate_bytes(stem, MAX_STEM_BYTES)
}

/// yt-dlp reads `-o` as a template, so a literal `%` in the name has to be doubled.
pub fn escape_template(text: &str) -> String {
    text.replace('%', "%%")
}

/// The `-o` template yt-dlp expands with the real extension.
pub fn output_template(dir: &Path, stem: &str) -> String {
    dir.join(format!("{}.%(ext)s", escape_template(stem))).to_string_lossy().into_owned()
}

/// Where a finished download should be, for skip detection and the stdout fallback.
pub fn expected_path(dir: &Path, stem: &str, format: &str) -> String {
    dir.join(format!("{stem}.{format}")).to_string_lossy().into_owned()
}

/// Where a download would land, for the save dialog's suggested name.
pub fn suggested_path(
    dir: &Path,
    artist: &str,
    title: &str,
    format: &str,
    playlist: Option<&str>,
) -> String {
    let dir = match playlist {
        Some(name) if !name.trim().is_empty() => dir.join(sanitize_component(name)),
        _ => dir.to_path_buf(),
    };
    expected_path(&dir, &file_stem(artist, title), format)
}

/// yt-dlp wants a VBR level (0-10) or a bitrate like `320K` here; the UI's numbers are kbps and
/// `best` means "leave yt-dlp its default" rather than passing the word through to ffmpeg.
pub fn audio_quality(quality: &str) -> Option<String> {
    match quality {
        "320" | "192" | "128" | "64" => Some(format!("{quality}K")),
        _ => None,
    }
}

/// The app stores the account's session as a `Cookie:` header value; yt-dlp only takes a Netscape
/// jar. Every pair lands on `.youtube.com` (that is where YouTube sets them), secure, with a far
/// future expiry — yt-dlp ignores the expiry for sending, and the app keeps the real session fresh.
///
/// `None` when the header carries no usable pair (signed out), so yt-dlp is left to its own device.
pub fn cookie_jar_contents(header: &str) -> Option<String> {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Written by Limusic for yt-dlp.\n");
    let mut pairs = 0;
    for part in header.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        // A tab or newline would break the row; a comment-looking name would disappear into one.
        let unsafe_text = |text: &str| text.contains(['\t', '\r', '\n']);
        if name.is_empty()
            || value.is_empty()
            || name.starts_with('#')
            || name.chars().any(char::is_whitespace)
            || unsafe_text(value)
        {
            continue;
        }
        out.push_str(&format!(".youtube.com\tTRUE\t/\tTRUE\t2147483647\t{name}\t{value}\n"));
        pairs += 1;
    }
    (pairs > 0).then_some(out)
}

/// A cookie jar for one yt-dlp run: created `0600` and deleted when dropped (success or failure),
/// so the session never outlives the download in a readable temp file.
pub struct CookieJar(PathBuf);

impl CookieJar {
    /// The jar's path, for yt-dlp's `--cookies`.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for CookieJar {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes a `0600` jar for one download; `None` when the session carries no cookies.
pub fn write_cookie_jar(header: &str) -> Result<Option<CookieJar>, String> {
    let Some(contents) = cookie_jar_contents(header) else {
        return Ok(None);
    };
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path =
        std::env::temp_dir().join(format!("limusic-ytdlp-{}-{nonce}.cookies", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file =
        options.open(&path).map_err(|e| format!("Could not write the cookie jar: {e}"))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Could not write the cookie jar: {e}"))?;
    Ok(Some(CookieJar(path)))
}

/// How long a jar may linger before a startup considers it abandoned. Runs of another Limusic
/// instance (`LIMUSIC_MULTI`) write their jar at the start of each download, so anything this old is
/// not in use by a live download and is safe to remove.
const STALE_JAR_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// A crash or a force-quit skips `CookieJar::drop`, leaving a live session in the temp dir. Swept
/// once at startup, never per download: removing every matching jar mid-run would delete the cookie
/// file another concurrent download is about to hand to yt-dlp.
pub fn sweep_stale_jars() {
    let now = SystemTime::now();
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("limusic-ytdlp-") || !name.ends_with(".cookies") {
            continue;
        }
        let modified = entry.metadata().ok().and_then(|meta| meta.modified().ok());
        if is_stale_jar(modified, now) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A jar is abandoned once it is older than `STALE_JAR_AGE`.
fn is_stale_jar(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age > STALE_JAR_AGE)
}

/// The yt-dlp argv. `-x` extracts audio and re-encodes to `format`; ffmpeg is already required for
/// that, so the cover can be embedded too. The URL is last, and `--no-playlist` keeps a video id
/// that belongs to a playlist down to one file.
pub fn args(req: &TrackDownload<'_>, template: &str, cookies: Option<&Path>) -> Vec<String> {
    let mut args = vec!["--no-playlist".to_owned()];
    if let Some(cookies) = cookies {
        args.extend(["--cookies".to_owned(), cookies.to_string_lossy().into_owned()]);
    }
    args.extend(["-x".to_owned(), "--audio-format".to_owned(), req.format.to_owned()]);
    if let Some(quality) = audio_quality(req.quality) {
        args.extend(["--audio-quality".to_owned(), quality]);
    }
    args.push("--embed-metadata".to_owned());
    if req.format != "wav" {
        args.extend([
            "--embed-thumbnail".to_owned(),
            "--convert-thumbnails".to_owned(),
            "jpg".to_owned(),
            // Embedded only: no `Artist - Title.jpg` left beside the audio.
            "--no-write-thumbnail".to_owned(),
        ]);
    }
    args.extend([
        // One machine-readable line per progress tick, instead of the redrawn `[download]` bar.
        // `--print` implies `--quiet`, where yt-dlp suppresses progress unless `--progress` is
        // given; without it the template lines never arrive.
        "--progress".to_owned(),
        "--newline".to_owned(),
        "--progress-template".to_owned(),
        format!(
            "download:{PROGRESS_PREFIX}%(progress.downloaded_bytes)s \
             %(progress.total_bytes)s %(progress.total_bytes_estimate)s \
             %(progress.speed)s %(progress.eta)s"
        ),
        "--print".to_owned(),
        "after_move:filepath".to_owned(),
        "-o".to_owned(),
        template.to_owned(),
        format!("https://www.youtube.com/watch?v={}", req.video_id),
    ]);
    args
}

/// One progress tick for the UI. `total` is absent when YouTube does not say how big the stream is
/// (the bar then stays indeterminate); `percent` is 0 in that case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Progress {
    pub id: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: f64,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
}

pub type ProgressSink<'a> = &'a (dyn Fn(Progress) + Send + Sync);

/// `LIMUSIC-PROGRESS <downloaded> <total> <estimate> <speed> <eta>`; yt-dlp writes `NA` for a field
/// it does not know. Returns `(downloaded, total-or-estimate, speed, eta)`.
pub fn parse_progress(line: &str) -> Option<(u64, Option<u64>, Option<f64>, Option<u64>)> {
    let rest = line.trim().strip_prefix(PROGRESS_PREFIX)?;
    let mut fields = rest.split_whitespace();
    let downloaded = fields.next()?.parse::<u64>().ok()?;
    let total = fields.next().and_then(|v| v.parse::<u64>().ok());
    let estimate = fields.next().and_then(|v| v.parse::<u64>().ok());
    let speed = fields.next().and_then(|v| v.parse::<f64>().ok());
    let eta = fields.next().and_then(|v| v.parse::<u64>().ok());
    Some((downloaded, total.or(estimate), speed, eta))
}

/// 0-100 for a progress tick; 0 when yt-dlp does not know the size yet.
fn progress_percent(downloaded: u64, total: Option<u64>) -> f64 {
    match total {
        Some(total) if total > 0 => (downloaded as f64 / total as f64 * 100.0).clamp(0.0, 100.0),
        _ => 0.0,
    }
}

/// `yt-dlp` from `PATH`, unless the environment points at another binary (tests, unusual installs).
pub fn program() -> OsString {
    std::env::var_os("LIMUSIC_YTDLP").unwrap_or_else(|| OsString::from("yt-dlp"))
}

/// `~/Downloads` as Tauri resolves it for this platform.
pub fn downloads_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().download_dir().map_err(|e| format!("No Downloads folder: {e}"))
}

/// The folder picked in Settings ▸ Downloads, when it is a usable absolute path. Relative or blank
/// values fall back to `~/Downloads` rather than dropping files next to the process.
pub fn configured_dir(configured: Option<&str>) -> Option<PathBuf> {
    configured
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// Settings folder if set, otherwise the platform's Downloads folder.
pub fn effective_dir(app: &tauri::AppHandle, configured: Option<&str>) -> Result<PathBuf, String> {
    match configured_dir(configured) {
        Some(dir) => Ok(dir),
        None => downloads_dir(app),
    }
}

/// Every yt-dlp spawn goes through here. Limusic is a GUI process on Windows
/// (`windows_subsystem = "windows"`), so a console child would flash its own window on every
/// download and version check unless CREATE_NO_WINDOW is set.
fn ytdlp_command(binary: &OsStr) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

/// The installed yt-dlp's version string, for the download dialog's install hint.
pub async fn version() -> Result<String, String> {
    let output = ytdlp_command(&program())
        .arg("--version")
        .output()
        .await
        .map_err(|e| format!("Could not start yt-dlp: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Runs one download and reports where the file landed.
pub async fn run(
    req: &TrackDownload<'_>,
    dest: Option<&Path>,
    download_dir: &Path,
    cookies: Option<&Path>,
    id: &str,
    progress: Option<ProgressSink<'_>>,
) -> Result<DownloadOutcome, String> {
    run_with(&program(), req, dest, download_dir, cookies, id, progress).await
}

/// `dest` is a full path *without* extension, chosen in the save dialog. Without one the file goes
/// to `download_dir` (or `download_dir/<playlist>/`). Progress ticks go to `progress`; the process
/// is read line by line while it runs, so a long download keeps reporting instead of blocking the
/// caller until it ends.
async fn run_with(
    binary: &OsStr,
    req: &TrackDownload<'_>,
    dest: Option<&Path>,
    download_dir: &Path,
    cookies: Option<&Path>,
    id: &str,
    progress: Option<ProgressSink<'_>>,
) -> Result<DownloadOutcome, String> {
    validate_format(req.format)?;
    validate_quality(req.quality)?;
    validate_video_id(req.video_id)?;

    let (template, expected) = match dest {
        Some(dest) => {
            let parent = dest
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| "The save location has no folder".to_owned())?;
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
            (
                format!("{}.%(ext)s", escape_template(&dest.to_string_lossy())),
                format!("{}.{}", dest.to_string_lossy(), req.format),
            )
        }
        None => {
            let dir = match req.playlist {
                Some(name) if !name.trim().is_empty() => {
                    download_dir.join(sanitize_component(name))
                }
                _ => download_dir.to_path_buf(),
            };
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
            let stem = file_stem(req.artist, req.title);
            (output_template(&dir, &stem), expected_path(&dir, &stem, req.format))
        }
    };

    // Checked before the run: yt-dlp skips an existing file and prints nothing for it.
    let skipped = Path::new(&expected).exists();
    let mut child = ytdlp_command(binary)
        .args(args(req, &template, cookies))
        // Python block-buffers stdout when it is a pipe; unbuffered keeps the progress ticks
        // arriving while the file is still downloading instead of in one lump at the end.
        .env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start yt-dlp: {e}"))?;
    let stdout = child.stdout.take().ok_or("yt-dlp gave no stdout")?;
    let stderr = child.stderr.take().ok_or("yt-dlp gave no stderr")?;

    // Drained in parallel: a filled stderr pipe would block the child while we are reading stdout.
    let stderr_task = tokio::spawn(async move {
        let mut text = String::new();
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => text.push_str(&String::from_utf8_lossy(&buf)),
            }
        }
        text
    });

    let mut printed = None;
    let mut stdout_text = String::new();
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = match reader.read_until(b'\n', &mut buf).await {
            Ok(read) => read,
            Err(e) => {
                let _ = child.kill().await;
                let _ = stderr_task.await;
                return Err(e.to_string());
            }
        };
        if read == 0 {
            break;
        }
        // yt-dlp writes its output with the platform's encoding (on Windows, the ANSI code page),
        // so a non-ASCII title can be invalid UTF-8. Lossy decoding keeps the path line usable
        // instead of aborting a download that already finished.
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\r', '\n']);
        if let Some((downloaded, total, speed, eta)) = parse_progress(line) {
            if let Some(sink) = progress {
                sink(Progress {
                    id: id.to_owned(),
                    downloaded,
                    total,
                    percent: progress_percent(downloaded, total),
                    speed,
                    eta,
                });
            }
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() && Path::new(trimmed).is_absolute() {
            printed = Some(trimmed.to_owned());
        }
        stdout_text.push_str(line);
        stdout_text.push('\n');
    }

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let stderr_text = stderr_task.await.unwrap_or_default();

    if !status.success() {
        return Err(format!("{stderr_text}{stdout_text}").trim().to_owned());
    }

    // `--print after_move:filepath` gives the real post-processed path; a skipped download prints
    // nothing, so fall back to the name we predicted.
    Ok(DownloadOutcome { path: printed.unwrap_or(expected), skipped })
}

/// Caps a filename stem on a char boundary so the path stays valid UTF-8.
fn truncate_bytes(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(format: &'a str, quality: &'a str) -> TrackDownload<'a> {
        TrackDownload {
            video_id: "dQw4w9WgXcQ",
            artist: "Artist",
            title: "Title",
            format,
            quality,
            playlist: None,
        }
    }

    #[test]
    fn sanitize_strips_separators_and_reserved_characters() {
        assert_eq!(sanitize_component("A/B:C*?\"<>|\\D"), "A_B_C_______D");
        assert_eq!(sanitize_component("tab\there"), "tab_here");
    }

    #[test]
    fn sanitize_trims_dots_spaces_and_guards_windows_names() {
        assert_eq!(sanitize_component("  x.  "), "x");
        assert_eq!(sanitize_component("CON"), "_CON");
        assert_eq!(sanitize_component("con.txt"), "_con.txt");
        assert_eq!(sanitize_component("..."), "Unknown");
    }

    #[test]
    fn sanitize_truncates_on_a_char_boundary() {
        let long = "日".repeat(200);
        let name = sanitize_component(&long);
        assert!(name.len() <= MAX_STEM_BYTES, "{} bytes", name.len());
        assert_eq!(name.chars().count(), MAX_STEM_BYTES / 3); // no replacement chars / broken UTF-8
    }

    #[test]
    fn file_stem_falls_back_to_title_then_a_constant() {
        assert_eq!(file_stem("Artist", "Title"), "Artist - Title");
        assert_eq!(file_stem("", "Title"), "Title");
        assert_eq!(file_stem("Artist", ""), "Artist");
        assert_eq!(file_stem(" ", "\t"), "Unknown track");
    }

    #[test]
    fn template_escapes_percent_and_ends_with_the_extension_marker() {
        assert_eq!(escape_template("100%"), "100%%");
        let template = output_template(Path::new("/tmp"), "100% - Song");
        assert!(template.ends_with("100%% - Song.%(ext)s"), "{template}");
    }

    #[test]
    fn args_ask_for_machine_readable_progress_lines() {
        let args = args(&req("mp3", "best"), "/tmp/x.%(ext)s", None);
        // `--print` implies --quiet; without --progress the template lines are suppressed.
        assert!(args.contains(&"--progress".to_owned()));
        assert!(args.contains(&"--newline".to_owned()));
        let at = args
            .iter()
            .position(|a| a == "--progress-template")
            .expect("progress template is present");
        assert!(args[at + 1].contains(PROGRESS_PREFIX), "{}", args[at + 1]);
        assert!(!args.iter().any(|a| a == "--no-progress"));
    }

    #[test]
    fn progress_lines_parse_the_template_fields() {
        assert_eq!(
            parse_progress("LIMUSIC-PROGRESS 512 2048 NA 64000.5 7"),
            Some((512, Some(2048), Some(64000.5), Some(7)))
        );
        // A missing total falls back to the estimate; `NA` fields become None.
        assert_eq!(
            parse_progress("LIMUSIC-PROGRESS 512 NA 900 NA NA"),
            Some((512, Some(900), None, None))
        );
        assert_eq!(parse_progress("[download]  10% of 2.00MiB"), None);
    }

    #[test]
    fn progress_percent_stays_inside_the_bar() {
        assert_eq!(progress_percent(0, Some(100)), 0.0);
        assert_eq!(progress_percent(50, Some(100)), 50.0);
        assert_eq!(progress_percent(120, Some(100)), 100.0);
        assert_eq!(progress_percent(10, None), 0.0);
    }

    #[test]
    fn args_extract_audio_with_the_requested_container() {
        let args = args(&req("flac", "320"), "/tmp/Artist - Title.%(ext)s", None);
        let joined = args.join(" ");
        assert!(args.contains(&"-x".to_owned()));
        assert!(joined.contains("--audio-format flac"));
        assert!(joined.contains("--audio-quality 320K"));
    }

    #[test]
    fn best_quality_leaves_ytdlp_its_own_default() {
        let args = args(&req("mp3", "best"), "/tmp/x.%(ext)s", None);
        assert!(!args.iter().any(|a| a == "--audio-quality"));
    }

    #[test]
    fn args_embed_metadata_and_cover_except_for_wav() {
        let mp3 = args(&req("mp3", "best"), "/tmp/x.%(ext)s", None).join(" ");
        assert!(mp3.contains("--embed-metadata"));
        assert!(mp3.contains("--embed-thumbnail"));
        assert!(mp3.contains("--convert-thumbnails jpg"));
        assert!(mp3.contains("--no-write-thumbnail"));

        let wav = args(&req("wav", "best"), "/tmp/x.%(ext)s", None).join(" ");
        assert!(wav.contains("--embed-metadata"));
        assert!(!wav.contains("--embed-thumbnail"));
    }

    #[test]
    fn args_never_follow_playlists_and_put_the_url_last() {
        let args = args(&req("mp3", "best"), "/tmp/x.%(ext)s", None);
        assert!(args.contains(&"--no-playlist".to_owned()));
        assert_eq!(args.last().unwrap(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn args_pass_the_cookie_jar_to_ytdlp() {
        let jar = Path::new("/tmp/limusic.cookies");
        let with_cookies = args(&req("mp3", "best"), "/tmp/x.%(ext)s", Some(jar));
        let at = with_cookies.iter().position(|a| a == "--cookies").expect("--cookies is present");
        assert_eq!(with_cookies[at + 1], "/tmp/limusic.cookies");
        assert!(!args(&req("mp3", "best"), "/tmp/x.%(ext)s", None)
            .iter()
            .any(|a| a == "--cookies"));
    }

    #[test]
    fn a_cookie_header_becomes_a_netscape_jar() {
        let jar =
            cookie_jar_contents("SID=abc; HSID=def=ghi; ; broken; __Secure-3PSID=jkl").unwrap();
        let rows: Vec<&str> = jar.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(rows.len(), 3, "{jar}");
        assert_eq!(rows[0], ".youtube.com\tTRUE\t/\tTRUE\t2147483647\tSID\tabc");
        // Everything after the first `=` belongs to the value.
        assert!(rows[1].ends_with("\tHSID\tdef=ghi"), "{jar}");
        assert!(rows[2].ends_with("\t__Secure-3PSID\tjkl"), "{jar}");
    }

    #[test]
    fn cookie_pairs_that_would_break_a_row_are_skipped() {
        let jar = cookie_jar_contents("good=1; tab\tname=2; bad=line\nbreak; #comment=3; =empty")
            .unwrap();
        let rows: Vec<&str> = jar.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(rows.len(), 1, "{jar}");
        assert_eq!(rows[0], ".youtube.com\tTRUE\t/\tTRUE\t2147483647\tgood\t1");
        assert!(cookie_jar_contents("; ; =").is_none(), "signed out writes no jar");
    }

    #[cfg(unix)]
    #[test]
    fn the_cookie_jar_is_private_and_removed_when_dropped() {
        use std::os::unix::fs::PermissionsExt;
        let jar = write_cookie_jar("SID=abc").unwrap().expect("a jar");
        let path = jar.path().to_path_buf();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        drop(jar);
        assert!(!path.exists(), "the session does not outlive the download");
    }

    #[test]
    fn video_ids_with_shell_metacharacters_are_refused() {
        assert!(validate_video_id("dQw4w9WgXcQ").is_ok());
        assert!(validate_video_id("abc&rm -rf").is_err());
        assert!(validate_video_id("").is_err());
        assert!(validate_video_id("a b").is_err());
    }

    #[test]
    fn format_and_quality_allowlists_reject_unknown_values() {
        assert!(validate_format("ogg").is_err());
        assert!(validate_quality("999").is_err());
    }

    #[test]
    fn playlist_downloads_nest_under_the_playlist_folder() {
        let path = suggested_path(Path::new("/dl"), "A/B", "Song", "mp3", Some("My/List"));
        assert_eq!(path, "/dl/My_List/A_B - Song.mp3");
    }

    #[test]
    fn only_absolute_configured_folders_are_used() {
        assert_eq!(
            configured_dir(Some("/home/me/Music")).as_deref(),
            Some(Path::new("/home/me/Music"))
        );
        assert_eq!(configured_dir(Some("  /tmp/dl  ")).as_deref(), Some(Path::new("/tmp/dl")));
        assert_eq!(configured_dir(Some("relative/dir")), None);
        assert_eq!(configured_dir(Some("   ")), None);
        assert_eq!(configured_dir(None), None);
    }

    #[test]
    fn cookie_jars_are_only_stale_after_a_day() {
        let now = SystemTime::now();
        assert!(!is_stale_jar(Some(now), now), "a jar written now belongs to a live download");
        assert!(
            !is_stale_jar(Some(now - Duration::from_secs(3600)), now),
            "an hour old is still in use"
        );
        assert!(
            is_stale_jar(Some(now - Duration::from_secs(25 * 3600)), now),
            "a day old is abandoned"
        );
        assert!(!is_stale_jar(None, now), "an unknown mtime is not evidence of abandonment");
    }

    #[cfg(unix)]
    fn fake_program(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-yt-dlp");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_fake_ytdlp_reports_the_path_it_printed() {
        let tmp = tempfile::tempdir().unwrap();
        let printed = tmp.path().join("Artist - Title.mp3");
        let fake = fake_program(tmp.path(), &format!("echo '{}'", printed.display()));
        let out =
            run_with(fake.as_os_str(), &req("mp3", "best"), None, tmp.path(), None, "test", None)
                .await
                .unwrap();
        assert_eq!(out.path, printed.to_string_lossy());
        assert!(!out.skipped);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_fake_ytdlp_surfaces_its_stderr() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_program(tmp.path(), "echo boom >&2\nexit 1");
        let err =
            run_with(fake.as_os_str(), &req("mp3", "best"), None, tmp.path(), None, "test", None)
                .await
                .unwrap_err();
        assert!(err.contains("boom"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn progress_ticks_reach_the_sink_and_the_path_still_comes_back() {
        use std::sync::Mutex;

        let tmp = tempfile::tempdir().unwrap();
        let printed = tmp.path().join("out.mp3");
        let fake = fake_program(
            tmp.path(),
            &format!(
                "echo 'LIMUSIC-PROGRESS 512 1024 NA 64000 1'\n\
                 echo 'LIMUSIC-PROGRESS 1024 1024 NA 64000 0'\n\
                 echo '{}'",
                printed.display()
            ),
        );
        let seen = Mutex::new(Vec::new());
        let sink = |p: Progress| seen.lock().unwrap().push(p);
        let out = run_with(
            fake.as_os_str(),
            &req("mp3", "best"),
            None,
            tmp.path(),
            None,
            "job-1",
            Some(&sink),
        )
        .await
        .unwrap();

        assert_eq!(out.path, printed.to_string_lossy());
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].id, "job-1");
        assert_eq!(seen[0].percent, 50.0);
        assert_eq!(seen[1].percent, 100.0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_utf8_output_does_not_fail_a_finished_download() {
        let tmp = tempfile::tempdir().unwrap();
        let printed = tmp.path().join("out.mp3");
        // A Windows code page can emit invalid UTF-8 for a title; the path line after it must still
        // be read and the download reported as a success.
        let fake = fake_program(
            tmp.path(),
            &format!("printf '\\377\\376\\n'\necho '{}'", printed.display()),
        );
        let out =
            run_with(fake.as_os_str(), &req("mp3", "best"), None, tmp.path(), None, "job-1", None)
                .await
                .unwrap();
        assert_eq!(out.path, printed.to_string_lossy());
    }
}
