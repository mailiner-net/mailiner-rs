//! Address flattening and self-exclusion for Reply-All.

use mailiner_core::{EmailAddr, EmailAddress};

use crate::model::draft::ComposerAddress;

/// Flatten List or Group into a linear chip list.
///
/// - Group: expand all members; group display name is discarded in v1.
/// - Skip entries with missing/empty email.
/// - Trim; do not invent emails from display names alone.
pub fn flatten_addresses(addr: &EmailAddress) -> Vec<ComposerAddress> {
    match addr {
        EmailAddress::List(list) => list.iter().filter_map(try_composer_address).collect(),
        EmailAddress::Group(groups) => groups
            .iter()
            .flat_map(|g| g.members.iter().filter_map(try_composer_address))
            .collect(),
    }
}

/// Try convert one [`EmailAddr`]; returns `None` if email missing/empty.
pub fn try_composer_address(a: &EmailAddr) -> Option<ComposerAddress> {
    let email = a.email.as_deref()?.trim();
    if email.is_empty() {
        return None;
    }
    Some(ComposerAddress {
        name: a
            .name
            .as_ref()
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
        email: email.to_string(), // already trimmed
    })
}

impl From<ComposerAddress> for EmailAddr {
    fn from(c: ComposerAddress) -> Self {
        EmailAddr {
            name: c.name,
            email: Some(c.email),
        }
    }
}

/// Normalize for comparison: trim, lowercase (ASCII).
///
/// v1 does **not** strip plus-tags or apply IDNA.
pub fn emails_equal(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Drop addresses matching `self_email` (using [`emails_equal`]).
pub fn exclude_self(list: Vec<ComposerAddress>, self_email: &str) -> Vec<ComposerAddress> {
    list.into_iter()
        .filter(|a| !emails_equal(&a.email, self_email))
        .collect()
}

/// Deduplicate by email (case-insensitive), keeping first occurrence.
pub fn dedupe_addresses(list: Vec<ComposerAddress>) -> Vec<ComposerAddress> {
    let mut out = Vec::new();
    for a in list {
        if out
            .iter()
            .any(|b: &ComposerAddress| emails_equal(&b.email, &a.email))
        {
            continue;
        }
        out.push(a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::Group;

    #[test]
    fn flatten_list_skips_empty_email() {
        let addr = EmailAddress::List(vec![
            EmailAddr {
                name: Some("A".into()),
                email: Some("a@example.com".into()),
            },
            EmailAddr {
                name: Some("NoMail".into()),
                email: None,
            },
            EmailAddr {
                name: None,
                email: Some("  ".into()),
            },
        ]);
        let v = flatten_addresses(&addr);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].email, "a@example.com");
    }

    #[test]
    fn flatten_group_expands_members() {
        let addr = EmailAddress::Group(vec![Group {
            name: Some("Team".into()),
            members: vec![
                EmailAddr {
                    name: None,
                    email: Some("t1@ex.com".into()),
                },
                EmailAddr {
                    name: None,
                    email: Some("t2@ex.com".into()),
                },
            ],
        }]);
        assert_eq!(flatten_addresses(&addr).len(), 2);
    }

    #[test]
    fn exclude_self_case_insensitive() {
        let list = vec![
            ComposerAddress::email_only("Me@Example.COM"),
            ComposerAddress::email_only("you@example.com"),
        ];
        let v = exclude_self(list, "me@example.com");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].email, "you@example.com");
    }
}
