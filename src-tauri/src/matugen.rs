use notify::{recommended_watcher, RecursiveMode, Watcher};
use std::{fs, path::PathBuf, sync::mpsc, thread, time::Duration};
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

pub fn start(app: AppHandle) {
    let path = match path(&app) {
        Ok(p) => p,
        Err(_) => return,
    };
    let Some(parent) = path.parent() else { return };
    let _ = fs::create_dir_all(parent);
    let watched = path.clone();
    let (tx, rx) = mpsc::channel();
    let Ok(mut watcher) = recommended_watcher(move |r| {
        let _ = tx.send(r);
    }) else {
        return;
    };
    if watcher.watch(parent, RecursiveMode::NonRecursive).is_err() {
        return;
    }

    thread::spawn(move || {
        let _watcher = watcher;
        while let Ok(result) = rx.recv() {
            match result {
                Ok(event) => {
                    if !event.paths.iter().any(|p| p == &watched) {
                        continue;
                    }
                    thread::sleep(Duration::from_millis(75));
                    let css = fs::read_to_string(&watched).ok();
                    let _ = app.emit("matugen-theme-changed", css);
                }
                Err(error) => tracing::warn!(%error, "Matugen watcher reported an error"),
            }
        }
    });
}
