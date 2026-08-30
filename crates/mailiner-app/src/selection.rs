//! Message-list selection: one or many IDs, plus last-clicked focus.

use std::collections::HashSet;

use crate::message::MessageId;

/// Current list selection. Single-select is just `ids.len() == 1`.
///
/// The viewer shows [`Self::focus`] (last-clicked / keyboard focus). That id
/// is always a member of [`Self::ids`] when the set is non-empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageSelection {
    ids: HashSet<MessageId>,
    focus: Option<MessageId>,
    /// Index of the Shift+click / Shift+arrow range anchor.
    anchor_index: Option<usize>,
    /// Index of `focus` when it was focused (before unread-sort relocate).
    focus_at_index: Option<usize>,
}

impl MessageSelection {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when more than one row is selected (batch-action, not a read).
    pub fn is_multi(&self) -> bool {
        self.ids.len() > 1
    }

    pub fn contains(&self, id: &MessageId) -> bool {
        self.ids.contains(id)
    }

    pub fn focus(&self) -> Option<&MessageId> {
        self.focus.as_ref()
    }

    pub fn focus_at_index(&self) -> Option<usize> {
        self.focus_at_index
    }

    pub fn anchor_index(&self) -> Option<usize> {
        self.anchor_index
    }

    pub fn ids(&self) -> &HashSet<MessageId> {
        &self.ids
    }

    pub fn ids_vec(&self) -> Vec<MessageId> {
        self.ids.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Replace the set with a single message (plain click / arrow).
    pub fn replace(&mut self, id: MessageId, index: Option<usize>) {
        self.ids.clear();
        self.ids.insert(id.clone());
        self.focus = Some(id);
        self.anchor_index = index;
        self.focus_at_index = index;
    }

    /// Ctrl/Cmd+click: toggle membership. Focus follows the clicked row.
    /// The last remaining id cannot be toggled off (selection stays non-empty
    /// while a message is focused).
    pub fn toggle(&mut self, id: MessageId, index: Option<usize>) {
        if self.ids.contains(&id) {
            if self.ids.len() <= 1 {
                self.anchor_index = index;
                return;
            }
            self.ids.remove(&id);
            if self.focus.as_ref() == Some(&id) {
                self.focus = self.ids.iter().next().cloned();
                self.focus_at_index = None;
            }
        } else {
            self.ids.insert(id.clone());
            self.focus = Some(id);
            self.focus_at_index = index;
        }
        self.anchor_index = index;
    }

    /// Shift+click / Shift+arrow: select `ids` and focus `focus`.
    /// The range anchor is left unchanged so further Shift+moves grow/shrink
    /// from the original click.
    pub fn set_range(
        &mut self,
        ids: impl IntoIterator<Item = MessageId>,
        focus: MessageId,
        focus_index: Option<usize>,
    ) {
        self.ids = ids.into_iter().collect();
        if self.ids.is_empty() {
            self.ids.insert(focus.clone());
        } else {
            self.ids.insert(focus.clone());
        }
        self.focus = Some(focus);
        self.focus_at_index = focus_index;
        if self.anchor_index.is_none() {
            self.anchor_index = focus_index;
        }
    }

    /// Update focus without changing membership (after a range was applied).
    pub fn note_focus(&mut self, id: MessageId, index: Option<usize>) {
        self.ids.insert(id.clone());
        self.focus = Some(id);
        self.focus_at_index = index;
    }

    pub fn remove_ids(&mut self, gone: &HashSet<MessageId>) {
        self.ids.retain(|id| !gone.contains(id));
        if self.focus.as_ref().is_some_and(|id| gone.contains(id)) {
            self.focus = self.ids.iter().next().cloned();
            self.focus_at_index = None;
        }
    }
}

/// Auto-mark `\Seen` only when opening a single message.
///
/// `requested` is the caller's intent (plain click / arrow, or Unread-first
/// mailbox-open which already passes `false`). Multi-select is a batch-action
/// gesture and must not consume unread.
pub fn should_auto_mark_read(requested: bool, is_multi: bool) -> bool {
    requested && !is_multi
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> MessageId {
        MessageId::new(mailiner_core::FolderId::new("INBOX"), s)
    }

    #[test]
    fn replace_is_single_select() {
        let mut s = MessageSelection::default();
        s.replace(id("a"), Some(3));
        assert_eq!(s.len(), 1);
        assert!(s.contains(&id("a")));
        assert_eq!(s.focus(), Some(&id("a")));
        assert_eq!(s.anchor_index(), Some(3));
        assert_eq!(s.focus_at_index(), Some(3));
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut s = MessageSelection::default();
        s.replace(id("a"), Some(0));
        s.toggle(id("b"), Some(1));
        assert_eq!(s.len(), 2);
        assert_eq!(s.focus(), Some(&id("b")));
        s.toggle(id("b"), Some(1));
        assert_eq!(s.len(), 1);
        assert!(s.contains(&id("a")));
        assert_eq!(s.focus(), Some(&id("a")));
    }

    #[test]
    fn toggle_cannot_clear_last() {
        let mut s = MessageSelection::default();
        s.replace(id("a"), Some(0));
        s.toggle(id("a"), Some(0));
        assert_eq!(s.len(), 1);
        assert!(s.contains(&id("a")));
    }

    #[test]
    fn set_range_keeps_anchor() {
        let mut s = MessageSelection::default();
        s.replace(id("a"), Some(2));
        s.set_range([id("a"), id("b"), id("c")], id("c"), Some(4));
        assert_eq!(s.len(), 3);
        assert!(s.is_multi());
        assert_eq!(s.focus(), Some(&id("c")));
        assert_eq!(s.anchor_index(), Some(2));
        assert_eq!(s.focus_at_index(), Some(4));
    }

    #[test]
    fn single_select_is_not_multi() {
        let mut s = MessageSelection::default();
        assert!(!s.is_multi());
        s.replace(id("a"), Some(0));
        assert!(!s.is_multi());
    }

    #[test]
    fn auto_mark_only_when_opening_one_message() {
        assert!(should_auto_mark_read(true, false));
        assert!(!should_auto_mark_read(true, true));
        assert!(!should_auto_mark_read(false, false));
        assert!(!should_auto_mark_read(false, true));
    }
}
