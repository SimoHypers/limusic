//! Session-only sleep timer. Monotonic deadlines survive wall-clock changes; taking clears once.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum Snapshot {
    Off,
    Duration {
        #[serde(rename = "remainingSeconds")]
        remaining_seconds: u64,
    },
    AfterTrack,
}

#[derive(Default)]
enum Mode {
    #[default]
    Off,
    Deadline(Instant),
    AfterTrack,
}

#[derive(Default)]
pub struct Timer {
    mode: Mutex<Mode>,
}

impl Timer {
    pub fn set_duration(&self, minutes: i64) -> Result<Snapshot, &'static str> {
        self.set_duration_at(minutes, Instant::now())
    }

    fn set_duration_at(&self, minutes: i64, now: Instant) -> Result<Snapshot, &'static str> {
        if !(1..=1440).contains(&minutes) {
            return Err("Sleep timer duration must be from 1 to 1440 minutes.");
        }
        let seconds = minutes as u64 * 60;
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) =
            Mode::Deadline(now + Duration::from_secs(seconds));
        Ok(Snapshot::Duration { remaining_seconds: seconds })
    }

    pub fn after_track(&self) -> Snapshot {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::AfterTrack;
        Snapshot::AfterTrack
    }

    pub fn cancel(&self) -> Snapshot {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::Off;
        Snapshot::Off
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> Snapshot {
        match *self.mode.lock().unwrap_or_else(|e| e.into_inner()) {
            Mode::Off => Snapshot::Off,
            Mode::AfterTrack => Snapshot::AfterTrack,
            Mode::Deadline(deadline) => {
                let remaining = deadline.saturating_duration_since(now);
                Snapshot::Duration {
                    remaining_seconds: remaining.as_secs()
                        + u64::from(remaining.subsec_nanos() != 0),
                }
            }
        }
    }

    pub fn take_if_due(&self) -> bool {
        self.take_if_due_at(Instant::now())
    }

    fn take_if_due_at(&self, now: Instant) -> bool {
        let mut mode = self.mode.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*mode, Mode::Deadline(deadline) if now >= deadline) {
            *mode = Mode::Off;
            true
        } else {
            false
        }
    }

    pub fn take_at_track_end(&self) -> bool {
        let mut mode = self.mode.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*mode, Mode::AfterTrack) {
            *mode = Mode::Off;
            true
        } else {
            false
        }
    }

    pub fn is_after_track(&self) -> bool {
        matches!(*self.mode.lock().unwrap_or_else(|e| e.into_inner()), Mode::AfterTrack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn duration_fires_once_at_the_monotonic_deadline() {
        let timer = Timer::default();
        let now = Instant::now();
        assert_eq!(timer.snapshot_at(now), Snapshot::Off);
        assert_eq!(
            timer.set_duration_at(1, now).unwrap(),
            Snapshot::Duration { remaining_seconds: 60 }
        );
        assert_eq!(
            timer.snapshot_at(now + Duration::from_millis(1)),
            Snapshot::Duration { remaining_seconds: 60 }
        );
        assert!(!timer.take_if_due_at(now + Duration::from_secs(59)));
        assert!(timer.take_if_due_at(now + Duration::from_secs(60)));
        assert!(!timer.take_if_due_at(now + Duration::from_secs(61)));
        assert_eq!(timer.snapshot_at(now), Snapshot::Off);
    }

    #[test]
    fn replacement_and_cancel_prevent_the_old_timer_firing() {
        let timer = Timer::default();
        let now = Instant::now();
        timer.set_duration_at(1, now).unwrap();
        timer.set_duration_at(2, now + Duration::from_secs(30)).unwrap();
        assert!(!timer.take_if_due_at(now + Duration::from_secs(60)));
        assert_eq!(timer.cancel(), Snapshot::Off);
        assert!(!timer.take_if_due_at(now + Duration::from_secs(200)));
        timer.after_track();
        timer.set_duration_at(1, now).unwrap();
        assert!(!timer.take_at_track_end());
        assert!(timer.take_if_due_at(now + Duration::from_secs(60)));
    }

    #[test]
    fn after_track_fires_only_at_track_end_and_clears_once() {
        let timer = Timer::default();
        assert_eq!(timer.after_track(), Snapshot::AfterTrack);
        assert!(timer.is_after_track());
        assert!(!timer.take_if_due_at(Instant::now() + Duration::from_secs(100_000)));
        assert!(timer.take_at_track_end());
        assert!(!timer.take_at_track_end());
        assert!(!timer.is_after_track());
        assert_eq!(timer.snapshot(), Snapshot::Off);
    }

    #[test]
    fn ipc_duration_bounds_reject_without_replacing_the_active_timer() {
        let timer = Timer::default();
        timer.after_track();
        for minutes in [i64::MIN, -1, 0, 1441, i64::MAX] {
            assert!(timer.set_duration(minutes).is_err());
            assert_eq!(timer.snapshot(), Snapshot::AfterTrack);
        }
        assert!(timer.set_duration(1).is_ok());
        assert!(timer.set_duration(1440).is_ok());
    }

    #[test]
    fn snapshot_matches_the_frontend_wire_contract() {
        assert_eq!(serde_json::to_value(Snapshot::Off).unwrap(), serde_json::json!({"mode":"off"}));
        assert_eq!(
            serde_json::to_value(Snapshot::AfterTrack).unwrap(),
            serde_json::json!({"mode":"afterTrack"})
        );
        assert_eq!(
            serde_json::to_value(Snapshot::Duration { remaining_seconds: 60 }).unwrap(),
            serde_json::json!({"mode":"duration", "remainingSeconds":60})
        );
    }
}
