//! Keep Windows and macOS awake while a cast plays (#399).
//!
//! Both already stay awake while an audio stream is open (the audio driver's power request,
//! coreaudiod's assertion), which is why local playback needs nothing here. While casting, mpv is
//! stopped and nothing holds the machine up, so a laptop would fall asleep mid-song and take the
//! cast with it: the device fetches every track from this computer. Linux has `inhibit.rs`.
//! Suspend only: the screen still blanks and locks as usual.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

static TX: OnceLock<Mutex<Sender<bool>>> = OnceLock::new();

/// Hold off idle sleep (`true`) or let it happen again (`false`). Cheap to repeat.
pub fn set(on: bool) {
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<bool>();
        // One thread owns the hold: on Windows the request belongs to the thread that made it.
        std::thread::spawn(move || {
            let mut held = Hold::default();
            for on in rx {
                held.set(on);
            }
        });
        Mutex::new(tx)
    });
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(on);
    }
}

#[derive(Default)]
struct Hold {
    on: bool,
    #[cfg(target_os = "macos")]
    child: Option<std::process::Child>,
}

impl Hold {
    fn set(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.on = on;
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::System::Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
            };
            let flags = if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
            unsafe { SetThreadExecutionState(flags) };
        }
        #[cfg(target_os = "macos")]
        {
            // ponytail: `caffeinate` rather than IOKit bindings. `-w` ties it to this process, so
            // a crash never leaves the Mac held awake.
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if on {
                self.child = std::process::Command::new("/usr/bin/caffeinate")
                    .args(["-i", "-w", &std::process::id().to_string()])
                    .spawn()
                    .ok();
            }
        }
        tracing::debug!(on, "keep awake for the cast");
    }
}
