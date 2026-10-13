use crate::cache_codec::CacheFormat;
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

pub struct InstrumentalCache {
    dir: PathBuf,
    format: CacheFormat,
    max_bytes: std::sync::atomic::AtomicU64,
}

impl InstrumentalCache {
    pub fn new(dir: PathBuf, format: CacheFormat, max_bytes: u64) -> Result<Self> {
        let tmp_dir = dir.join("tmp");
        fs::create_dir_all(&dir)?;
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
        if let Ok(entries) = fs::read_dir(&dir) {
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

        Ok(Self { dir, format, max_bytes: std::sync::atomic::AtomicU64::new(max_bytes) })
    }

    pub fn format(&self) -> CacheFormat {
        self.format
    }

    pub fn set_max_bytes(&self, max_bytes: u64) {
        self.max_bytes.store(max_bytes, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn path_for(&self, track_id: &str) -> PathBuf {
        let sanitized = sanitize_track_id(track_id);
        let ext = self.format.extension();
        self.dir.join(format!("{}.{}", sanitized, ext))
    }

    pub fn tmp_wav_path(&self, track_id: &str) -> PathBuf {
        let sanitized = sanitize_track_id(track_id);
        self.dir.join("tmp").join(format!("{}.wav", sanitized))
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
                        if ext.eq_ignore_ascii_case("flac") || ext.eq_ignore_ascii_case("opus") {
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
                        if ext.eq_ignore_ascii_case("flac") || ext.eq_ignore_ascii_case("opus") {
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

            if fs::remove_file(&path).is_ok() {
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
                        if ext.eq_ignore_ascii_case("flac")
                            || ext.eq_ignore_ascii_case("opus")
                            || ext.eq_ignore_ascii_case("tmp")
                        {
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

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 1024 * 1024)?;
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

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 20)?;

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

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 10)?;

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

        let _cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 1024)?;

        assert!(!stale_sub_tmp.exists(), "Stale sub tmp file should be removed by new()");
        assert!(!stale_root_tmp.exists(), "Stale root tmp file should be removed by new()");

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_cache_set_max_bytes() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_cache_test_set_max");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 100)?;
        cache.set_max_bytes(200);
        assert_eq!(cache.max_bytes.load(std::sync::atomic::Ordering::SeqCst), 200);

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }
}
