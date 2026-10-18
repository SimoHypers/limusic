use crate::cache_codec::CACHE_EXT;
use anyhow::Result;
use std::fs::{self, File};
use std::path::PathBuf;

fn fnv1a_64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn sanitize_track_id(track_id: &str) -> String {
    let mut sanitized = String::new();
    for c in track_id.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            sanitized.push(c);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() || sanitized.len() > 100 {
        sanitized = format!("{:016x}", fnv1a_64(track_id));
    }
    let hash = fnv1a_64(track_id);
    let suffix = format!("{:016x}", hash);
    format!("{}-{}", sanitized, &suffix[..8])
}

/// Which file the player has handed to mpv: the one playing, or the one queued next (gapless or
/// crossfade preload). See [`InstrumentalCache::pin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSlot {
    Current = 0,
    Next = 1,
}

pub struct InstrumentalCache {
    dir: PathBuf,
    max_bytes: std::sync::atomic::AtomicU64,
    /// Files the player is using, by [`PinSlot`]. Eviction and [`Self::clear`] hold this lock
    /// while they decide about and delete each file, and [`Self::pin`] takes it to hand a file
    /// out, so a file the player got can never be deleted before (or while) mpv uses it. Held for
    /// one file at a time: playback waits at most for one delete.
    pins: std::sync::Mutex<[Option<PathBuf>; 2]>,
    model_key: std::sync::Mutex<String>,
}

impl InstrumentalCache {
    /// No I/O, so building one at launch can never fail. The directory is created, and a crashed
    /// run's leftovers removed, by [`InstrumentalCache::prepare`] when karaoke is first used.
    /// Nothing else waits for `prepare`: size, eviction and clearing always read the directory as
    /// it is on disk, so a previous session's files count (and can be cleared) with karaoke off,
    /// and a directory that does not exist yet simply holds nothing.
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self {
            dir,
            max_bytes: std::sync::atomic::AtomicU64::new(max_bytes),
            pins: std::sync::Mutex::new([None, None]),
            model_key: std::sync::Mutex::new(crate::models::ModelId::DEFAULT.key().to_string()),
        }
    }

    pub fn set_model(&self, key: &str) {
        let mut m = self.model_key.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *m = key.to_string();
    }

    pub fn model_key(&self) -> String {
        let m = self.model_key.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        m.clone()
    }

    /// Creates the cache directory and deletes temporary files a crash left behind.
    pub fn prepare(&self) -> Result<()> {
        let dir = &self.dir;
        let tmp_dir = dir.join("tmp");
        fs::create_dir_all(dir)?;
        fs::create_dir_all(&tmp_dir)?;

        // Delete any leftover files in dir/tmp
        if let Ok(entries) = fs::read_dir(&tmp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        // Delete any *.tmp in dir (stale from a crash)
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case("tmp") {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn set_max_bytes(&self, max_bytes: u64) {
        self.max_bytes.store(max_bytes, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn path_for(&self, track_id: &str) -> PathBuf {
        let mk = self.model_key();
        let sanitized = sanitize_track_id(track_id);
        let ext = CACHE_EXT;
        self.dir.join(format!("{}__{}.{}", mk, sanitized, ext))
    }

    pub fn tmp_wav_path(&self, track_id: &str) -> PathBuf {
        let mk = self.model_key();
        let sanitized = sanitize_track_id(track_id);
        self.dir.join("tmp").join(format!("{}__{}.wav", mk, sanitized))
    }

    fn lock_pins(&self) -> std::sync::MutexGuard<'_, [Option<PathBuf>; 2]> {
        self.pins.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The player is about to hand `track_id`'s instrumental to mpv in `slot`: returns its path
    /// and pins it, or, when it is not cached, clears the slot and returns `None`. Done under the
    /// lock eviction deletes under, so the path returned cannot be deleted before mpv opens it.
    pub fn pin(&self, slot: PinSlot, track_id: &str) -> Option<PathBuf> {
        let mut pins = self.lock_pins();
        let path = self.get(track_id);
        pins[slot as usize] = path.clone();
        path
    }

    /// `slot` no longer holds an instrumental (the original stream was handed to mpv instead).
    pub fn unpin(&self, slot: PinSlot) {
        self.lock_pins()[slot as usize] = None;
    }

    /// A gapless or crossfade advance: what was queued next is now playing.
    pub fn promote_next(&self) {
        let mut pins = self.lock_pins();
        pins[PinSlot::Current as usize] = pins[PinSlot::Next as usize].take();
    }

    /// Deletes `path` unless the player has it pinned. `false` if kept: pinned, or the delete
    /// failed, which on Windows is what deleting a file another process has open does. The file
    /// then stays until a later eviction.
    fn remove_unless_pinned(&self, path: &std::path::Path) -> bool {
        let pins = self.lock_pins();
        if pins.iter().flatten().any(|p| p == path) {
            return false;
        }
        fs::remove_file(path).is_ok()
    }

    pub fn get(&self, track_id: &str) -> Option<PathBuf> {
        let path = self.path_for(track_id);
        if path.exists() {
            if let Ok(meta) = fs::metadata(&path) {
                if meta.len() > 0 {
                    let now = std::time::SystemTime::now();
                    if let Ok(file) = File::options().write(true).open(&path) {
                        let _ = file.set_modified(now);
                    }
                    return Some(path);
                }
            }
        }
        None
    }

    pub fn contains(&self, track_id: &str) -> bool {
        let path = self.path_for(track_id);
        if path.exists() {
            if let Ok(meta) = fs::metadata(&path) {
                return meta.len() > 0;
            }
        }
        false
    }

    pub fn total_size(&self) -> u64 {
        let mut total = 0;
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case(CACHE_EXT) {
                            if let Ok(meta) = fs::metadata(&path) {
                                total += meta.len();
                            }
                        }
                    }
                }
            }
        }
        total
    }

    pub fn evict(&self, keep: &[PathBuf]) -> Result<u64> {
        let max_bytes = self.max_bytes.load(std::sync::atomic::Ordering::SeqCst);
        let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case(CACHE_EXT) {
                            if let Ok(meta) = fs::metadata(&path) {
                                let modified =
                                    meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                                files.push((path, meta.len(), modified));
                            }
                        }
                    }
                }
            }
        }

        // Sort by modified time ascending (oldest first)
        files.sort_by(|a, b| a.2.cmp(&b.2));

        let mut current_size: u64 = files.iter().map(|f| f.1).sum();
        let mut freed_bytes = 0;
        let mut removed = 0;

        for (path, size, _) in files {
            if current_size <= max_bytes {
                break;
            }
            let mut protected = false;
            for k in keep {
                if let (Ok(p_canon), Ok(k_canon)) = (path.canonicalize(), k.canonicalize()) {
                    if p_canon == k_canon {
                        protected = true;
                        break;
                    }
                } else if path == *k {
                    protected = true;
                    break;
                }
            }
            if protected {
                continue;
            }

            if self.remove_unless_pinned(&path) {
                current_size = current_size.saturating_sub(size);
                freed_bytes += size;
                removed += 1;
            }
        }

        let total_bytes = self.total_size();
        if removed > 0 {
            tracing::info!(target: "karaoke::cache", removed, freed_bytes, total_bytes, "karaoke cache evict");
        } else {
            tracing::debug!(target: "karaoke::cache", removed, freed_bytes, total_bytes, "karaoke cache evict");
        }

        Ok(freed_bytes)
    }

    pub fn clear(&self) -> Result<u64> {
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case(CACHE_EXT) {
                            // What is playing or queued next stays (see `pin`).
                            let _ = self.remove_unless_pinned(&path);
                        } else if ext.eq_ignore_ascii_case("tmp") {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
        let tmp_dir = self.dir.join("tmp");
        if let Ok(entries) = fs::read_dir(&tmp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        Ok(self.total_size())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_collision() {
        let id1 = "a/b";
        let id2 = "a_b";
        let p1 = sanitize_track_id(id1);
        let p2 = sanitize_track_id(id2);
        assert_ne!(p1, p2, "Different track IDs must not collide after sanitization");
    }

    #[test]
    fn test_cache_basic_operations() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_basic");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 1024 * 1024);
        cache.prepare()?;
        assert!(cache.get("track1").is_none());

        let p1 = cache.path_for("track1");
        fs::write(&p1, b"dummy audio data")?;

        assert!(cache.get("track1").is_some());
        assert!(cache.total_size() > 0);

        cache.clear()?;
        assert!(cache.get("track1").is_none());
        assert_eq!(cache.total_size(), 0);

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_cache_eviction_and_keep() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_evict");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 20);
        cache.prepare()?;

        let p1 = cache.path_for("track1");
        let p2 = cache.path_for("track2");

        fs::write(&p1, b"12345678901234567890")?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&p2, b"12345678901234567890")?;

        cache.evict(&[p1.clone()])?;

        assert!(p1.exists(), "p1 should be kept because it was specified in keep");
        assert!(!p2.exists(), "p2 should be evicted");

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_cache_eviction_multiple_protected() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_evict_multi");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 10);
        cache.prepare()?;

        let p_old = cache.path_for("old");
        let p1 = cache.path_for("track1");
        let p2 = cache.path_for("track2");
        let p3 = cache.path_for("track3");

        fs::write(&p_old, b"12345678901234567890")?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&p1, b"1234567890")?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&p2, b"1234567890")?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&p3, b"1234567890")?;

        cache.evict(&[p1.clone(), p2.clone(), p3.clone()])?;

        assert!(p1.exists(), "p1 should survive");
        assert!(p2.exists(), "p2 should survive");
        assert!(p3.exists(), "p3 should survive");
        assert!(!p_old.exists(), "older unprotected file should be evicted");

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_stale_tmp_cleanup() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_stale");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let tmp_dir = tmp.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        let stale_sub_tmp = tmp_dir.join("stale.wav");
        fs::write(&stale_sub_tmp, b"stale")?;

        let stale_root_tmp = tmp.join("stale.flac.tmp");
        fs::write(&stale_root_tmp, b"stale")?;

        let cache = InstrumentalCache::new(tmp.clone(), 1024);
        assert!(stale_sub_tmp.exists() && stale_root_tmp.exists(), "new() must not touch the disk");

        cache.prepare()?;
        assert!(!stale_sub_tmp.exists(), "Stale sub tmp file should be removed by prepare()");
        assert!(!stale_root_tmp.exists(), "Stale root tmp file should be removed by prepare()");

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_cache_set_max_bytes() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_set_max");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 100);
        cache.set_max_bytes(200);
        assert_eq!(cache.max_bytes.load(std::sync::atomic::Ordering::SeqCst), 200);

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    /// The app builds the cache at launch and only calls `prepare` when karaoke is first turned
    /// on, so every lookup has to cope with the directory not existing yet.
    #[test]
    fn test_unprepared_cache_is_empty_and_harmless() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_unprepared");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 100);
        assert!(cache.get("track1").is_none());
        assert!(!cache.contains("track1"));
        assert_eq!(cache.total_size(), 0);
        assert_eq!(cache.evict(&[])?, 0);
        assert_eq!(cache.clear()?, 0);
        assert!(!tmp.exists(), "nothing may be created before prepare()");
        Ok(())
    }

    /// After a restart karaoke is off and `prepare` has not run, but Settings still has to see,
    /// evict and clear what earlier sessions left on disk.
    #[test]
    fn test_unprepared_cache_sees_previous_session_files() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_previous_session");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }
        // An earlier session's cache, written before this instance existed.
        let earlier = InstrumentalCache::new(tmp.clone(), u64::MAX);
        earlier.prepare()?;
        let old = earlier.path_for("old");
        let new = earlier.path_for("new");
        fs::write(&old, [0u8; 30])?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&new, [0u8; 30])?;

        // This launch: constructed, never prepared.
        let cache = InstrumentalCache::new(tmp.clone(), u64::MAX);
        assert_eq!(cache.total_size(), 60, "usage counts the earlier files");

        cache.set_max_bytes(40);
        assert_eq!(cache.evict(&[])?, 30, "a lower limit evicts the oldest");
        assert!(!old.exists() && new.exists());
        assert_eq!(cache.total_size(), 30);

        assert_eq!(cache.clear()?, 0, "clear removes the rest");
        assert!(!new.exists());

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    /// What the player has handed to mpv (playing, or queued next) survives eviction and "Clear
    /// cache", however old it is; once unpinned it goes like any other file.
    #[test]
    fn test_pinned_files_are_never_deleted() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_pins");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }
        let cache = InstrumentalCache::new(tmp.clone(), 10);
        cache.prepare()?;
        let playing = cache.path_for("playing");
        let next = cache.path_for("next");
        let other = cache.path_for("other");
        fs::write(&playing, [0u8; 20])?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&next, [0u8; 20])?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&other, [0u8; 20])?;

        assert_eq!(cache.pin(PinSlot::Current, "playing"), Some(playing.clone()));
        assert_eq!(cache.pin(PinSlot::Next, "next"), Some(next.clone()));
        assert_eq!(cache.pin(PinSlot::Next, "not cached"), None, "nothing to pin");
        assert_eq!(cache.pin(PinSlot::Next, "next"), Some(next.clone()));

        cache.evict(&[])?;
        assert!(playing.exists() && next.exists(), "the oldest files are pinned");
        assert!(!other.exists(), "the only unpinned file went");

        cache.clear()?;
        assert!(playing.exists() && next.exists(), "clear keeps what is in use");

        // Gapless advance: next becomes current, the old current is free.
        cache.promote_next();
        cache.clear()?;
        assert!(!playing.exists(), "no longer pinned");
        assert!(next.exists(), "now the current track");

        cache.unpin(PinSlot::Current);
        cache.clear()?;
        assert!(!next.exists());
        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_multi_model_cache() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_multi_model");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 1024 * 1024);
        cache.prepare()?;

        let p_kara = cache.path_for("track1");
        fs::write(&p_kara, b"kara audio")?;
        assert!(cache.contains("track1"));

        cache.set_model("inst_hq_3");
        assert!(!cache.contains("track1"));
        let p_inst = cache.path_for("track1");
        assert_ne!(p_kara, p_inst);
        assert!(!p_inst.exists());

        fs::write(&p_inst, b"inst audio")?;
        assert!(cache.contains("track1"));

        cache.set_model("kara");
        assert!(cache.contains("track1"));
        assert_eq!(fs::read_to_string(&cache.path_for("track1"))?, "kara audio");

        cache.set_model("inst_hq_3");
        assert_eq!(cache.total_size() as usize, "kara audio".len() + "inst audio".len());

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_set_model_does_not_touch_pinned_file() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_model_pin");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), 10);
        cache.prepare()?;

        let p1 = cache.path_for("track1");
        fs::write(&p1, b"pinned under kara")?;
        assert_eq!(cache.pin(PinSlot::Current, "track1"), Some(p1.clone()));

        cache.set_model("inst_hq_3");
        cache.clear()?;

        assert!(p1.exists());

        cache.unpin(PinSlot::Current);
        cache.clear()?;
        assert!(!p1.exists());

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }
}
