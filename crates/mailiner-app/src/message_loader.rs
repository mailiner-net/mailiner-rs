//! Structure → parse → selective FETCH → TE decode pipeline.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use mailiner_core::connector::EmailConnector;
use mailiner_core::error::{MailinerError, Result};
use mailiner_core::ids::{FolderId, MessageId};
use mailiner_core::models::{LoadedMessage, MessageContent, MessagePart};
use mailiner_mime::{MessageParser, decode_part_content};

/// Skip prefetch when BODYSTRUCTURE size exceeds this (wire octets).
pub(crate) const MAX_PREFETCH_OCTETS: u64 = 2 * 1024 * 1024;
/// Session-only decoded bodies kept for the current message and its neighbors.
const MAX_CACHED_BODIES: usize = 8;

pub(crate) fn within_prefetch_budget(part: &MessagePart) -> bool {
    match part.original_size {
        Some(n) => n <= MAX_PREFETCH_OCTETS,
        None => true,
    }
}

/// Previous and next list indices around `index` (`None` at the edges).
pub(crate) fn adjacent_neighbor_indices(index: usize, total: usize) -> [Option<usize>; 2] {
    if total == 0 || index >= total {
        return [None, None];
    }
    let prev = index.checked_sub(1);
    let next = (index + 1 < total).then_some(index + 1);
    [prev, next]
}

fn prefetch_sections(parts: &[MessagePart]) -> Vec<String> {
    let mut sections: Vec<String> = parts
        .iter()
        .filter(|p| p.should_prefetch() && within_prefetch_budget(p))
        .map(|p| p.section())
        .collect();
    sections.sort();
    sections.dedup();
    sections
}

/// In-memory LRU of decoded bodies so opening a prefetched neighbor does not re-FETCH.
#[derive(Debug)]
pub(crate) struct LoadedMessageCache {
    entries: HashMap<MessageId, Arc<LoadedMessage>>,
    lru: VecDeque<MessageId>,
    cap: usize,
}

impl Default for LoadedMessageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadedMessageCache {
    pub fn new() -> Self {
        Self::with_capacity(MAX_CACHED_BODIES)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    pub fn get(&mut self, id: &MessageId) -> Option<Arc<LoadedMessage>> {
        if self.entries.contains_key(id) {
            self.touch(id);
            self.entries.get(id).cloned()
        } else {
            None
        }
    }

    pub fn contains(&self, id: &MessageId) -> bool {
        self.entries.contains_key(id)
    }

    pub fn insert(&mut self, id: MessageId, loaded: Arc<LoadedMessage>) {
        if self.entries.insert(id.clone(), loaded).is_some() {
            self.touch(&id);
            return;
        }
        self.lru.push_back(id);
        self.evict_overflow();
    }

    pub fn remove(&mut self, id: &MessageId) {
        if self.entries.remove(id).is_some()
            && let Some(i) = self.lru.iter().position(|x| x == id)
        {
            self.lru.remove(i);
        }
    }

    pub fn remove_many(&mut self, ids: &[MessageId]) {
        for id in ids {
            self.remove(id);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
    }

    fn touch(&mut self, id: &MessageId) {
        if let Some(i) = self.lru.iter().position(|x| x == id)
            && let Some(id) = self.lru.remove(i)
        {
            self.lru.push_back(id);
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

/// Load a message: BODYSTRUCTURE → parse parts → FETCH content sections → decode.
pub async fn load_message<C: EmailConnector>(
    connector: &C,
    folder_id: &FolderId,
    message_id: &MessageId,
) -> Result<LoadedMessage> {
    let structure = connector.get_body_structure(folder_id, message_id).await?;
    let parser = MessageParser::with_defaults();
    let mut parts = parser.parse(message_id, &structure);

    let sections = prefetch_sections(&parts);

    if sections.is_empty() {
        return Ok(LoadedMessage {
            envelope_id: message_id.clone(),
            folder_id: folder_id.clone(),
            parts,
        });
    }

    let raw = connector
        .fetch_raw_parts(folder_id, message_id, &sections)
        .await?;

    let mut missing = Vec::new();
    for part in &mut parts {
        if !part.should_prefetch() || !within_prefetch_budget(part) {
            continue;
        }
        let sec = part.section();
        match raw.get(&sec) {
            Some(bytes) => match decode_part_content(
                bytes,
                part.encoding,
                &part.content_type,
                part.charset.as_deref(),
            ) {
                Ok(content) => {
                    part.content = content;
                }
                Err(_) => {
                    missing.push(sec);
                }
            },
            None => {
                missing.push(sec);
            }
        }
    }

    let had_display = parts
        .iter()
        .any(|p| p.is_top_level() && p.is_display_part());
    let any_display = parts.iter().any(|p| {
        p.is_top_level() && p.is_display_part() && !matches!(p.content, MessageContent::Empty)
    });
    if had_display && !any_display {
        return Err(MailinerError::Connector(format!(
            "failed to load content sections: {}",
            missing.join(", ")
        )));
    }

    Ok(LoadedMessage {
        envelope_id: message_id.clone(),
        folder_id: folder_id.clone(),
        parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::connector::MockConnector;
    use mailiner_core::models::PartKind;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn loads_multipart_prefers_html_content() {
        let connector = MockConnector::new();
        let folder = FolderId::new("inbox");
        let msg = MessageId::new(folder.clone(), "1");
        let loaded = block_on(load_message(&connector, &folder, &msg)).unwrap();

        assert_eq!(loaded.envelope_id, msg);
        // Prefetched content: plain + html (not the pdf attachment).
        let html = loaded
            .parts
            .iter()
            .find(|p| p.kind == PartKind::TextHtml)
            .expect("html part");
        match &html.content {
            MessageContent::Text(t) => assert!(t.contains("HTML")),
            other => panic!("expected text, got {:?}", other),
        }
        let att = loaded
            .parts
            .iter()
            .find(|p| p.is_attachment && !p.is_hidden)
            .expect("attachment");
        assert!(matches!(att.content, MessageContent::Empty));
        assert!(loaded.attachments().count() >= 1);
    }

    #[test]
    fn content_parts_decoded_attachment_not_fetched() {
        let connector = MockConnector::new();
        let folder = FolderId::new("inbox");
        let msg = MessageId::new(folder.clone(), "42");
        let loaded = block_on(load_message(&connector, &folder, &msg)).unwrap();

        for p in loaded.attachments() {
            assert!(matches!(p.content, MessageContent::Empty));
        }
        assert!(
            loaded
                .content_parts()
                .any(|p| !matches!(p.content, MessageContent::Empty))
        );
    }

    #[test]
    fn adjacent_neighbor_indices_edges_and_middle() {
        assert_eq!(adjacent_neighbor_indices(0, 0), [None, None]);
        assert_eq!(adjacent_neighbor_indices(0, 1), [None, None]);
        assert_eq!(adjacent_neighbor_indices(5, 3), [None, None]);
        assert_eq!(adjacent_neighbor_indices(0, 3), [None, Some(1)]);
        assert_eq!(adjacent_neighbor_indices(1, 3), [Some(0), Some(2)]);
        assert_eq!(adjacent_neighbor_indices(2, 3), [Some(1), None]);
    }

    fn part_sized(path: &str, size: Option<u64>, attachment: bool) -> MessagePart {
        let folder = FolderId::new("inbox");
        let msg = MessageId::new(folder, "1");
        let mut part = mailiner_core::connector::mock_text_part(msg, path, "x");
        part.path = if path == "TEXT" {
            Vec::new()
        } else {
            path.split('.').map(str::to_string).collect()
        };
        part.original_size = size;
        part.is_attachment = attachment;
        part
    }

    #[test]
    fn prefetch_sections_skips_over_budget_and_attachments() {
        let parts = [
            part_sized("1", Some(100), false),
            part_sized("2", Some(MAX_PREFETCH_OCTETS), false),
            part_sized("3", Some(MAX_PREFETCH_OCTETS + 1), false),
            part_sized("4", None, false),
            part_sized("5", Some(10), true),
        ];
        assert_eq!(prefetch_sections(&parts), vec!["1", "2", "4"]);
        assert!(within_prefetch_budget(&parts[1]));
        assert!(!within_prefetch_budget(&parts[2]));
        assert!(within_prefetch_budget(&parts[3]));
    }

    fn dummy_loaded(uid: &str) -> Arc<LoadedMessage> {
        let folder = FolderId::new("inbox");
        let id = MessageId::new(folder.clone(), uid);
        Arc::new(LoadedMessage {
            envelope_id: id,
            folder_id: folder,
            parts: vec![],
        })
    }

    #[test]
    fn loaded_message_cache_lru_and_remove() {
        let mut cache = LoadedMessageCache::with_capacity(2);
        let a = MessageId::new(FolderId::new("inbox"), "a");
        let b = MessageId::new(FolderId::new("inbox"), "b");
        let c = MessageId::new(FolderId::new("inbox"), "c");
        cache.insert(a.clone(), dummy_loaded("a"));
        cache.insert(b.clone(), dummy_loaded("b"));
        assert!(cache.get(&a).is_some());
        cache.insert(c.clone(), dummy_loaded("c"));
        assert!(cache.contains(&a), "recent get should keep a");
        assert!(!cache.contains(&b), "oldest unused should evict");
        assert!(cache.contains(&c));
        cache.remove(&c);
        assert!(!cache.contains(&c));
        cache.remove_many(std::slice::from_ref(&a));
        assert!(!cache.contains(&a));
        cache.insert(a.clone(), dummy_loaded("a"));
        cache.clear();
        assert!(!cache.contains(&a));
    }
}
