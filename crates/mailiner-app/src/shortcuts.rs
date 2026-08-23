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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub id: ShortcutId,
    pub keys: &'static [&'static str],
    pub label: &'static str,
    pub description: &'static str,
    pub group: ShortcutGroup,
}

/// All global shortcuts. Add new ones here only.
pub const GLOBAL_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        id: ShortcutId::JumpToFolder,
        keys: &["j", "J"],
        label: "J",
        description: "Go to folder",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::MoveToFolder,
        keys: &["m", "M"],
        label: "M",
        description: "Move message to folder",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::NextMessage,
        keys: &["ArrowDown"],
        label: "↓",
        description: "Next message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::PrevMessage,
        keys: &["ArrowUp"],
        label: "↑",
        description: "Previous message",
        group: ShortcutGroup::Mail,
    },
    Shortcut {
        id: ShortcutId::ScrollMessageDown,
        keys: &["ArrowRight"],
        label: "→",
        description: "Scroll message down",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::ScrollMessageUp,
        keys: &["ArrowLeft"],
        label: "←",
        description: "Scroll message up",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::PageMessageDown,
        keys: &["PageDown"],
        label: "Page Down",
        description: "Page message down",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::PageMessageUp,
        keys: &["PageUp"],
        label: "Page Up",
        description: "Page message up",
        group: ShortcutGroup::Reading,
    },
    Shortcut {
        id: ShortcutId::ShowHelp,
        keys: &["?"],
        label: "?",
        description: "Show keyboard shortcuts",
        group: ShortcutGroup::Help,
    },
];

pub fn shortcut_for_key(key: &str) -> Option<&'static Shortcut> {
    GLOBAL_SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.keys.iter().any(|bound| *bound == key))
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
                let found = shortcut_for_key(key).expect(key);
                assert_eq!(found.id, shortcut.id);
                assert_eq!(found.description, shortcut.description);
            }
        }
    }

    #[test]
    fn unknown_key_is_none() {
        assert!(shortcut_for_key("x").is_none());
        assert!(shortcut_for_key("Escape").is_none());
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
