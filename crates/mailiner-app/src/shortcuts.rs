//! Global keyboard shortcuts: one catalog for dispatch and the help dialog.
//!
//! [`GLOBAL_SHORTCUTS`] is the default map. User remaps live in
//! [`crate::ui_prefs`] and overlay that catalog at resolve time.

use crate::ui_prefs::{ShortcutBinding, ShortcutMapBlob};

/// Stable id used by the window handler to run an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

    /// Letter/action keys shown in Settings. Modifier chords and standard
    /// navigation/delete keys stay fixed.
    pub fn remappable(self) -> bool {
        matches!(
            self,
            Self::Compose
                | Self::Reply
                | Self::ReplyAll
                | Self::Forward
                | Self::JumpToFolder
                | Self::MoveToFolder
                | Self::CopyToFolder
                | Self::Archive
                | Self::MoveToJunk
                | Self::NextMessage
                | Self::PrevMessage
                | Self::NextUnread
                | Self::PrevUnread
                | Self::ToggleStar
                | Self::ToggleFlag
                | Self::ShowHelp
        )
    }

    /// Stable `localStorage` key for a remap entry.
    pub fn as_key(self) -> &'static str {
        match self {
            Self::Compose => "compose",
            Self::Reply => "reply",
            Self::ReplyAll => "reply_all",
            Self::Forward => "forward",
            Self::Send => "send",
            Self::JumpToFolder => "jump_to_folder",
            Self::MoveToFolder => "move_to_folder",
            Self::CopyToFolder => "copy_to_folder",
            Self::Archive => "archive",
            Self::MoveToJunk => "move_to_junk",
            Self::NextMessage => "next_message",
            Self::PrevMessage => "prev_message",
            Self::NextUnread => "next_unread",
            Self::PrevUnread => "prev_unread",
            Self::ExtendNextMessage => "extend_next_message",
            Self::ExtendPrevMessage => "extend_prev_message",
            Self::ScrollMessageDown => "scroll_message_down",
            Self::ScrollMessageUp => "scroll_message_up",
            Self::PageMessageDown => "page_message_down",
            Self::PageMessageUp => "page_message_up",
            Self::MoveToTrash => "move_to_trash",
            Self::DeletePermanently => "delete_permanently",
            Self::SelectAll => "select_all",
            Self::ToggleStar => "toggle_star",
            Self::ToggleFlag => "toggle_flag",
            Self::ShowHelp => "show_help",
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "compose" => Some(Self::Compose),
            "reply" => Some(Self::Reply),
            "reply_all" => Some(Self::ReplyAll),
            "forward" => Some(Self::Forward),
            "send" => Some(Self::Send),
            "jump_to_folder" => Some(Self::JumpToFolder),
            "move_to_folder" => Some(Self::MoveToFolder),
            "copy_to_folder" => Some(Self::CopyToFolder),
            "archive" => Some(Self::Archive),
            "move_to_junk" => Some(Self::MoveToJunk),
            "next_message" => Some(Self::NextMessage),
            "prev_message" => Some(Self::PrevMessage),
            "next_unread" => Some(Self::NextUnread),
            "prev_unread" => Some(Self::PrevUnread),
            "extend_next_message" => Some(Self::ExtendNextMessage),
            "extend_prev_message" => Some(Self::ExtendPrevMessage),
            "scroll_message_down" => Some(Self::ScrollMessageDown),
            "scroll_message_up" => Some(Self::ScrollMessageUp),
            "page_message_down" => Some(Self::PageMessageDown),
            "page_message_up" => Some(Self::PageMessageUp),
            "move_to_trash" => Some(Self::MoveToTrash),
            "delete_permanently" => Some(Self::DeletePermanently),
            "select_all" => Some(Self::SelectAll),
            "toggle_star" => Some(Self::ToggleStar),
            "toggle_flag" => Some(Self::ToggleFlag),
            "show_help" => Some(Self::ShowHelp),
            _ => None,
        }
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

/// Catalog entry after applying persisted remaps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveShortcut {
    pub id: ShortcutId,
    pub keys: Vec<String>,
    pub require_shift: bool,
    pub label: String,
    pub description: &'static str,
    pub group: ShortcutGroup,
    pub remapped: bool,
}

/// Why a remap was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShortcutRemapError {
    NotRemappable,
    InvalidKey,
    Conflict { other_description: &'static str },
}

impl ShortcutRemapError {
    pub fn message(&self) -> String {
        match self {
            Self::NotRemappable => "This shortcut cannot be changed.".to_string(),
            Self::InvalidKey => "That key cannot be used.".to_string(),
            Self::Conflict { other_description } => {
                format!("Already used by {other_description}.")
            }
        }
    }
}

fn catalog_entry(id: ShortcutId) -> Option<&'static Shortcut> {
    GLOBAL_SHORTCUTS.iter().find(|shortcut| shortcut.id == id)
}

fn catalog_description(id: ShortcutId) -> &'static str {
    catalog_entry(id)
        .map(|shortcut| shortcut.description)
        .unwrap_or("another shortcut")
}

/// Normalize a captured `KeyboardEvent.key` into a stored binding.
pub fn normalize_binding(key: &str, shift: bool) -> Option<ShortcutBinding> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    match key {
        "Shift" | "Control" | "Ctrl" | "Alt" | "Meta" | "AltGraph" | "OS" | "Hyper" | "Super"
        | "Tab" | "Escape" | "Unidentified" | "Dead" => return None,
        _ => {}
    }
    if key.chars().count() == 1 {
        let c = key.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(ShortcutBinding {
                key: c.to_ascii_lowercase().to_string(),
                shift,
            });
        }
        // Punctuation already encodes Shift (`?`, `!`).
        return Some(ShortcutBinding {
            key: key.to_string(),
            shift: false,
        });
    }
    Some(ShortcutBinding {
        key: key.to_string(),
        shift,
    })
}

fn expand_binding_keys(key: &str) -> Vec<String> {
    if key.chars().count() == 1 {
        let c = key.chars().next().expect("len 1");
        if c.is_ascii_alphabetic() {
            return vec![
                c.to_ascii_lowercase().to_string(),
                c.to_ascii_uppercase().to_string(),
            ];
        }
    }
    vec![key.to_string()]
}

/// Pretty label for a stored binding (`C`, `Shift+C`, `↓`).
pub fn binding_label(key: &str, require_shift: bool) -> String {
    let pretty = match key {
        "ArrowDown" => "↓".to_string(),
        "ArrowUp" => "↑".to_string(),
        "ArrowRight" => "→".to_string(),
        "ArrowLeft" => "←".to_string(),
        "Delete" => "Del".to_string(),
        "Backspace" => "Backspace".to_string(),
        "PageDown" => "Page Down".to_string(),
        "PageUp" => "Page Up".to_string(),
        " " | "Spacebar" | "Space" => "Space".to_string(),
        "Escape" => "Esc".to_string(),
        "Enter" => "Enter".to_string(),
        "Home" => "Home".to_string(),
        "End" => "End".to_string(),
        "Insert" => "Ins".to_string(),
        other if other.chars().count() == 1 => other.to_ascii_uppercase(),
        other => other.to_string(),
    };
    if require_shift {
        format!("Shift+{pretty}")
    } else {
        pretty
    }
}

impl Shortcut {
    fn default_binding(&self) -> Option<ShortcutBinding> {
        let primary = self
            .keys
            .iter()
            .find(|key| key.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
            .or_else(|| self.keys.first())?;
        Some(ShortcutBinding {
            key: (*primary).to_string(),
            shift: self.require_shift,
        })
    }
}

fn binding_for(blob: &ShortcutMapBlob, id: ShortcutId) -> Option<&ShortcutBinding> {
    if !id.remappable() {
        return None;
    }
    blob.remaps.get(id.as_key())
}

fn resolve_one(shortcut: &Shortcut, blob: &ShortcutMapBlob) -> EffectiveShortcut {
    if let Some(raw) = binding_for(blob, shortcut.id)
        && let Some(binding) = normalize_binding(&raw.key, raw.shift)
        && shortcut.default_binding().as_ref() != Some(&binding)
    {
        return EffectiveShortcut {
            id: shortcut.id,
            keys: expand_binding_keys(&binding.key),
            require_shift: binding.shift,
            label: binding_label(&binding.key, binding.shift),
            description: shortcut.description,
            group: shortcut.group,
            remapped: true,
        };
    }
    EffectiveShortcut {
        id: shortcut.id,
        keys: shortcut.keys.iter().map(|key| (*key).to_string()).collect(),
        require_shift: shortcut.require_shift,
        label: shortcut.label.to_string(),
        description: shortcut.description,
        group: shortcut.group,
        remapped: false,
    }
}

/// Catalog with remaps applied, in display order.
pub fn effective_shortcuts_in(blob: &ShortcutMapBlob) -> Vec<EffectiveShortcut> {
    GLOBAL_SHORTCUTS
        .iter()
        .map(|shortcut| resolve_one(shortcut, blob))
        .collect()
}

pub fn effective_shortcuts_in_group(group: ShortcutGroup) -> Vec<EffectiveShortcut> {
    effective_shortcuts_in(&crate::ui_prefs::load_shortcut_map())
        .into_iter()
        .filter(|shortcut| shortcut.group == group)
        .collect()
}

fn find_binding_in<'a>(
    list: &'a [EffectiveShortcut],
    key: &str,
    require_shift: bool,
) -> Option<&'a EffectiveShortcut> {
    list.iter().find(|shortcut| {
        shortcut.require_shift == require_shift && shortcut.keys.iter().any(|bound| bound == key)
    })
}

/// Resolve a catalog entry for `KeyboardEvent.key` and the Shift modifier.
pub fn shortcut_for_key_in(
    blob: &ShortcutMapBlob,
    key: &str,
    shift: bool,
) -> Option<EffectiveShortcut> {
    let list = effective_shortcuts_in(blob);
    if shift && let Some(shortcut) = find_binding_in(&list, key, true) {
        return Some(shortcut.clone());
    }
    find_binding_in(&list, key, false).cloned()
}

/// Resolve using the persisted remap map (defaults when unset).
pub fn shortcut_for_key(key: &str, shift: bool) -> Option<EffectiveShortcut> {
    shortcut_for_key_in(&crate::ui_prefs::load_shortcut_map(), key, shift)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn shortcuts_in_group(group: ShortcutGroup) -> impl Iterator<Item = &'static Shortcut> {
    GLOBAL_SHORTCUTS
        .iter()
        .filter(move |shortcut| shortcut.group == group)
}

fn conflict_for(blob: &ShortcutMapBlob, id: ShortcutId) -> Option<ShortcutId> {
    let mine = resolve_one(catalog_entry(id)?, blob);
    if mine.keys.is_empty() {
        return None;
    }
    effective_shortcuts_in(blob).into_iter().find_map(|other| {
        if other.id == id || other.require_shift != mine.require_shift {
            return None;
        }
        other
            .keys
            .iter()
            .any(|key| mine.keys.iter().any(|bound| bound == key))
            .then_some(other.id)
    })
}

/// Persist a remap. Same as the catalog default drops the stored entry.
pub fn remap_shortcut(id: ShortcutId, key: &str, shift: bool) -> Result<(), ShortcutRemapError> {
    if !id.remappable() {
        return Err(ShortcutRemapError::NotRemappable);
    }
    let binding = normalize_binding(key, shift).ok_or(ShortcutRemapError::InvalidKey)?;
    let mut blob = crate::ui_prefs::load_shortcut_map();
    let catalog = catalog_entry(id).ok_or(ShortcutRemapError::NotRemappable)?;
    if catalog.default_binding().as_ref() == Some(&binding) {
        blob.remaps.remove(id.as_key());
    } else {
        blob.remaps.insert(id.as_key().to_string(), binding);
    }
    if let Some(other) = conflict_for(&blob, id) {
        return Err(ShortcutRemapError::Conflict {
            other_description: catalog_description(other),
        });
    }
    crate::ui_prefs::save_shortcut_map(&blob);
    Ok(())
}

/// Drop one remap. Fails when the catalog default now collides.
pub fn reset_shortcut(id: ShortcutId) -> Result<(), ShortcutRemapError> {
    if !id.remappable() {
        return Err(ShortcutRemapError::NotRemappable);
    }
    let mut blob = crate::ui_prefs::load_shortcut_map();
    blob.remaps.remove(id.as_key());
    if let Some(other) = conflict_for(&blob, id) {
        return Err(ShortcutRemapError::Conflict {
            other_description: catalog_description(other),
        });
    }
    crate::ui_prefs::save_shortcut_map(&blob);
    Ok(())
}

pub fn reset_all_shortcuts() {
    crate::ui_prefs::save_shortcut_map(&ShortcutMapBlob::empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_map() -> ShortcutMapBlob {
        ShortcutMapBlob::empty()
    }

    fn lookup(key: &str, shift: bool) -> Option<ShortcutId> {
        shortcut_for_key_in(&empty_map(), key, shift).map(|s| s.id)
    }

    fn remap_blob(id: ShortcutId, key: &str, shift: bool) -> ShortcutMapBlob {
        let binding = normalize_binding(key, shift).expect(key);
        let mut blob = ShortcutMapBlob::empty();
        blob.remaps.insert(id.as_key().to_string(), binding);
        blob
    }

    #[test]
    fn every_bound_key_resolves_to_its_shortcut() {
        for shortcut in GLOBAL_SHORTCUTS {
            if shortcut.keys.is_empty() {
                continue;
            }
            for key in shortcut.keys {
                let found = shortcut_for_key_in(&empty_map(), key, shortcut.require_shift)
                    .unwrap_or_else(|| panic!("{key}"));
                assert_eq!(found.id, shortcut.id);
                assert_eq!(found.description, shortcut.description);
            }
        }
    }

    #[test]
    fn compose_reply_and_forward_keys_resolve() {
        assert_eq!(lookup("c", false), Some(ShortcutId::Compose));
        assert_eq!(lookup("C", true), Some(ShortcutId::CopyToFolder));
        assert_eq!(lookup("r", false), Some(ShortcutId::Reply));
        assert_eq!(lookup("a", false), Some(ShortcutId::ReplyAll));
        assert_eq!(lookup("f", false), Some(ShortcutId::Forward));
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
        assert!(lookup("Enter", false).is_none());
        assert!(lookup("Enter", true).is_none());
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
        assert_eq!(lookup("a", false), Some(ShortcutId::ReplyAll));
    }

    #[test]
    fn unknown_key_is_none() {
        assert!(lookup("x", false).is_none());
        assert!(lookup("Escape", false).is_none());
    }

    #[test]
    fn archive_key_resolves() {
        assert_eq!(lookup("e", false), Some(ShortcutId::Archive));
        assert_eq!(lookup("E", true), Some(ShortcutId::Archive));
    }

    #[test]
    fn star_and_flag_keys_do_not_collide() {
        assert_eq!(lookup("s", false), Some(ShortcutId::ToggleStar));
        assert_eq!(lookup("S", true), Some(ShortcutId::ToggleStar));
        assert_eq!(lookup("i", false), Some(ShortcutId::ToggleFlag));
        assert_eq!(lookup("f", false), Some(ShortcutId::Forward));
    }

    #[test]
    fn next_and_prev_unread_keys_resolve() {
        assert_eq!(lookup("n", false), Some(ShortcutId::NextUnread));
        assert_eq!(lookup("N", true), Some(ShortcutId::NextUnread));
        assert_eq!(lookup("p", false), Some(ShortcutId::PrevUnread));
        assert_eq!(lookup("P", true), Some(ShortcutId::PrevUnread));
        assert_eq!(lookup("a", false), Some(ShortcutId::ReplyAll));
    }

    #[test]
    fn junk_key_resolves() {
        assert_eq!(lookup("!", false), Some(ShortcutId::MoveToJunk));
        assert_eq!(lookup("!", true), Some(ShortcutId::MoveToJunk));
    }

    #[test]
    fn delete_and_shift_delete_are_distinct() {
        assert_eq!(lookup("Delete", false), Some(ShortcutId::MoveToTrash));
        assert_eq!(lookup("Delete", true), Some(ShortcutId::DeletePermanently));
    }

    #[test]
    fn shift_does_not_block_letter_shortcuts() {
        assert_eq!(lookup("J", true), Some(ShortcutId::JumpToFolder));
    }

    #[test]
    fn shift_c_copies_plain_c_composes() {
        assert_eq!(lookup("c", false), Some(ShortcutId::Compose));
        assert_eq!(lookup("C", true), Some(ShortcutId::CopyToFolder));
        assert_eq!(lookup("c", true), Some(ShortcutId::CopyToFolder));
    }

    #[test]
    fn groups_cover_the_catalog() {
        let grouped: usize = ShortcutGroup::ALL
            .iter()
            .map(|g| shortcuts_in_group(*g).count())
            .sum();
        assert_eq!(grouped, GLOBAL_SHORTCUTS.len());
    }

    #[test]
    fn shortcut_id_keys_roundtrip() {
        for shortcut in GLOBAL_SHORTCUTS {
            let key = shortcut.id.as_key();
            assert_eq!(ShortcutId::from_key(key), Some(shortcut.id));
        }
        assert_eq!(ShortcutId::from_key("nope"), None);
        assert!(ShortcutId::Compose.remappable());
        assert!(ShortcutId::ShowHelp.remappable());
        assert!(!ShortcutId::Send.remappable());
        assert!(!ShortcutId::SelectAll.remappable());
        assert!(!ShortcutId::MoveToTrash.remappable());
        assert!(!ShortcutId::DeletePermanently.remappable());
    }

    #[test]
    fn remap_moves_compose_off_c() {
        let blob = remap_blob(ShortcutId::Compose, "x", false);
        assert_eq!(
            shortcut_for_key_in(&blob, "x", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        assert_eq!(
            shortcut_for_key_in(&blob, "X", true).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        assert!(shortcut_for_key_in(&blob, "c", false).is_none());
        assert_eq!(
            shortcut_for_key_in(&blob, "C", true).map(|s| s.id),
            Some(ShortcutId::CopyToFolder)
        );
        let compose = effective_shortcuts_in(&blob)
            .into_iter()
            .find(|s| s.id == ShortcutId::Compose)
            .expect("compose");
        assert!(compose.remapped);
        assert_eq!(compose.label, "X");
    }

    #[test]
    fn remap_shift_binding_keeps_unshifted_default() {
        let blob = remap_blob(ShortcutId::CopyToFolder, "x", true);
        assert_eq!(
            shortcut_for_key_in(&blob, "x", true).map(|s| s.id),
            Some(ShortcutId::CopyToFolder)
        );
        assert_eq!(
            shortcut_for_key_in(&blob, "c", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        assert_eq!(
            shortcut_for_key_in(&blob, "C", true).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
    }

    #[test]
    fn unknown_or_fixed_remap_is_ignored() {
        let mut blob = ShortcutMapBlob::empty();
        blob.remaps.insert(
            "nope".into(),
            ShortcutBinding {
                key: "x".into(),
                shift: false,
            },
        );
        blob.remaps.insert(
            "send".into(),
            ShortcutBinding {
                key: "x".into(),
                shift: false,
            },
        );
        assert!(shortcut_for_key_in(&blob, "x", false).is_none());
        assert!(lookup("x", false).is_none());
    }

    #[test]
    fn remap_rejects_collision_and_fixed_ids() {
        reset_all_shortcuts();
        let err = remap_shortcut(ShortcutId::Compose, "r", false).unwrap_err();
        assert!(matches!(err, ShortcutRemapError::Conflict { .. }));
        assert!(err.message().contains("Reply"));
        assert_eq!(
            remap_shortcut(ShortcutId::Send, "x", false),
            Err(ShortcutRemapError::NotRemappable)
        );
        assert_eq!(
            remap_shortcut(ShortcutId::Compose, "Escape", false),
            Err(ShortcutRemapError::InvalidKey)
        );
        remap_shortcut(ShortcutId::Compose, "x", false).expect("free key");
        assert_eq!(
            shortcut_for_key("x", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        assert!(shortcut_for_key("c", false).is_none());
        reset_shortcut(ShortcutId::Compose).expect("reset");
        assert_eq!(
            shortcut_for_key("c", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        reset_all_shortcuts();
    }

    #[test]
    fn reset_default_conflicts_when_another_remap_took_it() {
        reset_all_shortcuts();
        remap_shortcut(ShortcutId::Compose, "x", false).expect("compose");
        remap_shortcut(ShortcutId::Reply, "c", false).expect("reply took C");
        let err = reset_shortcut(ShortcutId::Compose).unwrap_err();
        assert!(matches!(err, ShortcutRemapError::Conflict { .. }));
        assert_eq!(
            shortcut_for_key("x", false).map(|s| s.id),
            Some(ShortcutId::Compose)
        );
        reset_all_shortcuts();
    }

    #[test]
    fn same_as_default_clears_stored_remap() {
        reset_all_shortcuts();
        remap_shortcut(ShortcutId::Compose, "x", false).expect("remap");
        remap_shortcut(ShortcutId::Compose, "c", false).expect("back to default");
        assert!(crate::ui_prefs::load_shortcut_map().remaps.is_empty());
        reset_all_shortcuts();
    }

    #[test]
    fn binding_label_matches_catalog_style() {
        assert_eq!(binding_label("c", false), "C");
        assert_eq!(binding_label("c", true), "Shift+C");
        assert_eq!(binding_label("ArrowDown", false), "↓");
        assert_eq!(binding_label("Delete", true), "Shift+Del");
        assert_eq!(binding_label("?", false), "?");
    }
}
