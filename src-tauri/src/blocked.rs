//! The user's blocked-artist list: a JSON array in one `settings` row, like `local_folders`.
//! The filtering itself lives in the innertube crate (`BlockList`); this only persists the list
//! and rebuilds the predicate from it. Plan 046.

use serde::{Deserialize, Serialize};

use crate::db::Db;

const BLOCKED_SETTING: &str = "blocked_artists";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedArtist {
    /// The channel browseId (`UC…`) when the row that was blocked linked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
}

impl BlockedArtist {
    /// Stable handle for "unblock this one". The channel id when there is one, else the name.
    pub fn key(&self) -> &str {
        self.id.as_deref().unwrap_or(&self.name)
    }
}

pub fn list(db: &Db) -> Vec<BlockedArtist> {
    db.get_setting(BLOCKED_SETTING)
        .and_then(|s| serde_json::from_str::<Vec<BlockedArtist>>(&s).ok())
        .unwrap_or_default()
}

fn save(db: &Db, list: &[BlockedArtist]) {
    db.set_setting(BLOCKED_SETTING, &serde_json::to_string(list).unwrap_or_else(|_| "[]".into()));
}

pub fn block(db: &Db, entry: BlockedArtist) -> Vec<BlockedArtist> {
    let mut l = list(db);
    add(&mut l, entry);
    save(db, &l);
    l
}

pub fn unblock(db: &Db, key: &str) -> Vec<BlockedArtist> {
    let mut l = list(db);
    // A name-only row is matched the way `BlockList` matches it, so the row a differently-cased
    // key came from still comes out.
    l.retain(|b| b.key() != key && !(b.id.is_none() && norm(&b.name) == norm(key)));
    save(db, &l);
    l
}

/// How `innertube::BlockList` compares names, kept in step here so the stored list cannot hold two
/// rows the filter treats as one.
fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Add an artist. No-op when the same artist is already blocked. An entry with a channel id
/// supersedes a name-only entry for the same name: the id is the stronger key and keeping both
/// would leave a duplicate row in the settings list.
fn add(l: &mut Vec<BlockedArtist>, entry: BlockedArtist) {
    let name = norm(&entry.name);
    match &entry.id {
        Some(id) => {
            if l.iter().any(|b| b.id.as_deref() == Some(id.as_str())) {
                return;
            }
            l.retain(|b| b.id.is_some() || norm(&b.name) != name);
        }
        // Nothing to add: `BlockList` blocks by name whatever the row's id, so a name already in
        // the list is already blocked. Compared normalized, or "Foo" and "foo" become two rows
        // and removing one leaves the artist blocked by the other.
        None => {
            if l.iter().any(|b| norm(&b.name) == name) {
                return;
            }
        }
    }
    l.push(entry);
}

/// The predicate the transport applies, built from the stored list.
pub fn block_list(db: &Db) -> innertube::BlockList {
    innertube::BlockList::new(list(db).into_iter().map(|b| (b.id, b.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn entry(id: Option<&str>, name: &str) -> BlockedArtist {
        BlockedArtist { id: id.map(str::to_owned), name: name.into() }
    }

    #[test]
    fn blocking_the_same_key_twice_stores_one_row() {
        let d = db();
        block(&d, entry(Some("UCfoo"), "Foo"));
        let l = block(&d, entry(Some("UCfoo"), "Foo renamed"));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name, "Foo");
    }

    #[test]
    fn an_id_entry_supersedes_the_name_only_entry_for_the_same_name() {
        let d = db();
        block(&d, entry(None, "Foo"));
        let l = block(&d, entry(Some("UCfoo"), " foo "));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].key(), "UCfoo");
    }

    #[test]
    fn unblock_works_by_id_and_by_name_and_survives_a_reload() {
        let d = db();
        block(&d, entry(Some("UCfoo"), "Foo"));
        block(&d, entry(None, "Bar"));
        assert_eq!(list(&d).len(), 2);
        assert_eq!(unblock(&d, "UCfoo").len(), 1);
        assert_eq!(list(&d)[0].key(), "Bar");
        assert!(unblock(&d, "Bar").is_empty());
        assert!(list(&d).is_empty());
    }

    #[test]
    fn a_name_is_stored_once_however_it_is_typed() {
        let d = db();
        block(&d, entry(None, "Foo"));
        let l = block(&d, entry(None, " FOO "));
        assert_eq!(l.len(), 1);
        // And the row comes out again whichever casing the caller hands back.
        assert!(unblock(&d, "foo").is_empty());
    }

    #[test]
    fn a_name_already_blocked_by_an_id_entry_is_not_stored_twice() {
        let d = db();
        block(&d, entry(Some("UCfoo"), "Foo"));
        assert_eq!(block(&d, entry(None, "foo")).len(), 1);
    }

    #[test]
    fn the_predicate_is_built_from_what_was_stored() {
        let d = db();
        assert!(block_list(&d).is_empty());
        block(&d, entry(Some("UCfoo"), "Foo"));
        assert!(!block_list(&d).is_empty());
    }
}
