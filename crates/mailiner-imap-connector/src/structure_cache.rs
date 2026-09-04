//! Bounded LRU of converted BODYSTRUCTURE trees.

use std::collections::{HashMap, VecDeque};

use mailiner_core::{BodyPart, FolderId, MessageId};

/// Session-only BODYSTRUCTURE rows kept for hot messages.
pub(crate) const MAX_CACHED_STRUCTURES: usize = 500;

pub(crate) type StructureCacheKey = (FolderId, MessageId);

/// In-memory LRU of BODYSTRUCTURE trees keyed by folder + UID.
pub(crate) struct StructureCache {
    entries: HashMap<StructureCacheKey, BodyPart>,
    lru: VecDeque<StructureCacheKey>,
    cap: usize,
}

impl Default for StructureCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StructureCache {
    pub(crate) fn new() -> Self {
        Self::with_capacity(MAX_CACHED_STRUCTURES)
    }

    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    pub(crate) fn get(&mut self, key: &StructureCacheKey) -> Option<&BodyPart> {
        if !self.entries.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.entries.get(key)
    }

    pub(crate) fn insert(&mut self, key: StructureCacheKey, part: BodyPart) {
        if self.entries.insert(key.clone(), part).is_some() {
            self.touch(&key);
            return;
        }
        self.lru.push_back(key);
        self.evict_overflow();
    }

    pub(crate) fn remove(&mut self, key: &StructureCacheKey) -> Option<BodyPart> {
        let part = self.entries.remove(key)?;
        if let Some(i) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(i);
        }
        Some(part)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&StructureCacheKey) -> bool) {
        self.entries.retain(|k, _| keep(k));
        self.lru.retain(|k| self.entries.contains_key(k));
    }

    fn touch(&mut self, key: &StructureCacheKey) {
        if let Some(key) = self
            .lru
            .iter()
            .position(|k| k == key)
            .and_then(|i| self.lru.remove(i))
        {
            self.lru.push_back(key);
        }
    }

    fn evict_overflow(&mut self) {
        while self.entries.len() > self.cap {
            if let Some(old) = self.lru.pop_front() {
                self.entries.remove(&old);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
impl StructureCache {
    fn contains(&self, key: &StructureCacheKey) -> bool {
        self.entries.contains_key(key)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(folder: &str, uid: &str) -> StructureCacheKey {
        let folder = FolderId::new(folder);
        let id = MessageId::new(folder.clone(), uid);
        (folder, id)
    }

    fn part(subtype: &str) -> BodyPart {
        BodyPart {
            type_: "text".into(),
            subtype: subtype.into(),
            ..Default::default()
        }
    }

    #[test]
    fn get_keeps_hot_entry_when_over_cap() {
        let mut cache = StructureCache::with_capacity(2);
        let a = key("INBOX", "1");
        let b = key("INBOX", "2");
        let c = key("INBOX", "3");
        cache.insert(a.clone(), part("plain"));
        cache.insert(b.clone(), part("plain"));
        assert!(cache.get(&a).is_some());
        cache.insert(c.clone(), part("html"));
        assert!(cache.contains(&a), "recent get should keep a");
        assert!(!cache.contains(&b), "oldest unused should evict");
        assert!(cache.contains(&c));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn reinsert_updates_value_and_recency() {
        let mut cache = StructureCache::with_capacity(2);
        let a = key("INBOX", "1");
        let b = key("INBOX", "2");
        let c = key("Sent", "3");
        cache.insert(a.clone(), part("plain"));
        cache.insert(b.clone(), part("plain"));
        cache.insert(a.clone(), part("html"));
        cache.insert(c.clone(), part("plain"));
        assert_eq!(cache.get(&a).map(|p| p.subtype.as_str()), Some("html"));
        assert!(!cache.contains(&b), "untouched b should evict");
        assert!(cache.contains(&c));
    }

    #[test]
    fn retain_drops_folder_and_keeps_lru_order() {
        let mut cache = StructureCache::with_capacity(3);
        let inbox_a = key("INBOX", "1");
        let sent = key("Sent", "2");
        let inbox_b = key("INBOX", "3");
        cache.insert(inbox_a.clone(), part("plain"));
        cache.insert(sent.clone(), part("plain"));
        cache.insert(inbox_b.clone(), part("plain"));
        cache.retain(|(fid, _)| fid.as_str() != "INBOX");
        assert!(!cache.contains(&inbox_a));
        assert!(!cache.contains(&inbox_b));
        assert!(cache.contains(&sent));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn remove_drops_entry() {
        let mut cache = StructureCache::with_capacity(2);
        let a = key("INBOX", "1");
        cache.insert(a.clone(), part("plain"));
        assert!(cache.remove(&a).is_some());
        assert!(!cache.contains(&a));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn default_cap_matches_previous_limit() {
        let mut cache = StructureCache::new();
        for i in 0..=MAX_CACHED_STRUCTURES {
            cache.insert(key("INBOX", &i.to_string()), part("plain"));
        }
        assert_eq!(cache.len(), MAX_CACHED_STRUCTURES);
        assert!(!cache.contains(&key("INBOX", "0")));
        assert!(cache.contains(&key("INBOX", &MAX_CACHED_STRUCTURES.to_string())));
    }
}
