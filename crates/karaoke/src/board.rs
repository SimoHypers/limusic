//! The state the karaoke worker shares with the rest of the app, and the rules for touching it.
//!
//! Playback reads it on every track start, so:
//! - `is_enabled` is an atomic: it never blocks and never panics.
//! - The mutex only ever guards short bookkeeping. Model load, inference and disk I/O happen with
//!   it released; callers snapshot what they need, work, then re-lock to publish.
//! - A panic while it is held does not poison it for everyone else: every lock recovers with
//!   `PoisonError::into_inner` (the bookkeeping stays consistent, since each critical section is
//!   a handful of field writes).
//! - A job runs under [`isolate`], so a panic fails that job instead of killing the worker.

use crate::jobs::{DesiredSet, PrepareRequest};
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The job the worker is on. `cancel` is checked by the job between steps.
#[derive(Clone, Debug)]
pub struct Running {
    pub video_id: String,
    pub cancel: Arc<AtomicBool>,
    pub seq: u64,
}

/// Bookkeeping behind the board's mutex.
#[derive(Default)]
pub struct BoardState {
    pub desired: DesiredSet,
    pub last_reqs: Vec<PrepareRequest>,
    pub running: Option<Running>,
    pub job_seq: u64,
    pub last_progress: f64,
    pub announced_ready: HashSet<String>,
    /// Set by [`JobBoard::notify`]; consumed by [`JobBoard::wait`], so a wake-up sent while the
    /// worker was busy is not lost.
    woken: bool,
}

#[derive(Default)]
pub struct JobBoard {
    enabled: AtomicBool,
    state: Mutex<BoardState>,
    wake: Condvar,
}

impl JobBoard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock-free: safe on the playback hot path.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn lock(&self) -> MutexGuard<'_, BoardState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A short critical section. Never do I/O or heavy work inside `f`.
    pub fn with<R>(&self, f: impl FnOnce(&mut BoardState) -> R) -> R {
        f(&mut self.lock())
    }

    /// Wakes the worker. Remembered if the worker is not waiting right now.
    pub fn notify(&self) {
        self.lock().woken = true;
        self.wake.notify_all();
    }

    /// Turns karaoke on or off without waiting for the worker. Off clears the queue and cancels
    /// the running job; the worker sees the flag and unloads when its current step ends.
    pub fn set_enabled(&self, on: bool) {
        // The flag goes first, then the lock: `begin` reads the flag under the lock, so a job
        // either starts before this and is cancelled below, or sees `false` and never starts.
        self.enabled.store(on, Ordering::SeqCst);
        {
            let mut s = self.lock();
            if !on {
                s.desired.clear();
                s.last_reqs.clear();
                s.announced_ready.clear();
                s.last_progress = 0.0;
                if let Some(r) = s.running.take() {
                    r.cancel.store(true, Ordering::SeqCst);
                }
            }
            s.woken = true;
        }
        self.wake.notify_all();
    }

    /// Worker side: blocks until notified, or until `timeout`. Returns `true` on timeout.
    pub fn wait(&self, timeout: Option<Duration>) -> bool {
        let mut s = self.lock();
        while !s.woken {
            match timeout {
                None => s = self.wake.wait(s).unwrap_or_else(PoisonError::into_inner),
                Some(t) => {
                    let (g, r) =
                        self.wake.wait_timeout(s, t).unwrap_or_else(PoisonError::into_inner);
                    s = g;
                    if r.timed_out() && !s.woken {
                        return true;
                    }
                }
            }
        }
        s.woken = false;
        false
    }

    /// Marks `video_id` as the running job. `None` if karaoke was turned off meanwhile.
    pub fn begin(&self, video_id: &str) -> Option<Running> {
        let mut s = self.lock();
        if !self.is_enabled() {
            return None;
        }
        s.job_seq = s.job_seq.wrapping_add(1);
        let running = Running {
            video_id: video_id.to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            seq: s.job_seq,
        };
        s.running = Some(running.clone());
        s.last_progress = 0.0;
        Some(running)
    }

    /// Clears the running job if it is still `seq` (off or clear-cache may have taken it already).
    pub fn finish(&self, seq: u64) {
        let mut s = self.lock();
        if s.running.as_ref().is_some_and(|r| r.seq == seq) {
            s.running = None;
        }
        s.last_progress = 0.0;
    }
}

/// Runs one job so that a panic inside it becomes an ordinary error.
pub fn isolate<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(anyhow::anyhow!("karaoke job panicked: {what}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;

    fn req(id: &str) -> PrepareRequest {
        PrepareRequest {
            video_id: id.into(),
            url: String::new(),
            headers: HashMap::new(),
            expected_secs: None,
        }
    }

    /// While a slow job runs (lock released, as the worker does), everything playback and the
    /// commands do returns at once, and turning karaoke off cancels the job.
    #[test]
    fn nothing_waits_for_a_running_job() {
        let board = Arc::new(JobBoard::new());
        board.set_enabled(true);
        board.with(|s| {
            s.desired.replace(vec![req("a")]);
        });
        let running = board.begin("a").expect("enabled");

        let worker = {
            let board = board.clone();
            let cancel = running.cancel.clone();
            std::thread::spawn(move || {
                // The "job": 400 ms of work with the lock released, checking cancel between steps.
                for _ in 0..40 {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                board.finish(running.seq);
            })
        };
        std::thread::sleep(Duration::from_millis(30));

        let t = Instant::now();
        assert!(board.is_enabled());
        board.with(|s| {
            s.desired.replace(vec![req("b"), req("c")]);
        });
        board.notify();
        board.set_enabled(false);
        assert!(t.elapsed() < Duration::from_millis(50), "took {:?}", t.elapsed());
        assert!(!board.is_enabled());
        assert!(board.with(|s| s.running.is_none()), "off takes the running job");

        worker.join().unwrap();
    }

    #[test]
    fn a_panic_under_the_lock_does_not_poison_it() {
        let board = Arc::new(JobBoard::new());
        board.set_enabled(true);
        let b = board.clone();
        let r = std::thread::spawn(move || b.with(|_| panic!("boom while locked"))).join();
        assert!(r.is_err(), "the thread did panic");

        // Everyone else carries on.
        assert!(board.is_enabled());
        board.with(|s| {
            s.desired.replace(vec![req("x")]);
        });
        assert_eq!(board.with(|s| s.desired.ids()), vec!["x".to_string()]);
        assert!(board.begin("x").is_some());
        board.set_enabled(false);
    }

    #[test]
    fn a_panicking_job_fails_and_the_next_one_runs() {
        let failed = isolate::<()>(|| panic!("inference blew up"));
        let msg = failed.unwrap_err().to_string();
        assert!(msg.contains("panicked") && msg.contains("inference blew up"), "{msg}");

        let next = isolate(|| Ok(7));
        assert_eq!(next.unwrap(), 7);
    }

    #[test]
    fn a_notify_sent_before_wait_is_not_lost() {
        let board = JobBoard::new();
        board.notify();
        let t = Instant::now();
        assert!(!board.wait(Some(Duration::from_secs(2))), "returned as woken, not timed out");
        assert!(t.elapsed() < Duration::from_millis(500));
        // Consumed: the next wait times out.
        assert!(board.wait(Some(Duration::from_millis(20))));
    }

    #[test]
    fn no_job_starts_after_karaoke_is_turned_off() {
        let board = JobBoard::new();
        board.set_enabled(true);
        board.set_enabled(false);
        assert!(board.begin("a").is_none());
    }

    #[test]
    fn finish_ignores_a_job_that_was_already_taken() {
        let board = JobBoard::new();
        board.set_enabled(true);
        let first = board.begin("a").unwrap();
        let second = board.begin("b").unwrap();
        board.finish(first.seq);
        assert_eq!(
            board.with(|s| s.running.as_ref().map(|r| r.video_id.clone())),
            Some("b".into())
        );
        board.finish(second.seq);
        assert!(board.with(|s| s.running.is_none()));
    }
}
