use notify::{recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    fs,
    path::PathBuf,
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
    let Some(parent) = watched.parent().map(PathBuf::from) else {
        return Err(format!("Matugen config path has no parent directory: {}", watched.display()));
    };
    fs::create_dir_all(&parent).map_err(|e| {
        format!("Matugen could not create its config directory {}: {e}", parent.display())
    })?;

    let (tx, rx) = mpsc::channel();
    let mut watcher = recommended_watcher(move |r| {
        let _ = tx.send(r);
    })
    .map_err(|e| format!("Matugen could not create a filesystem watcher: {e}"))?;
    watcher.watch(&parent, RecursiveMode::NonRecursive).map_err(|e| {
        format!("Matugen could not watch its config directory {}: {e}", parent.display())
    })?;

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
            if event.kind.is_access() || !event.paths.contains(&watched) {
                continue;
            }
            // One write is several events (truncate, data, close): read once after they stop.
            while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}
            let _ = app.emit("matugen-theme-changed", fs::read_to_string(&watched).ok());
        }
    });

    *guard = Some(watcher);
    Ok(())
}

pub fn stop() {
    WATCHER.lock().unwrap_or_else(|e| e.into_inner()).take();
}
