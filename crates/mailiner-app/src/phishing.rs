//! Local sender-spoofing cues for the message viewer.
//!
//! No network or reputation service. Heuristics stay conservative so common
//! mailing lists (`Name <list@host>`, `Brand <notifications@brand.com>`) do
//! not warn. First-time sender is not a cue: every new list would trip it.

use mailiner_core::models::{EmailAddr, EmailAddress};

use crate::ui_prefs::{domain_of_email, normalize_domain, normalize_email};

/// Why the From line looks suspicious.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderCue {
    /// Display name contains an email that is not this mailbox (and not the
    /// same registrable domain).
    DisplayNameMismatch { claimed: String, actual: String },
    /// Sender domain is a lookalike of a domain claimed in the display name,
    /// or of the user's own mailbox domain.
    LookalikeDomain {
        lookalike: String,
        resembles: String,
    },
    /// Domain mixes Latin with another script, or uses punycode (`xn--`).
    HomographDomain { domain: String },
}

impl SenderCue {
    pub fn message(&self) -> String {
        match self {
            Self::DisplayNameMismatch { claimed, actual } => {
                format!("This sender's name looks like {claimed}, but the address is {actual}.")
            }
            Self::LookalikeDomain {
                lookalike,
                resembles,
            } => format!("The sender domain {lookalike} looks similar to {resembles}."),
            Self::HomographDomain { domain } => format!(
                "This sender's domain ({domain}) uses mixed scripts or punycode, which can impersonate another site."
            ),
        }
    }
}

/// Inspect From. `own_email` is the selected account mailbox (lookalike-to-self).
pub fn analyze_from(from: Option<&EmailAddress>, own_email: Option<&str>) -> Vec<SenderCue> {
    let mut cues = Vec::new();
    let own = own_email.and_then(normalize_email);
    let own_domain = own.as_deref().and_then(domain_of_email);
    for addr in flatten_from(from) {
        analyze_addr(addr, own_domain.as_deref(), &mut cues);
    }
    dedup_cues(cues)
}

fn flatten_from(from: Option<&EmailAddress>) -> Vec<&EmailAddr> {
    match from {
        None => Vec::new(),
        Some(EmailAddress::List(list)) => list.iter().collect(),
        Some(EmailAddress::Group(groups)) => groups.iter().flat_map(|g| g.members.iter()).collect(),
    }
}

fn analyze_addr(addr: &EmailAddr, own_domain: Option<&str>, cues: &mut Vec<SenderCue>) {
    let Some(actual) = addr.email.as_deref().and_then(normalize_email) else {
        return;
    };
    let Some(actual_domain) = domain_of_email(&actual) else {
        return;
    };

    if let Some(name) = addr.name.as_deref() {
        for claimed in emails_in_text(name) {
            if claimed == actual {
                continue;
            }
            let Some(claimed_domain) = domain_of_email(&claimed) else {
                continue;
            };
            if same_registrable(&claimed_domain, &actual_domain) {
                continue;
            }
            cues.push(SenderCue::DisplayNameMismatch {
                claimed,
                actual: actual.clone(),
            });
            push_domain_cues(&claimed_domain, &actual_domain, cues);
        }
        if let Some(claimed_domain) = bare_domain_name(name)
            && !same_registrable(&claimed_domain, &actual_domain)
        {
            push_domain_cues(&claimed_domain, &actual_domain, cues);
        }
        if let Some(brand) = display_name_brand_sld(name)
            && let Some(actual_sld) = sld_of(&actual_domain)
            && brand != actual_sld.as_str()
            && labels_lookalike(brand, &actual_sld)
        {
            cues.push(SenderCue::LookalikeDomain {
                lookalike: registrable_domain(&actual_domain)
                    .unwrap_or_else(|| actual_domain.clone()),
                resembles: format!("{brand}.com"),
            });
        }
    }

    if domain_is_homograph(&actual_domain) {
        cues.push(SenderCue::HomographDomain {
            domain: actual_domain.clone(),
        });
    }

    if let Some(own_domain) = own_domain
        && !same_registrable(&actual_domain, own_domain)
    {
        push_domain_cues(own_domain, &actual_domain, cues);
    }
}

fn push_domain_cues(trusted: &str, observed: &str, cues: &mut Vec<SenderCue>) {
    if same_registrable(trusted, observed) {
        return;
    }
    if domains_lookalike(trusted, observed) || embeds_foreign_domain(observed, trusted) {
        let lookalike = registrable_domain(observed).unwrap_or_else(|| observed.to_string());
        let resembles = registrable_domain(trusted).unwrap_or_else(|| trusted.to_string());
        if lookalike != resembles {
            cues.push(SenderCue::LookalikeDomain {
                lookalike,
                resembles,
            });
        }
    }
}

fn dedup_cues(cues: Vec<SenderCue>) -> Vec<SenderCue> {
    let mut out = Vec::new();
    for cue in cues {
        if out.contains(&cue) {
            continue;
        }
        // Mismatch already names both mailboxes; drop the matching lookalike.
        if let SenderCue::LookalikeDomain {
            lookalike,
            resembles,
        } = &cue
        {
            let redundant = out.iter().any(|existing| match existing {
                SenderCue::DisplayNameMismatch { claimed, actual } => {
                    domains_match_pair(claimed, actual, resembles, lookalike)
                }
                _ => false,
            });
            if redundant {
                continue;
            }
        }
        out.push(cue);
    }
    out
}

fn domains_match_pair(claimed: &str, actual: &str, resembles: &str, lookalike: &str) -> bool {
    let claimed_d = domain_of_email(claimed);
    let actual_d = domain_of_email(actual);
    match (claimed_d, actual_d) {
        (Some(c), Some(a)) => {
            (same_registrable(&c, resembles) && same_registrable(&a, lookalike))
                || (same_registrable(&c, lookalike) && same_registrable(&a, resembles))
        }
        _ => false,
    }
}

fn emails_in_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let (left, after_at) = rest.split_at(at);
        let local = email_local_end(left);
        let domain = email_domain_start(&after_at[1..]);
        match (local, domain) {
            (Some(local), Some(domain)) => {
                if let Some(norm) = normalize_email(&format!("{local}@{domain}"))
                    && !out.contains(&norm)
                {
                    out.push(norm);
                }
                rest = &after_at[1 + domain.len()..];
            }
            _ => rest = &after_at[1..],
        }
    }
    out
}

fn email_local_end(left: &str) -> Option<&str> {
    let bytes = left.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        let b = bytes[i - 1];
        if is_local_byte(b) {
            i -= 1;
        } else {
            break;
        }
    }
    let local = left[i..].trim_matches('.');
    if local.is_empty() || local.starts_with('.') {
        None
    } else {
        Some(local)
    }
}

fn is_local_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn email_domain_start(after_at: &str) -> Option<&str> {
    let bytes = after_at.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' {
            i += 1;
        } else {
            break;
        }
    }
    let domain = after_at[..i].trim_matches('.');
    if domain.contains('.') && !domain.starts_with('-') && !domain.ends_with('-') {
        Some(domain)
    } else {
        None
    }
}

/// First token of a display name matches a well-known brand SLD.
///
/// Used only with a lookalike mailbox domain (`PayPal` + `paypa1.com`).
/// `PayPal` + `gmail.com` is not a lookalike and stays quiet.
fn display_name_brand_sld(name: &str) -> Option<&'static str> {
    let n = name.trim().to_ascii_lowercase();
    let first = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|s| !s.is_empty())?;
    BRAND_SLDS.iter().copied().find(|brand| *brand == first)
}

/// Whole display name is a bare host (`paypal.com`), not a person name.
fn bare_domain_name(name: &str) -> Option<String> {
    let trimmed = name.trim().trim_matches(|c| c == '"' || c == '\'');
    if trimmed.is_empty() || trimmed.contains([' ', '@', '/', ':', '<', '>']) {
        return None;
    }
    if !trimmed.contains('.') {
        return None;
    }
    normalize_domain(trimmed)
}

fn same_registrable(a: &str, b: &str) -> bool {
    match (registrable_domain(a), registrable_domain(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// eTLD+1 using a small multi-label public-suffix list (no network PSL).
fn registrable_domain(domain: &str) -> Option<String> {
    let d = normalize_domain(domain)?;
    if d.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }
    for tld in MULTI_LABEL_TLDS {
        let suffix = format!(".{tld}");
        if let Some(rest) = d.strip_suffix(&suffix) {
            let sld = rest.rsplit('.').next().filter(|s| !s.is_empty())?;
            return Some(format!("{sld}.{tld}"));
        }
        if d == *tld {
            return None;
        }
    }
    let mut labels = d.rsplit('.');
    let tld = labels.next()?;
    let sld = labels.next()?;
    if tld.is_empty() || sld.is_empty() {
        return None;
    }
    Some(format!("{sld}.{tld}"))
}

fn sld_of(domain: &str) -> Option<String> {
    let reg = registrable_domain(domain)?;
    Some(reg.split('.').next()?.to_string())
}

fn domains_lookalike(trusted: &str, observed: &str) -> bool {
    let Some(a) = sld_of(trusted) else {
        return false;
    };
    let Some(b) = sld_of(observed) else {
        return false;
    };
    if a == b {
        // Same SLD, different TLD (`company.com` vs `company.net`).
        let Some(ra) = registrable_domain(trusted) else {
            return false;
        };
        let Some(rb) = registrable_domain(observed) else {
            return false;
        };
        return ra != rb && a.len() >= 5;
    }
    labels_lookalike(&a, &b)
}

/// `paypal.com.evil.ru` — trusted SLD appears as a non-registrable label.
fn embeds_foreign_domain(observed: &str, trusted: &str) -> bool {
    let Some(trusted_sld) = sld_of(trusted) else {
        return false;
    };
    if trusted_sld.len() < 4 {
        return false;
    }
    let Some(obs) = normalize_domain(observed) else {
        return false;
    };
    let Some(obs_sld) = sld_of(&obs) else {
        return false;
    };
    if obs_sld == trusted_sld {
        return false;
    }
    let folded_trusted = fold_confusables(&trusted_sld);
    obs.split('.').any(|label| {
        if label == obs_sld.as_str() {
            return false;
        }
        let folded = fold_confusables(label);
        folded == folded_trusted || labels_lookalike(label, &trusted_sld)
    })
}

fn labels_lookalike(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return false;
    }
    let a = fold_confusables(a);
    let b = fold_confusables(b);
    if a == b {
        return true;
    }
    let (shorter, longer) = if a.len() <= b.len() {
        (a.as_str(), b.as_str())
    } else {
        (b.as_str(), a.as_str())
    };
    if shorter.len() >= 5 && longer.len() > shorter.len() {
        if (longer.starts_with(shorter) || longer.ends_with(shorter))
            && longer.len() - shorter.len() <= 8
        {
            return true;
        }
        if shorter.len() >= 6 && longer.contains(shorter) {
            return true;
        }
    }
    let dist = levenshtein(&a, &b);
    match dist {
        // `gmail`/`email` is one leading-letter swap — too common to warn.
        1 if shorter.len() >= 5 && !leading_letter_swap(&a, &b) => true,
        2 if shorter.len() >= 8 => true,
        _ => false,
    }
}

fn leading_letter_swap(a: &str, b: &str) -> bool {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    ac.len() == bc.len() && ac.len() <= 6 && ac.first() != bc.first() && ac.get(1..) == bc.get(1..)
}

fn domain_is_homograph(domain: &str) -> bool {
    let d = domain.trim_end_matches('.');
    if d.split('.')
        .any(|label| label.len() >= 4 && label[..4].eq_ignore_ascii_case("xn--"))
    {
        return true;
    }
    mixed_scripts(d)
}

fn mixed_scripts(domain: &str) -> bool {
    let mut latin = false;
    let mut other = false;
    for c in domain.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        if c.is_ascii_alphabetic() {
            latin = true;
        } else {
            other = true;
        }
        if latin && other {
            return true;
        }
    }
    false
}

fn fold_confusables(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        out.push(fold_confusable_char(c));
    }
    out
}

fn fold_confusable_char(c: char) -> char {
    match c {
        '0' | '\u{03bf}' | '\u{043e}' => 'o', // Greek/Cyrillic o
        '1' | 'l' | 'I' | '|' | '\u{0456}' | '\u{04cf}' => 'i',
        '3' => 'e',
        '4' => 'a',
        '5' => 's',
        '7' => 't',
        '\u{0430}' => 'a', // Cyrillic a
        '\u{0435}' => 'e',
        '\u{0440}' => 'p',
        '\u{0441}' => 'c',
        '\u{0443}' => 'y',
        '\u{0445}' => 'x',
        '\u{0455}' => 's',
        '\u{0501}' => 'd',
        _ => c.to_ascii_lowercase(),
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = b.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0; m + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// High-value brand SLDs for display-name + lookalike-domain checks.
/// Not a reputation list — compared locally to the mailbox SLD only.
const BRAND_SLDS: &[&str] = &[
    "paypal",
    "apple",
    "google",
    "gmail",
    "microsoft",
    "outlook",
    "hotmail",
    "amazon",
    "facebook",
    "instagram",
    "whatsapp",
    "netflix",
    "linkedin",
    "github",
    "dropbox",
    "adobe",
    "chase",
];

/// Common two-label public suffixes. Not a full PSL — enough to avoid
/// treating `co.uk` as the registrable domain of `paypal.co.uk`.
const MULTI_LABEL_TLDS: &[&str] = &[
    "ac.uk", "co.uk", "gov.uk", "ltd.uk", "me.uk", "net.uk", "org.uk", "plc.uk", "sch.uk",
    "com.au", "net.au", "org.au", "edu.au", "gov.au", "co.nz", "net.nz", "org.nz", "co.jp",
    "or.jp", "ne.jp", "ac.jp", "go.jp", "co.za", "org.za", "com.br", "net.br", "org.br", "com.mx",
    "co.in", "net.in", "org.in", "ac.in", "gov.in", "com.cn", "net.cn", "org.cn", "com.tw",
    "com.hk", "co.kr", "com.sg", "com.my", "com.ar", "com.tr", "co.id",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(name: Option<&str>, email: Option<&str>) -> EmailAddr {
        EmailAddr {
            name: name.map(str::to_string),
            email: email.map(str::to_string),
        }
    }

    fn list(name: Option<&str>, email: Option<&str>) -> EmailAddress {
        EmailAddress::List(vec![addr(name, email)])
    }

    fn kinds(cues: &[SenderCue]) -> Vec<&'static str> {
        cues.iter()
            .map(|c| match c {
                SenderCue::DisplayNameMismatch { .. } => "mismatch",
                SenderCue::LookalikeDomain { .. } => "lookalike",
                SenderCue::HomographDomain { .. } => "homograph",
            })
            .collect()
    }

    #[test]
    fn ordinary_from_is_quiet() {
        assert!(analyze_from(Some(&list(Some("Ada"), Some("ada@example.com"))), None).is_empty());
        assert!(analyze_from(Some(&list(None, Some("ada@example.com"))), None).is_empty());
        assert!(analyze_from(None, None).is_empty());
    }

    #[test]
    fn mailing_lists_do_not_warn() {
        assert!(
            analyze_from(
                Some(&list(Some("GitHub"), Some("notifications@github.com"))),
                None
            )
            .is_empty()
        );
        assert!(
            analyze_from(
                Some(&list(
                    Some("Jane Doe via rust-dev"),
                    Some("rust-dev@mailman.example.com")
                )),
                None
            )
            .is_empty()
        );
        assert!(
            analyze_from(
                Some(&list(Some("Jane Doe"), Some("jane@work.example"))),
                None
            )
            .is_empty()
        );
        // Same address in the name, or same registrable domain.
        assert!(
            analyze_from(
                Some(&list(
                    Some("notifications@github.com"),
                    Some("notifications@github.com")
                )),
                None
            )
            .is_empty()
        );
        assert!(
            analyze_from(
                Some(&list(
                    Some("security@paypal.com"),
                    Some("noreply@mail.paypal.com")
                )),
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn display_name_email_mismatch() {
        let cues = analyze_from(
            Some(&list(
                Some("support@paypal.com"),
                Some("attacker@evil.example"),
            )),
            None,
        );
        assert_eq!(kinds(&cues), ["mismatch"]);
        match &cues[0] {
            SenderCue::DisplayNameMismatch { claimed, actual } => {
                assert_eq!(claimed, "support@paypal.com");
                assert_eq!(actual, "attacker@evil.example");
            }
            other => panic!("{other:?}"),
        }
        assert!(cues[0].message().contains("support@paypal.com"));
        assert!(cues[0].message().contains("attacker@evil.example"));
    }

    #[test]
    fn display_name_can_embed_the_claimed_email() {
        let cues = analyze_from(
            Some(&list(
                Some("PayPal Support support@paypal.com"),
                Some("phish@gmail.com"),
            )),
            None,
        );
        assert!(kinds(&cues).contains(&"mismatch"));
    }

    #[test]
    fn lookalike_typo_in_sender_domain() {
        let cues = analyze_from(
            Some(&list(Some("PayPal"), Some("service@paypa1.com"))),
            Some("me@paypal.com"),
        );
        assert!(
            cues.iter().any(|c| matches!(
                c,
                SenderCue::LookalikeDomain {
                    lookalike,
                    resembles
                } if lookalike == "paypa1.com" && resembles.contains("paypal")
            )),
            "{cues:?}"
        );
    }

    #[test]
    fn brand_display_name_plus_lookalike_domain() {
        let cues = analyze_from(
            Some(&list(Some("PayPal Support"), Some("service@paypa1.com"))),
            None,
        );
        assert!(
            cues.iter()
                .any(|c| matches!(c, SenderCue::LookalikeDomain { .. })),
            "{cues:?}"
        );
        // Brand name on an unrelated mailbox is not a lookalike.
        assert!(
            analyze_from(Some(&list(Some("PayPal"), Some("friend@gmail.com"))), None).is_empty()
        );
    }

    #[test]
    fn lookalike_from_claimed_domain() {
        let cues = analyze_from(
            Some(&list(Some("paypal.com"), Some("help@paypa1.com"))),
            None,
        );
        assert!(
            cues.iter()
                .any(|c| matches!(c, SenderCue::LookalikeDomain { .. })),
            "{cues:?}"
        );
    }

    #[test]
    fn lookalike_embedded_brand_label() {
        let cues = analyze_from(
            Some(&list(
                Some("security@paypal.com"),
                Some("login@paypal.com.evil.example"),
            )),
            None,
        );
        assert!(kinds(&cues).contains(&"mismatch"), "{cues:?}");
        assert!(
            cues.iter().any(|c| matches!(
                c,
                SenderCue::LookalikeDomain { lookalike, .. } if lookalike.contains("evil")
            )) || cues
                .iter()
                .any(|c| matches!(c, SenderCue::DisplayNameMismatch { .. })),
            "{cues:?}"
        );
    }

    #[test]
    fn mixed_script_domain_is_homograph() {
        // Cyrillic 'а' in otherwise Latin paypal.
        let cues = analyze_from(
            Some(&list(Some("PayPal"), Some("help@p\u{0430}ypal.com"))),
            None,
        );
        assert!(
            cues.iter()
                .any(|c| matches!(c, SenderCue::HomographDomain { .. })),
            "{cues:?}"
        );
    }

    #[test]
    fn punycode_label_is_homograph() {
        let cues = analyze_from(Some(&list(None, Some("help@xn--pypal-4ve.com"))), None);
        assert!(
            cues.iter()
                .any(|c| matches!(c, SenderCue::HomographDomain { domain } if domain == "xn--pypal-4ve.com")),
            "{cues:?}"
        );
    }

    #[test]
    fn short_domains_are_not_lookalikes() {
        assert!(
            analyze_from(
                Some(&list(Some("IBM"), Some("sales@ibm.co"))),
                Some("me@ibm.com")
            )
            .is_empty()
        );
    }

    #[test]
    fn same_org_subdomain_is_not_lookalike() {
        assert!(
            analyze_from(
                Some(&list(Some("IT"), Some("alerts@mail.company.com"))),
                Some("me@company.com")
            )
            .is_empty()
        );
    }

    #[test]
    fn own_domain_tld_swap_is_lookalike() {
        let cues = analyze_from(
            Some(&list(Some("IT"), Some("alerts@company.net"))),
            Some("me@company.com"),
        );
        assert!(
            cues.iter().any(|c| matches!(
                c,
                SenderCue::LookalikeDomain {
                    lookalike,
                    resembles
                } if lookalike == "company.net" && resembles == "company.com"
            )),
            "{cues:?}"
        );
    }

    #[test]
    fn multi_label_tld_keeps_sld() {
        assert_eq!(
            registrable_domain("www.paypal.co.uk").as_deref(),
            Some("paypal.co.uk")
        );
        assert_eq!(
            registrable_domain("mail.example.com").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn group_from_is_scanned() {
        let from = EmailAddress::Group(vec![mailiner_core::models::Group {
            name: Some("Team".into()),
            members: vec![addr(
                Some("support@apple.com"),
                Some("not-apple@evil.example"),
            )],
        }]);
        let cues = analyze_from(Some(&from), None);
        assert!(kinds(&cues).contains(&"mismatch"));
    }

    #[test]
    fn hyphenated_lookalike() {
        assert!(labels_lookalike("paypal", "paypal-secure"));
        assert!(labels_lookalike("microsoft", "microsft"));
        assert!(!labels_lookalike("paypal", "paypal"));
        assert!(!labels_lookalike("info", "invo"));
        assert!(!labels_lookalike("gmail", "email"));
    }

    #[test]
    fn digit_homoglyph_folds() {
        assert!(labels_lookalike("paypal", "paypa1"));
        assert!(labels_lookalike("microsoft", "m1crosoft"));
    }
}
