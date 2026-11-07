use notify::{recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    thread,
    time::Duration,
};
use tauri::{path::BaseDirectory, AppHandle, Emitter, Manager};

const THEME_PATH: &str = "limusic/matugen.css";

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().resolve(THEME_PATH, BaseDirectory::Config).map_err(|e| e.to_string())
}

pub fn read(app: &AppHandle) -> Result<Option<String>, String> {
    match fs::read_to_string(path(app)?) {
        Ok(css) => Ok(Some(css)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The live watcher while the setting is on. Dropping it ends the thread in `start`.
static WATCHER: Mutex<Option<RecommendedWatcher>> = Mutex::new(None);

pub fn start(app: AppHandle) -> Result<(), String> {
    let mut guard = WATCHER.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return Ok(());
    }
    let watched =
        path(&app).map_err(|e| format!("Matugen could not resolve its config path: {e}"))?;
    *guard = Some(watch(watched, move |css| {
        let _ = app.emit("matugen-theme-changed", css);
    })?);
    Ok(())
}

/// Calls `on_change` with the file's new contents after every write to `watched`.
fn watch(
    watched: PathBuf,
    on_change: impl Fn(Option<String>) + Send + 'static,
) -> Result<RecommendedWatcher, String> {
    let Some(parent) = watched.parent().map(PathBuf::from) else {
        return Err(format!("Matugen config path has no parent directory: {}", watched.display()));
    };
    fs::create_dir_all(&parent).map_err(|e| {
        format!("Matugen could not create its config directory {}: {e}", parent.display())
    })?;
    // pywal only writes into its own cache directory, so its users symlink this file there. A
    // directory watch never sees a write behind a symlink, so the target's directory is watched too.
    let target = match fs::symlink_metadata(&watched) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(&watched).ok(),
        _ => None,
    };

    let (tx, rx) = mpsc::channel();
    let mut watcher = recommended_watcher(move |r| {
        let _ = tx.send(r);
    })
    .map_err(|e| format!("Matugen could not create a filesystem watcher: {e}"))?;
    for dir in std::iter::once(parent.as_path()).chain(target.as_deref().and_then(Path::parent)) {
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Matugen could not watch {}: {e}", dir.display()))?;
    }

    thread::spawn(move || {
        // Ends when stop() drops the watcher, which closes the channel.
        while let Ok(event) = rx.recv() {
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(%error, "Matugen watcher reported an error");
                    continue;
                }
            };
            // inotify reports opens too, so without the access filter our own read below would
            // wake this loop forever.
            let ours = |p: &PathBuf| *p == watched || Some(p) == target.as_ref();
            if event.kind.is_access() || !event.paths.iter().any(ours) {
                continue;
            }
            // One write is several events (truncate, data, close): read once after they stop.
            while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}
            on_change(fs::read_to_string(&watched).ok());
        }
    });

    Ok(watcher)
}

pub fn stop() {
    WATCHER.lock().unwrap_or_else(|e| e.into_inner()).take();
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn follows_writes_behind_a_symlink() {
        let root = std::env::temp_dir().join(format!("limusic-matugen-{}", std::process::id()));
        let (config, cache) = (root.join("config"), root.join("cache"));
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&cache).unwrap();
        let real = cache.join("limusic.css");
        fs::write(&real, "old").unwrap();
        let link = config.join("matugen.css");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let (tx, rx) = mpsc::channel();
        let _watcher = watch(link, move |css| {
            let _ = tx.send(css);
        })
        .unwrap();
        fs::write(&real, "new").unwrap();
        let css = rx.recv_timeout(Duration::from_secs(5));
        let _ = fs::remove_dir_all(&root);
        assert_eq!(css.unwrap().as_deref(), Some("new"));
    }
}
