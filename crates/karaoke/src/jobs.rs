use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// The longest track karaoke will prepare, in seconds (15 minutes).
///
/// The pipeline holds a whole track in memory several times over. Per stereo sample-frame:
/// - decoding: the f32 interleaved read plus the left/right copies, 16 B;
/// - separation: the padded mixture, result and divider buffers plus the output channels, about
///   36 B;
/// - encoding: the i32 interleaved samples plus the FLAC stream and its byte sink, about 20 B.
///
/// With the input still alive that is roughly 80-90 B per frame. At 44.1 kHz that is about
/// 3.7 MB per second of audio, so **≈ 3.3 GB at 15 minutes**, already a lot next to a browser on
/// an 8 GB machine. A 1-hour mix would need over 13 GB (and a 1.3 GB temporary WAV). Longer
/// tracks are skipped (see [`too_long`]) and the decoder stops a few seconds past the cap, so a
/// track with no known duration is refused before it can use more.
pub const MAX_TRACK_SECS: f64 = 900.0;

/// The pipeline's error for a track over [`MAX_TRACK_SECS`] (matched by the app, which shows it as
/// "too long" rather than as a failure to retry).
pub const TOO_LONG: &str = "track too long for karaoke";

/// A track whose known duration is over [`MAX_TRACK_SECS`]: never queued for preparation.
pub fn too_long(expected_secs: Option<f64>) -> bool {
    expected_secs.is_some_and(|s| s > MAX_TRACK_SECS)
}

#[derive(Clone, Debug)]
pub struct PrepareRequest {
    pub video_id: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub expected_secs: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct DesiredSet {
    items: Vec<PrepareRequest>,
    failed: HashSet<String>,
}

impl DesiredSet {
    pub const MAX_DESIRED: usize = 3;

    pub fn replace(&mut self, reqs: Vec<PrepareRequest>) -> Vec<String> {
        let mut new_items = Vec::new();
        let mut seen = HashSet::new();
        for r in reqs {
            if seen.insert(r.video_id.clone()) {
                new_items.push(r);
                if new_items.len() >= Self::MAX_DESIRED {
                    break;
                }
            }
        }
        let old_ids: HashSet<String> = self.items.iter().map(|i| i.video_id.clone()).collect();
        let new_ids: HashSet<String> = new_items.iter().map(|i| i.video_id.clone()).collect();
        let removed: Vec<String> = old_ids.difference(&new_ids).cloned().collect();
        self.items = new_items;
        self.failed.retain(|id| new_ids.contains(id));
        removed
    }

    pub fn next_pending(&self, is_ready: impl Fn(&str) -> bool) -> Option<PrepareRequest> {
        for item in &self.items {
            if !self.failed.contains(&item.video_id) && !is_ready(&item.video_id) {
                return Some(item.clone());
            }
        }
        None
    }

    pub fn complete(&mut self, video_id: &str) {
        self.items.retain(|i| i.video_id != video_id);
        self.failed.remove(video_id);
    }

    pub fn fail(&mut self, video_id: &str) {
        if self.items.iter().any(|i| i.video_id == video_id) {
            self.failed.insert(video_id.to_string());
        }
    }

    pub fn clear_failed(&mut self) -> usize {
        let count = self.failed.len();
        self.failed.clear();
        count
    }

    pub fn failed_ids(&self) -> &HashSet<String> {
        &self.failed
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.failed.clear();
    }

    pub fn contains(&self, video_id: &str) -> bool {
        self.items.iter().any(|i| i.video_id == video_id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn ids(&self) -> Vec<String> {
        self.items.iter().map(|i| i.video_id.clone()).collect()
    }
}

pub const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

#[derive(Clone, Debug, PartialEq)]
pub enum RetryState {
    Waiting(Instant),
    Exhausted,
    Active,
}

#[derive(Default, Debug)]
pub struct RetryTracker {
    entries: HashMap<String, (usize, RetryState, String)>,
}

impl RetryTracker {
    pub fn on_failure(&mut self, id: &str, url: &str, now: Instant) {
        let entry = self.entries.entry(id.to_string()).or_insert((0, RetryState::Active, url.to_string()));
        entry.2 = url.to_string();
        entry.0 += 1;
        let attempts = entry.0;
        if attempts <= RETRY_DELAYS.len() {
            entry.1 = RetryState::Waiting(now + RETRY_DELAYS[attempts - 1]);
        } else {
            entry.1 = RetryState::Exhausted;
        }
    }

    pub fn refresh(&mut self, id: &str, url: &str) -> bool {
        if let Some(entry) = self.entries.get(id) {
            if entry.2 != url {
                self.entries.remove(id);
                return true;
            }
        }
        false
    }

    pub fn on_success(&mut self, id: &str) {
        self.entries.remove(id);
    }

    pub fn is_blocked(&self, id: &str) -> bool {
        if let Some((_, state, _)) = self.entries.get(id) {
            matches!(state, RetryState::Waiting(_) | RetryState::Exhausted)
        } else {
            false
        }
    }

    pub fn take_due(&mut self, now: Instant) -> Vec<String> {
        let mut due_ids = Vec::new();
        for (id, (_, state, _)) in &mut self.entries {
            if let RetryState::Waiting(due) = *state {
                if due <= now {
                    due_ids.push(id.clone());
                }
            }
        }
        for id in &due_ids {
            if let Some((_, state, _)) = self.entries.get_mut(id) {
                *state = RetryState::Active;
            }
        }
        due_ids
    }

    pub fn next_due(&self, now: Instant) -> Option<Duration> {
        self.entries
            .values()
            .filter_map(|(_, state, _)| {
                if let RetryState::Waiting(due) = *state {
                    Some(due.saturating_duration_since(now))
                } else {
                    None
                }
            })
            .min()
    }

    pub fn reset(&mut self) {
        self.entries.clear();
    }

    pub fn retain_wanted(&mut self, ids: &[String]) {
        let wanted: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
        self.entries.retain(|id, _| wanted.contains(id.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desired_set_replace_and_dedup() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r2 = PrepareRequest {
            video_id: "v2".into(),
            url: "u2".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r3 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r4 = PrepareRequest {
            video_id: "v3".into(),
            url: "u3".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r6 = PrepareRequest {
            video_id: "v4".into(),
            url: "u4".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };

        let removed = set.replace(vec![r1.clone(), r2.clone(), r3, r4, r6]);
        assert_eq!(set.len(), 3);
        assert_eq!(set.ids(), vec!["v1".to_string(), "v2".to_string(), "v3".to_string()]);
        assert!(removed.is_empty());

        let r5 = PrepareRequest {
            video_id: "v5".into(),
            url: "u5".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let removed2 = set.replace(vec![r5]);
        assert_eq!(set.len(), 1);
        assert_eq!(set.ids(), vec!["v5".to_string()]);
        assert!(removed2.contains(&"v1".to_string()));
        assert!(removed2.contains(&"v2".to_string()));
        assert!(removed2.contains(&"v3".to_string()));
    }

    #[test]
    fn test_desired_set_max_desired_3() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r2 = PrepareRequest {
            video_id: "v2".into(),
            url: "u2".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r3 = PrepareRequest {
            video_id: "v3".into(),
            url: "u3".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r4 = PrepareRequest {
            video_id: "v4".into(),
            url: "u4".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };

        set.replace(vec![r1, r2, r3, r4]);
        assert_eq!(set.len(), 3);
        assert_eq!(set.ids(), vec!["v1".to_string(), "v2".to_string(), "v3".to_string()]);
        assert!(!set.contains("v4"));
    }

    #[test]
    fn test_desired_set_next_pending() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r2 = PrepareRequest {
            video_id: "v2".into(),
            url: "u2".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        set.replace(vec![r1, r2]);

        let next = set.next_pending(|id| id == "v1");
        assert!(next.is_some());
        assert_eq!(next.unwrap().video_id, "v2");

        let next_all_ready = set.next_pending(|_| true);
        assert!(next_all_ready.is_none());
    }

    #[test]
    fn test_desired_set_complete_and_clear() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r2 = PrepareRequest {
            video_id: "v2".into(),
            url: "u2".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        set.replace(vec![r1, r2]);

        assert!(set.contains("v1"));
        set.complete("v1");
        assert!(!set.contains("v1"));
        assert!(set.contains("v2"));

        set.clear();
        assert!(!set.contains("v2"));
    }

    #[test]
    fn test_desired_set_clear_failed() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r2 = PrepareRequest {
            video_id: "v2".into(),
            url: "u2".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        set.replace(vec![r1, r2]);

        set.fail("v1");
        let next = set.next_pending(|_| false);
        assert!(next.is_some());
        assert_eq!(next.unwrap().video_id, "v2");

        let count = set.clear_failed();
        assert_eq!(count, 1);

        let next2 = set.next_pending(|_| false);
        assert!(next2.is_some());
        assert_eq!(next2.unwrap().video_id, "v1");
    }

    #[test]
    fn test_desired_set_reorder() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest {
            video_id: "v1".into(),
            url: "u1".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        let r2 = PrepareRequest {
            video_id: "v2".into(),
            url: "u2".into(),
            headers: HashMap::new(),
            expected_secs: None,
        };
        set.replace(vec![r1.clone(), r2.clone()]);
        assert_eq!(set.ids(), vec!["v1".to_string(), "v2".to_string()]);

        let removed = set.replace(vec![r2, r1]);
        assert!(removed.is_empty());
        assert_eq!(set.ids(), vec!["v2".to_string(), "v1".to_string()]);
    }

    #[test]
    fn test_retry_tracker_behavior() {
        let mut tracker = RetryTracker::default();
        let now = Instant::now();

        // 1st failure -> Waiting (5s)
        tracker.on_failure("v1", "u1", now);
        assert!(tracker.is_blocked("v1"));
        assert_eq!(tracker.next_due(now), Some(Duration::from_secs(5)));

        // take_due before due time returns nothing, after returns the id
        assert!(tracker.take_due(now + Duration::from_secs(4)).is_empty());
        assert!(tracker.is_blocked("v1"));
        let due = tracker.take_due(now + Duration::from_secs(5));
        assert_eq!(due, vec!["v1".to_string()]);

        // an Active id is not blocked and a new failure increments its attempts
        assert!(!tracker.is_blocked("v1"));
        tracker.on_failure("v1", "u1", now); // 2nd failure (15s)
        assert_eq!(tracker.next_due(now), Some(Duration::from_secs(15)));

        // 3rd, 4th, 5th failure -> Exhausted
        let _ = tracker.take_due(now + Duration::from_secs(100));
        tracker.on_failure("v1", "u1", now); // 3rd (30s)
        let _ = tracker.take_due(now + Duration::from_secs(100));
        tracker.on_failure("v1", "u1", now); // 4th (60s)
        let _ = tracker.take_due(now + Duration::from_secs(100));
        tracker.on_failure("v1", "u1", now); // 5th failure -> Exhausted
        assert!(tracker.is_blocked("v1"));

        // on_success removes the entry
        tracker.on_success("v1");
        assert!(!tracker.is_blocked("v1"));

        // retain_wanted drops unknown ids
        tracker.on_failure("v1", "u1", now);
        tracker.on_failure("v2", "u1", now);
        tracker.retain_wanted(&["v1".to_string()]);
        assert!(tracker.is_blocked("v1"));
        assert!(!tracker.is_blocked("v2"));

        tracker.reset();
        assert!(!tracker.is_blocked("v1"));
    }

    #[test]
    fn test_retry_tracker_refresh() {
        let mut tracker = RetryTracker::default();
        let now = Instant::now();

        // refresh for an unknown id returns false
        assert!(!tracker.refresh("v1", "u1"));

        tracker.on_failure("v1", "u1", now);
        assert!(tracker.is_blocked("v1"));

        // refresh with the same url changes nothing
        assert!(!tracker.refresh("v1", "u1"));
        assert!(tracker.is_blocked("v1"));

        // refresh with a different url unblocks and resets attempts
        assert!(tracker.refresh("v1", "u2"));
        assert!(!tracker.is_blocked("v1"));
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub video_id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

pub fn build_slots(
    desired_ids: &[String],
    running: Option<(&str, f64)>,
    failed: &HashSet<String>,
    is_cached: impl Fn(&str) -> bool,
) -> Vec<Slot> {
    desired_ids
        .iter()
        .map(|id| {
            if failed.contains(id) {
                Slot { video_id: id.clone(), state: "error".to_string(), progress: None }
            } else if is_cached(id) {
                Slot { video_id: id.clone(), state: "ready".to_string(), progress: None }
            } else if let Some((r_id, prog)) = running {
                if r_id == id {
                    Slot {
                        video_id: id.clone(),
                        state: "preparing".to_string(),
                        progress: Some(prog),
                    }
                } else {
                    Slot { video_id: id.clone(), state: "queued".to_string(), progress: None }
                }
            } else {
                Slot { video_id: id.clone(), state: "queued".to_string(), progress: None }
            }
        })
        .collect()
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn test_build_slots() {
        let desired = vec!["v1".to_string(), "v2".to_string(), "v3".to_string()];
        let mut failed = HashSet::new();
        failed.insert("v3".to_string());

        let slots = build_slots(&desired, Some(("v2", 0.5)), &failed, |id| id == "v1");

        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].video_id, "v1");
        assert_eq!(slots[0].state, "ready");
        assert_eq!(slots[0].progress, None);

        assert_eq!(slots[1].video_id, "v2");
        assert_eq!(slots[1].state, "preparing");
        assert_eq!(slots[1].progress, Some(0.5));

        assert_eq!(slots[2].video_id, "v3");
        assert_eq!(slots[2].state, "error");
        assert_eq!(slots[2].progress, None);
    }
}
