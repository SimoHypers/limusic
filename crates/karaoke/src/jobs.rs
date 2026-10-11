use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct PrepareRequest {
    pub video_id: String,
    pub url: String,
    pub headers: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct DesiredSet {
    items: Vec<PrepareRequest>,
}

impl DesiredSet {
    pub fn replace(&mut self, reqs: Vec<PrepareRequest>) -> Vec<String> {
        let mut new_items = Vec::new();
        let mut seen = HashSet::new();
        for r in reqs {
            if seen.insert(r.video_id.clone()) {
                new_items.push(r);
                if new_items.len() >= 2 {
                    break;
                }
            }
        }
        let old_ids: HashSet<String> = self.items.iter().map(|i| i.video_id.clone()).collect();
        let new_ids: HashSet<String> = new_items.iter().map(|i| i.video_id.clone()).collect();
        let removed: Vec<String> = old_ids.difference(&new_ids).cloned().collect();
        self.items = new_items;
        removed
    }

    pub fn next_pending(&self, is_ready: impl Fn(&str) -> bool) -> Option<PrepareRequest> {
        for item in &self.items {
            if !is_ready(&item.video_id) {
                return Some(item.clone());
            }
        }
        None
    }

    pub fn complete(&mut self, video_id: &str) {
        self.items.retain(|i| i.video_id != video_id);
    }

    pub fn clear(&mut self) {
        self.items.clear();
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
        let r1 = PrepareRequest { video_id: "v1".into(), url: "u1".into(), headers: HashMap::new() };
        let r2 = PrepareRequest { video_id: "v2".into(), url: "u2".into(), headers: HashMap::new() };
        let r3 = PrepareRequest { video_id: "v1".into(), url: "u1".into(), headers: HashMap::new() };
        let r4 = PrepareRequest { video_id: "v3".into(), url: "u3".into(), headers: HashMap::new() };

        let removed = set.replace(vec![r1.clone(), r2.clone(), r3, r4]);
        assert_eq!(set.len(), 2);
        assert_eq!(set.ids(), vec!["v1".to_string(), "v2".to_string()]);
        assert!(removed.is_empty());

        let r5 = PrepareRequest { video_id: "v3".into(), url: "u3".into(), headers: HashMap::new() };
        let removed2 = set.replace(vec![r5]);
        assert_eq!(set.len(), 1);
        assert_eq!(set.ids(), vec!["v3".to_string()]);
        assert!(removed2.contains(&"v1".to_string()));
        assert!(removed2.contains(&"v2".to_string()));
    }

    #[test]
    fn test_desired_set_next_pending() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest { video_id: "v1".into(), url: "u1".into(), headers: HashMap::new() };
        let r2 = PrepareRequest { video_id: "v2".into(), url: "u2".into(), headers: HashMap::new() };
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
        let r1 = PrepareRequest { video_id: "v1".into(), url: "u1".into(), headers: HashMap::new() };
        let r2 = PrepareRequest { video_id: "v2".into(), url: "u2".into(), headers: HashMap::new() };
        set.replace(vec![r1, r2]);

        assert!(set.contains("v1"));
        set.complete("v1");
        assert!(!set.contains("v1"));
        assert!(set.contains("v2"));

        set.clear();
        assert!(!set.contains("v2"));
    }

    #[test]
    fn test_desired_set_reorder() {
        let mut set = DesiredSet::default();
        let r1 = PrepareRequest { video_id: "v1".into(), url: "u1".into(), headers: HashMap::new() };
        let r2 = PrepareRequest { video_id: "v2".into(), url: "u2".into(), headers: HashMap::new() };
        set.replace(vec![r1.clone(), r2.clone()]);
        assert_eq!(set.ids(), vec!["v1".to_string(), "v2".to_string()]);

        let removed = set.replace(vec![r2, r1]);
        assert!(removed.is_empty());
        assert_eq!(set.ids(), vec!["v2".to_string(), "v1".to_string()]);
    }
}
