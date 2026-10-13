use std::collections::{HashMap, HashSet};

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
