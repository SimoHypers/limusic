//! Watch time for a registered play (#764).
//!
//! After the playback ping music.youtube.com reports which stretches of the track were heard, as
//! `st`/`et` lists, and closes the play with `final=1`. Without these YouTube knew only that a play
//! started. We report every [`EVERY`], on a pause, and once more when the play ends; the official
//! client reports at 10, 20 and 30 s first, which we skip to send about half as many requests.
//!
//! This is the bookkeeping, one [`Watch`] per play. `AppState` feeds it position ticks and sends
//! what it returns.

use std::time::{Duration, Instant};

use innertube::Watchtime;

/// Between reports while playing: YouTube's own `videostatsDefaultFlushIntervalSeconds`.
pub const EVERY: Duration = Duration::from_secs(40);

/// Where a play's reports go: the watch-time URL and the client it was issued to.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub url: String,
    pub client: String,
}

/// A report to send.
#[derive(Debug)]
pub struct Due {
    pub target: Target,
    pub cpn: String,
    pub report: Watchtime,
}

pub struct Watch {
    /// The play's nonce, so a late [`Watch::arm`] can't land on the play after it.
    pub cpn: String,
    started: Instant,
    /// Stretches already ended and not yet reported.
    done: Vec<(f64, f64)>,
    /// The stretch being heard now.
    open: Option<(f64, f64)>,
    position: f64,
    /// Set once the playback ping is out. Nothing is reported for a play that never registered.
    target: Option<Target>,
    next: Instant,
}

impl Watch {
    pub fn new(cpn: String, now: Instant) -> Self {
        Watch {
            cpn,
            started: now,
            done: Vec::new(),
            open: None,
            position: 0.0,
            target: None,
            next: now + EVERY,
        }
    }

    /// The playback ping went out: report from now on, the first one [`EVERY`] later.
    pub fn arm(&mut self, target: Target, now: Instant) {
        self.target = Some(target);
        self.next = now + EVERY;
    }

    /// A position tick. mpv ticks several times a second while audio plays, so any other move
    /// (a jump back, a jump of seconds forward, a tick while paused) ends the stretch, the same rule
    /// as `AppState::hear`.
    pub fn tick(&mut self, pos: f64, playing: bool, now: Instant, volume: i64) -> Option<Due> {
        match self.open {
            Some((start, end)) if playing && pos >= end && pos - end < 2.0 => {
                self.open = Some((start, pos));
            }
            _ => {
                self.shut();
                self.open = playing.then_some((pos, pos));
            }
        }
        self.position = pos;
        if now < self.next {
            return None;
        }
        self.report(now, false, volume)
    }

    /// Paused: report what was heard up to here.
    pub fn pause(&mut self, now: Instant, volume: i64) -> Option<Due> {
        self.shut();
        self.report(now, false, volume)
    }

    /// The play is over (another started, or this one looped): report the rest and close it.
    pub fn close(mut self, now: Instant, volume: i64) -> Option<Due> {
        self.shut();
        self.report(now, true, volume)
    }

    fn shut(&mut self) {
        if let Some((start, end)) = self.open.take().filter(|(s, e)| e > s) {
            self.done.push((start, end));
        }
    }

    fn report(&mut self, now: Instant, last: bool, volume: i64) -> Option<Due> {
        let target = self.target.clone()?;
        let playing = self.open.is_some();
        let mut stretches = std::mem::take(&mut self.done);
        if let Some((start, end)) = self.open {
            if end > start {
                stretches.push((start, end));
            }
            // Heard on from here: the next report starts where this one ends.
            self.open = Some((end, end));
        }
        if stretches.is_empty() && !last {
            return None;
        }
        self.next = now + EVERY;
        Some(Due {
            target,
            cpn: self.cpn.clone(),
            report: Watchtime {
                stretches,
                position: self.position,
                elapsed: now.duration_since(self.started).as_secs_f64(),
                next_in: (playing && !last).then_some(EVERY.as_secs_f64()),
                last,
                volume,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays `from..to` in 0.25 s ticks, returning every report it produced.
    fn play(w: &mut Watch, t0: Instant, from: f64, to: f64, at: &mut f64) -> Vec<Due> {
        let mut out = Vec::new();
        let mut pos = from;
        while pos <= to {
            out.extend(w.tick(pos, true, t0 + Duration::from_secs_f64(*at), 50));
            pos += 0.25;
            *at += 0.25;
        }
        out
    }

    #[test]
    fn reports_every_stretch_heard_once_and_closes_the_play() {
        let t0 = Instant::now();
        let target = Target { url: "u".into(), client: "WEB_REMIX".into() };
        let mut w = Watch::new("cpn".into(), t0);
        let mut at = 0.0;
        // Nothing before the playback ping, but the time heard before it is kept.
        assert!(play(&mut w, t0, 0.0, 30.0, &mut at).is_empty());
        w.arm(target.clone(), t0 + Duration::from_secs(30));

        // Every 40 s from the ping, from where the last report ended.
        let due = play(&mut w, t0, 30.25, 75.0, &mut at);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].report.stretches, vec![(0.0, 70.0)]);
        assert_eq!(due[0].report.next_in, Some(40.0));
        assert_eq!((due[0].target.clone(), due[0].cpn.as_str()), (target, "cpn"));

        // A seek forward ends the stretch, a pause reports what is left.
        assert!(play(&mut w, t0, 120.0, 130.0, &mut at).is_empty());
        let paused = w.pause(t0 + Duration::from_secs_f64(at), 50).unwrap().report;
        assert_eq!(paused.stretches, vec![(70.0, 75.0), (120.0, 130.0)]);
        assert_eq!((paused.next_in, paused.last), (None, false));
        assert!(w.pause(t0 + Duration::from_secs_f64(at), 50).is_none(), "nothing new");

        // Resumed and skipped: the closing report has the rest and `final`.
        play(&mut w, t0, 130.0, 140.0, &mut at);
        let last = w.close(t0 + Duration::from_secs_f64(at), 50).unwrap().report;
        assert_eq!(last.stretches, vec![(130.0, 140.0)]);
        assert!(last.last && last.next_in.is_none());
    }

    #[test]
    fn a_play_that_never_registered_reports_nothing() {
        let t0 = Instant::now();
        let mut w = Watch::new("cpn".into(), t0);
        let mut at = 0.0;
        assert!(play(&mut w, t0, 0.0, 100.0, &mut at).is_empty());
        assert!(w.pause(t0, 50).is_none());
        assert!(w.close(t0, 50).is_none());
    }
}
