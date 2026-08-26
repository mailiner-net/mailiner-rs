//! Global keyboard shortcuts: one catalog for dispatch and the help dialog.

/// Stable id used by the window handler to run an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutId {
    JumpToFolder,
    MoveToFolder,
    NextMessage,
    PrevMessage,
    ScrollMessageDown,
    ScrollMessageUp,
    PageMessageDown,
    PageMessageUp,
    MoveToTrash,
    DeletePermanently,
    ShowHelp,
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
            for key in shortcut.keys {
                let found = shortcut_for_key(key, shortcut.require_shift).expect(key);
                assert_eq!(found.id, shortcut.id);
                assert_eq!(found.description, shortcut.description);
            }
        }
    }

    #[test]
    fn unknown_key_is_none() {
        assert!(shortcut_for_key("x", false).is_none());
        assert!(shortcut_for_key("Escape", false).is_none());
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
    fn groups_cover_the_catalog() {
        let grouped: usize = ShortcutGroup::ALL
            .iter()
            .map(|g| shortcuts_in_group(*g).count())
            .sum();
        assert_eq!(grouped, GLOBAL_SHORTCUTS.len());
    }
}
