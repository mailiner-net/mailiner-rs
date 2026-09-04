//! Global keyboard shortcuts: one catalog for dispatch and the help dialog.

/// Stable id used by the window handler to run an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutId {
    Compose,
    Reply,
    ReplyAll,
    Forward,
    Send,
    JumpToFolder,
    MoveToFolder,
    CopyToFolder,
    Archive,
    MoveToJunk,
    NextMessage,
    PrevMessage,
    NextUnread,
    PrevUnread,
    ExtendNextMessage,
    ExtendPrevMessage,
    ScrollMessageDown,
    ScrollMessageUp,
    PageMessageDown,
    PageMessageUp,
    MoveToTrash,
    DeletePermanently,
    SelectAll,
    ToggleStar,
    ToggleFlag,
    ShowHelp,
}

impl ShortcutId {
    /// These open a new draft and would replace the one already on screen.
    pub fn replaces_open_draft(self) -> bool {
        matches!(
            self,
            Self::Compose | Self::Reply | Self::ReplyAll | Self::Forward
        )
    }
}

/// Help-dialog grouping (order is display order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutGroup {
    Mail,
    Reading,
    Help,
}

impl ShortcutGroup {
    pub const ALL: &[ShortcutGroup] = &[
        ShortcutGroup::Mail,
        ShortcutGroup::Reading,
        ShortcutGroup::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ShortcutGroup::Mail => "Mail",
            ShortcutGroup::Reading => "Reading",
            ShortcutGroup::Help => "Help",
        }
    }
}

/// One global shortcut. `keys` are `KeyboardEvent.key` values.
///
/// Empty `keys` is help-dialog only (modifier chords handled locally).
/// `require_shift` bindings only fire with Shift held. Other bindings still
/// match when Shift is held (so J and Shift+J both jump), unless a
/// `require_shift` shortcut claims that key first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub id: ShortcutId,
    pub keys: &'static [&'static str],
    pub require_shift: bool,
    pub label: &'static str,
    pub description: &'static str,
    pub group: ShortcutGroup,
}

/// All global shortcuts. Add new ones here only.
pub const GLOBAL_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        id: ShortcutId::Compose,
        keys: &["c", "C"],
        require_shift: false,
        label: "C",
        description: "New message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::Reply,
        keys: &["r", "R"],
        require_shift: false,
        label: "R",
        description: "Reply",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::ReplyAll,
        keys: &["a", "A"],
        require_shift: false,
        label: "A",
        description: "Reply all",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::Forward,
        keys: &["f", "F"],
        require_shift: false,
        label: "F",
        description: "Forward",
        group: ShortcutGroup::Mail,
    },
    // Ctrl/Cmd+Enter is handled on the compose dialog; empty keys keep
    // shortcut_for_key from stealing Enter.
    Shortcut {
        id: ShortcutId::Send,
        keys: &[],
        require_shift: false,
        label: "Ctrl/⌘+Enter",
        description: "Send message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::JumpToFolder,
        keys: &["j", "J"],
        require_shift: false,
        label: "J",
        description: "Go to folder",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::MoveToFolder,
        keys: &["m", "M"],
        require_shift: false,
        label: "M",
        description: "Move message to folder",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::CopyToFolder,
        keys: &["c", "C"],
        require_shift: true,
        label: "Shift+C",
        description: "Copy message to folder",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::Archive,
        keys: &["e", "E"],
        require_shift: false,
        label: "E",
        description: "Archive message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::MoveToJunk,
        keys: &["!"],
        require_shift: false,
        label: "!",
        description: "Move message to junk",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::NextMessage,
        keys: &["ArrowDown"],
        require_shift: false,
        label: "↓",
        description: "Next message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::PrevMessage,
        keys: &["ArrowUp"],
        require_shift: false,
        label: "↑",
        description: "Previous message",
        group: ShortcutGroup::Mail,
    },
    // n/p (not KMail A/Z): A is already Reply all.
    Shortcut {
        id: ShortcutId::NextUnread,
        keys: &["n", "N"],
        require_shift: false,
        label: "N",
        description: "Next unread message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::PrevUnread,
        keys: &["p", "P"],
        require_shift: false,
        label: "P",
        description: "Previous unread message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::ExtendNextMessage,
        keys: &["ArrowDown"],
        require_shift: true,
        label: "Shift+↓",
        description: "Extend selection down",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::ExtendPrevMessage,
        keys: &["ArrowUp"],
        require_shift: true,
        label: "Shift+↑",
        description: "Extend selection up",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::MoveToTrash,
        keys: &["Delete"],
        require_shift: false,
        label: "Del",
        description: "Move message to trash",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::DeletePermanently,
        keys: &["Delete"],
        require_shift: true,
        label: "Shift+Del",
        description: "Delete message permanently",
        group: ShortcutGroup::Mail,
    },
    // Ctrl/Cmd+A is handled in the window listener; empty keys keep
    // shortcut_for_key from stealing A (Reply all).
    Shortcut {
        id: ShortcutId::SelectAll,
        keys: &[],
        require_shift: false,
        label: "Ctrl/⌘+A",
        description: "Select all messages",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::ToggleStar,
        keys: &["s", "S"],
        require_shift: false,
        label: "S",
        description: "Star or unstar message",
        group: ShortcutGroup::Mail,
    },
    // F is Forward; I is free and reads as "important".
    Shortcut {
        id: ShortcutId::ToggleFlag,
        keys: &["i", "I"],
        require_shift: false,
        label: "I",
        description: "Flag or unflag message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::ScrollMessageDown,
        keys: &["ArrowRight"],
        require_shift: false,
        label: "→",
        description: "Scroll message down",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::ScrollMessageUp,
        keys: &["ArrowLeft"],
        require_shift: false,
        label: "←",
        description: "Scroll message up",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::PageMessageDown,
        keys: &["PageDown"],
        require_shift: false,
        label: "Page Down",
        description: "Page message down",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::PageMessageUp,
        keys: &["PageUp"],
        require_shift: false,
        label: "Page Up",
        description: "Page message up",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::ShowHelp,
        keys: &["?"],
        require_shift: false,
        label: "?",
        description: "Show keyboard shortcuts",
        group: ShortcutGroup::Help,
    },
];

fn find_binding(key: &str, require_shift: bool) -> Option<&'static Shortcut> {
    GLOBAL_SHORTCUTS.iter().find(|shortcut| {
        shortcut.require_shift == require_shift && shortcut.keys.iter().any(|bound| *bound == key)
    })
}

/// Resolve a catalog entry for `KeyboardEvent.key` and the Shift modifier.
pub fn shortcut_for_key(key: &str, shift: bool) -> Option<&'static Shortcut> {
    if shift {
        if let Some(shortcut) = find_binding(key, true) {
            return Some(shortcut);
        }
    }
    find_binding(key, false)
}

pub fn shortcuts_in_group(group: ShortcutGroup) -> impl Iterator<Item = &'static Shortcut> {
    GLOBAL_SHORTCUTS
        .iter()
        .filter(move |shortcut| shortcut.group == group)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bound_key_resolves_to_its_shortcut() {
        for shortcut in GLOBAL_SHORTCUTS {
            if shortcut.keys.is_empty() {
                continue;
            }
            for key in shortcut.keys {
                let found = shortcut_for_key(key, shortcut.require_shift).expect(key);
                assert_eq!(found.id, shortcut.id);
                assert_eq!(found.description, shortcut.description);
            }
        }
    }

    #[test]
    fn compose_reply_and_forward_keys_resolve() {
        assert_eq!(
            shortcut_for_key("c", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        assert_eq!(
            shortcut_for_key("C", true).map(|s| s.id),
            Some(ShortcutId::CopyToFolder)
        );
        assert_eq!(
            shortcut_for_key("r", false).map(|s| s.id),
            Some(ShortcutId::Reply)
        );
        assert_eq!(
            shortcut_for_key("a", false).map(|s| s.id),
            Some(ShortcutId::ReplyAll)
        );
        assert_eq!(
            shortcut_for_key("f", false).map(|s| s.id),
            Some(ShortcutId::Forward)
        );
    }

    #[test]
    fn compose_family_shortcuts_replace_an_open_draft() {
        assert!(ShortcutId::Compose.replaces_open_draft());
        assert!(ShortcutId::Reply.replaces_open_draft());
        assert!(ShortcutId::ReplyAll.replaces_open_draft());
        assert!(ShortcutId::Forward.replaces_open_draft());
        assert!(!ShortcutId::NextMessage.replaces_open_draft());
        assert!(!ShortcutId::JumpToFolder.replaces_open_draft());
        assert!(!ShortcutId::Send.replaces_open_draft());
    }

    #[test]
    fn send_is_help_only_and_does_not_steal_enter() {
        let send = GLOBAL_SHORTCUTS
            .iter()
            .find(|s| s.id == ShortcutId::Send)
            .expect("Send catalog entry");
        assert!(send.keys.is_empty());
        assert_eq!(send.label, "Ctrl/⌘+Enter");
        assert_eq!(send.description, "Send message");
        assert!(shortcut_for_key("Enter", false).is_none());
        assert!(shortcut_for_key("Enter", true).is_none());
    }

    #[test]
    fn select_all_is_help_only_and_does_not_steal_a() {
        let select_all = GLOBAL_SHORTCUTS
            .iter()
            .find(|s| s.id == ShortcutId::SelectAll)
            .expect("SelectAll catalog entry");
        assert!(select_all.keys.is_empty());
        assert_eq!(select_all.label, "Ctrl/⌘+A");
        assert_eq!(select_all.description, "Select all messages");
        assert_eq!(
            shortcut_for_key("a", false).map(|s| s.id),
            Some(ShortcutId::ReplyAll)
        );
    }

    #[test]
    fn unknown_key_is_none() {
        assert!(shortcut_for_key("x", false).is_none());
        assert!(shortcut_for_key("Escape", false).is_none());
    }

    #[test]
    fn archive_key_resolves() {
        assert_eq!(
            shortcut_for_key("e", false).map(|s| s.id),
            Some(ShortcutId::Archive)
        );
        assert_eq!(
            shortcut_for_key("E", true).map(|s| s.id),
            Some(ShortcutId::Archive)
        );
    }

    #[test]
    fn star_and_flag_keys_do_not_collide() {
        assert_eq!(
            shortcut_for_key("s", false).map(|s| s.id),
            Some(ShortcutId::ToggleStar)
        );
        assert_eq!(
            shortcut_for_key("S", true).map(|s| s.id),
            Some(ShortcutId::ToggleStar)
        );
        assert_eq!(
            shortcut_for_key("i", false).map(|s| s.id),
            Some(ShortcutId::ToggleFlag)
        );
        assert_eq!(
            shortcut_for_key("f", false).map(|s| s.id),
            Some(ShortcutId::Forward)
        );
    }

    #[test]
    fn next_and_prev_unread_keys_resolve() {
        assert_eq!(
            shortcut_for_key("n", false).map(|s| s.id),
            Some(ShortcutId::NextUnread)
        );
        assert_eq!(
            shortcut_for_key("N", true).map(|s| s.id),
            Some(ShortcutId::NextUnread)
        );
        assert_eq!(
            shortcut_for_key("p", false).map(|s| s.id),
            Some(ShortcutId::PrevUnread)
        );
        assert_eq!(
            shortcut_for_key("P", true).map(|s| s.id),
            Some(ShortcutId::PrevUnread)
        );
        assert_eq!(
            shortcut_for_key("a", false).map(|s| s.id),
            Some(ShortcutId::ReplyAll)
        );
    }

    #[test]
    fn junk_key_resolves() {
        assert_eq!(
            shortcut_for_key("!", false).map(|s| s.id),
            Some(ShortcutId::MoveToJunk)
        );
        assert_eq!(
            shortcut_for_key("!", true).map(|s| s.id),
            Some(ShortcutId::MoveToJunk)
        );
    }

    #[test]
    fn delete_and_shift_delete_are_distinct() {
        assert_eq!(
            shortcut_for_key("Delete", false).map(|s| s.id),
            Some(ShortcutId::MoveToTrash)
        );
        assert_eq!(
            shortcut_for_key("Delete", true).map(|s| s.id),
            Some(ShortcutId::DeletePermanently)
        );
    }

    #[test]
    fn shift_does_not_block_letter_shortcuts() {
        assert_eq!(
            shortcut_for_key("J", true).map(|s| s.id),
            Some(ShortcutId::JumpToFolder)
        );
    }

    #[test]
    fn shift_c_copies_plain_c_composes() {
        assert_eq!(
            shortcut_for_key("c", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        assert_eq!(
            shortcut_for_key("C", true).map(|s| s.id),
            Some(ShortcutId::CopyToFolder)
        );
        assert_eq!(
            shortcut_for_key("c", true).map(|s| s.id),
            Some(ShortcutId::CopyToFolder)
        );
    }

    #[test]
    fn groups_cover_the_catalog() {
        let grouped: usize = ShortcutGroup::ALL
            .iter()
            .map(|g| shortcuts_in_group(*g).count())
            .sum();
        assert_eq!(grouped, GLOBAL_SHORTCUTS.len());
    }
}
