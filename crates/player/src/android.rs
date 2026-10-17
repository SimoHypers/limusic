//! Android audio player implementation.
//!
//! Provides the same high-level `Player` interface and `PlayerEvent` channel as the desktop
//! `libmpv` player, but delegates playback on Android to the platform's media service
//! (ExoPlayer / Media3 via MediaSessionService) through JNI or standalone state tracking.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::{Error, PlayerEvent};

static GLOBAL_TX: OnceLock<UnboundedSender<PlayerEvent>> = OnceLock::new();

pub(crate) fn register_event_sender(tx: UnboundedSender<PlayerEvent>) {
    let _ = GLOBAL_TX.set(tx);
}

pub(crate) fn get_event_sender() -> Option<UnboundedSender<PlayerEvent>> {
    GLOBAL_TX.get().cloned()
}

/// The Android player backend.
pub struct Player {
    events: Option<UnboundedReceiver<PlayerEvent>>,
    tx: UnboundedSender<PlayerEvent>,
    volume: AtomicI64,
    speed: Mutex<f64>,
    loop_file: AtomicBool,
    current_path: Mutex<Option<String>>,
    is_idle: AtomicBool,
    gain_db: Mutex<Option<f64>>,
}

impl Player {
    pub fn new(_cache_dir: &str) -> Result<Player, Error> {
        let (tx, rx) = unbounded_channel();
        register_event_sender(tx.clone());

        Ok(Player {
            events: Some(rx),
            tx,
            volume: AtomicI64::new(100),
            speed: Mutex::new(1.0),
            loop_file: AtomicBool::new(false),
            current_path: Mutex::new(None),
            is_idle: AtomicBool::new(true),
            gain_db: Mutex::new(None),
        })
    }

    pub fn take_events(&mut self) -> Option<UnboundedReceiver<PlayerEvent>> {
        self.events.take()
    }

    pub fn load(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        gain_db: Option<f64>,
        start_secs: Option<f64>,
    ) -> Result<(), Error> {
        *self.current_path.lock().unwrap() = Some(url.to_owned());
        *self.gain_db.lock().unwrap() = gain_db;
        self.is_idle.store(false, Ordering::SeqCst);

        // Notify UI that track is loading / playing
        let _ = self.tx.send(PlayerEvent::Playing(true));
        if let Some(pos) = start_secs {
            let _ = self.tx.send(PlayerEvent::Position(pos));
        }

        dispatch_android_load(url, headers, gain_db, start_secs);
        Ok(())
    }

    pub fn play(&self) -> Result<(), Error> {
        self.is_idle.store(false, Ordering::SeqCst);
        let _ = self.tx.send(PlayerEvent::Playing(true));
        let _ = self.tx.send(PlayerEvent::Paused(false));
        dispatch_android_play();
        Ok(())
    }

    pub fn pause(&self) -> Result<(), Error> {
        let _ = self.tx.send(PlayerEvent::Paused(true));
        dispatch_android_pause();
        Ok(())
    }

    pub fn toggle(&self) -> Result<(), Error> {
        dispatch_android_toggle();
        Ok(())
    }

    pub fn stop(&self) -> Result<(), Error> {
        self.is_idle.store(true, Ordering::SeqCst);
        *self.current_path.lock().unwrap() = None;
        let _ = self.tx.send(PlayerEvent::Playing(false));
        dispatch_android_stop();
        Ok(())
    }

    pub fn seek(&self, position_secs: f64) -> Result<(), Error> {
        let _ = self.tx.send(PlayerEvent::Position(position_secs));
        dispatch_android_seek(position_secs);
        Ok(())
    }

    pub fn enqueue(&self, url: &str) -> Result<(), Error> {
        dispatch_android_enqueue(url);
        Ok(())
    }

    pub fn clear_playlist(&self) -> Result<(), Error> {
        dispatch_android_clear_playlist();
        Ok(())
    }

    pub fn set_volume(&self, volume: i64) -> Result<(), Error> {
        let clamped = volume.clamp(0, 100);
        self.volume.store(clamped, Ordering::Relaxed);
        dispatch_android_volume(clamped);
        Ok(())
    }

    pub fn volume(&self) -> i64 {
        self.volume.load(Ordering::Relaxed)
    }

    pub fn set_gain(&self, gain_db: Option<f64>) -> Result<(), Error> {
        *self.gain_db.lock().unwrap() = gain_db;
        dispatch_android_gain(gain_db);
        Ok(())
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), Error> {
        let clamped = speed.clamp(0.25, 2.0);
        *self.speed.lock().unwrap() = clamped;
        dispatch_android_speed(clamped);
        Ok(())
    }

    pub fn set_pitch(&self, semitones: i32) -> Result<(), Error> {
        dispatch_android_pitch(semitones);
        Ok(())
    }

    pub fn set_loop_file(&self, loop_file: bool) -> Result<(), Error> {
        self.loop_file.store(loop_file, Ordering::SeqCst);
        dispatch_android_loop_file(loop_file);
        Ok(())
    }

    pub fn is_idle(&self) -> bool {
        self.is_idle.load(Ordering::SeqCst)
    }

    pub fn current_path(&self) -> Option<String> {
        self.current_path.lock().unwrap().clone()
    }

    pub fn set_video_for(&self, _audio_url: &str, _video_url: &str) {}

    pub fn set_video_visible(&self, _visible: bool) {}

    pub fn set_crossfade_ms(&self, _ms: u64) {}

    pub fn set_http_proxy(&self, _proxy: Option<&str>) -> Result<(), Error> {
        Ok(())
    }

    pub fn set_video_window(&self, _wid: i64) -> Result<(), Error> {
        Ok(())
    }

    pub fn video_output_up(&self) -> bool {
        false
    }

    pub fn video_thumbnail(&self, _max_width: usize, _unless_at: Option<f64>) -> Option<crate::video::Thumbnail> {
        None
    }
}

static JAVA_VM: OnceLock<jni::JavaVM> = OnceLock::new();

#[no_mangle]
pub extern "C" fn JNI_OnLoad(vm: *mut std::ffi::c_void, _reserved: *mut std::ffi::c_void) -> i32 {
    unsafe {
        if let Ok(jvm) = jni::JavaVM::from_raw(vm as *mut jni::sys::JavaVM) {
            let _ = JAVA_VM.set(jvm);
        }
    }
    0x00010006 // JNI_VERSION_1_6
}

fn with_env<F: FnOnce(&mut jni::JNIEnv) -> jni::errors::Result<()>>(f: F) {
    if let Some(jvm) = JAVA_VM.get() {
        if let Ok(mut env) = jvm.attach_current_thread_permanently() {
            let _ = f(&mut env);
        }
    }
}

// Android dispatch implementations bridging to AudioBridge in Kotlin
fn dispatch_android_load(url: &str, _headers: &HashMap<String, String>, _gain: Option<f64>, start: Option<f64>) {
    let start_pos = start.unwrap_or(0.0);
    with_env(|env| {
        let j_url = env.new_string(url)?;
        env.call_static_method(
            "com/limusic/desktop/AudioBridge",
            "load",
            "(Ljava/lang/String;D)V",
            &[jni::objects::JValue::Object(&j_url), jni::objects::JValue::Double(start_pos)],
        )?;
        Ok(())
    });
}

fn dispatch_android_play() {
    with_env(|env| {
        env.call_static_method("com/limusic/desktop/AudioBridge", "play", "()V", &[])?;
        Ok(())
    });
}

fn dispatch_android_pause() {
    with_env(|env| {
        env.call_static_method("com/limusic/desktop/AudioBridge", "pause", "()V", &[])?;
        Ok(())
    });
}

fn dispatch_android_toggle() {
    // Handled via play/pause state
}

fn dispatch_android_stop() {
    with_env(|env| {
        env.call_static_method("com/limusic/desktop/AudioBridge", "stop", "()V", &[])?;
        Ok(())
    });
}

fn dispatch_android_seek(pos: f64) {
    with_env(|env| {
        env.call_static_method(
            "com/limusic/desktop/AudioBridge",
            "seek",
            "(D)V",
            &[jni::objects::JValue::Double(pos)],
        )?;
        Ok(())
    });
}

fn dispatch_android_enqueue(_url: &str) {}
fn dispatch_android_clear_playlist() {}

fn dispatch_android_volume(vol: i64) {
    with_env(|env| {
        env.call_static_method(
            "com/limusic/desktop/AudioBridge",
            "setVolume",
            "(I)V",
            &[jni::objects::JValue::Int(vol as i32)],
        )?;
        Ok(())
    });
}

fn dispatch_android_gain(_gain: Option<f64>) {}
fn dispatch_android_speed(_speed: f64) {}
fn dispatch_android_pitch(_semitones: i32) {}
fn dispatch_android_loop_file(_loop_file: bool) {}

// JNI callback bridge for Android MediaSession/ExoPlayer service
#[no_mangle]
pub extern "C" fn Java_com_limusic_desktop_AudioBridge_nativeOnPosition(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    pos: f64,
) {
    if let Some(tx) = get_event_sender() {
        let _ = tx.send(PlayerEvent::Position(pos));
    }
}

#[no_mangle]
pub extern "C" fn Java_com_limusic_desktop_AudioBridge_nativeOnDuration(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    duration: f64,
) {
    if let Some(tx) = get_event_sender() {
        let _ = tx.send(PlayerEvent::Duration(duration));
    }
}

#[no_mangle]
pub extern "C" fn Java_com_limusic_desktop_AudioBridge_nativeOnPlaying(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    playing: bool,
) {
    if let Some(tx) = get_event_sender() {
        let _ = tx.send(PlayerEvent::Playing(playing));
    }
}

#[no_mangle]
pub extern "C" fn Java_com_limusic_desktop_AudioBridge_nativeOnPaused(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    paused: bool,
) {
    if let Some(tx) = get_event_sender() {
        let _ = tx.send(PlayerEvent::Paused(paused));
    }
}

#[no_mangle]
pub extern "C" fn Java_com_limusic_desktop_AudioBridge_nativeOnTrackEnded(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    if let Some(tx) = get_event_sender() {
        let _ = tx.send(PlayerEvent::TrackEnded);
    }
}

#[no_mangle]
pub extern "C" fn Java_com_limusic_desktop_AudioBridge_nativeOnTrackFailed(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    if let Some(tx) = get_event_sender() {
        let _ = tx.send(PlayerEvent::TrackFailed("Audio playback failed on Android".to_string()));
    }
}
