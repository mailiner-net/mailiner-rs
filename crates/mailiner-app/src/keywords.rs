//! Display helpers for IMAP keywords / labels.

use dioxus::prelude::*;
use mailiner_core::ImapKeyword;

/// Max chips on a list row; the rest collapse to `+N`.
const LIST_CHIP_CAP: usize = 3;

/// Protocol keywords that are not user-facing labels.
const HIDDEN_KEYWORDS: &[&str] = &["$mdnsent", "$forwarded", "$junk", "$notjunk", "$phishing"];

/// Chip shown on the message list and viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordChip {
    pub atom: String,
    pub label: String,
    pub tone: &'static str,
}

impl KeywordChip {
    pub fn css_class(&self) -> String {
        format!("message-keyword-chip {}", self.tone)
    }
}

pub fn has_visible_keywords(atoms: &[String]) -> bool {
    atoms
        .iter()
        .any(|atom| ImapKeyword::from_atom(atom).is_some() || !is_hidden_keyword(atom))
}

/// Visible labels: built-in keywords first (palette order), then other atoms.
pub fn visible_keyword_chips(atoms: &[String]) -> Vec<KeywordChip> {
    let mut chips = Vec::new();
    for keyword in ImapKeyword::ALL {
        if atoms
            .iter()
            .any(|atom| ImapKeyword::from_atom(atom) == Some(keyword))
        {
            chips.push(KeywordChip {
                atom: keyword.atom().to_string(),
                label: keyword.label().to_string(),
                tone: keyword_tone(Some(keyword)),
            });
        }
    }
    let mut unknown: Vec<&str> = atoms
        .iter()
        .map(String::as_str)
        .filter(|atom| ImapKeyword::from_atom(atom).is_none() && !is_hidden_keyword(atom))
        .collect();
    unknown.sort_unstable();
    unknown.dedup();
    for atom in unknown {
        chips.push(KeywordChip {
            atom: atom.to_string(),
            label: display_atom(atom),
            tone: keyword_tone(None),
        });
    }
    chips
}

pub fn keyword_tone(keyword: Option<ImapKeyword>) -> &'static str {
    match keyword {
        Some(ImapKeyword::Important) => "is-important",
        Some(ImapKeyword::Work) => "is-work",
        Some(ImapKeyword::Personal) => "is-personal",
        Some(ImapKeyword::Todo) => "is-todo",
        Some(ImapKeyword::Later) => "is-later",
        None => "is-other",
    }
}

fn is_hidden_keyword(atom: &str) -> bool {
    HIDDEN_KEYWORDS
        .iter()
        .any(|hidden| hidden.eq_ignore_ascii_case(atom))
}

fn display_atom(atom: &str) -> String {
    atom.strip_prefix('$').unwrap_or(atom).to_string()
}

#[component]
pub fn MessageKeywordChips(atoms: Vec<String>, compact: bool) -> Element {
    let chips = visible_keyword_chips(&atoms);
    if chips.is_empty() {
        return rsx! {};
    }
    let extra = if compact {
        chips.len().saturating_sub(LIST_CHIP_CAP)
    } else {
        0
    };
    let shown = if compact && extra > 0 {
        &chips[..LIST_CHIP_CAP]
    } else {
        chips.as_slice()
    };
    rsx! {
        span {
            class: "message-keyword-chips",
            class: if compact { "is-compact" },
            for chip in shown.iter() {
                span {
                    class: chip.css_class(),
                    title: "{chip.atom}",
                    "{chip.label}"
                }
            }
            if extra > 0 {
                span {
                    class: "message-keyword-chip is-other",
                    title: "More labels",
                    "+{extra}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_chips_order_known_then_unknown() {
        let chips = visible_keyword_chips(&[
            "ProjectX".into(),
            "$Todo".into(),
            "$MDNSent".into(),
            "$important".into(),
            "alpha".into(),
        ]);
        assert_eq!(
            chips
                .iter()
                .map(|c| (c.label.as_str(), c.tone))
                .collect::<Vec<_>>(),
            vec![
                ("Important", "is-important"),
                ("To do", "is-todo"),
                ("ProjectX", "is-other"),
                ("alpha", "is-other"),
            ]
        );
    }

    #[test]
    fn hidden_protocol_keywords_are_omitted() {
        let hidden = vec![
            "$Forwarded".into(),
            "$Junk".into(),
            "$NotJunk".into(),
            "$Phishing".into(),
        ];
        assert!(visible_keyword_chips(&hidden).is_empty());
        assert!(!has_visible_keywords(&hidden));
        assert!(has_visible_keywords(&["$Todo".into()]));
    }
}
