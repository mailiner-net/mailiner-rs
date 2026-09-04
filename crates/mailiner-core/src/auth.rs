//! Parse SPF / DKIM / DMARC results from already-fetched headers.
//!
//! This reads `Authentication-Results`, `ARC-Authentication-Results`,
//! `Received-SPF`, and a few aliases. It does **not** verify signatures.

use serde::{Deserialize, Serialize};

/// Pass / fail / neutral bucket shown in the message header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthVerdict {
    Pass,
    Fail,
    Neutral,
}

impl AuthVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Neutral => "neutral",
        }
    }

    /// Map an RFC 8601 / RFC 7208 result keyword onto a display bucket.
    pub fn from_result(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pass" => Self::Pass,
            "fail" | "softfail" | "permerror" => Self::Fail,
            _ => Self::Neutral,
        }
    }
}

/// Compact SPF / DKIM / DMARC results from authentication headers.
///
/// Absent methods stay `None` so the viewer can hide them. Older cached
/// envelopes deserialize as empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct AuthResults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spf: Option<AuthVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dkim: Option<AuthVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dmarc: Option<AuthVerdict>,
}

impl AuthResults {
    pub fn is_empty(&self) -> bool {
        self.spf.is_none() && self.dkim.is_none() && self.dmarc.is_none()
    }

    /// Present methods in a stable SPF → DKIM → DMARC order.
    pub fn methods(&self) -> impl Iterator<Item = (&'static str, AuthVerdict)> {
        [
            self.spf.map(|v| ("SPF", v)),
            self.dkim.map(|v| ("DKIM", v)),
            self.dmarc.map(|v| ("DMARC", v)),
        ]
        .into_iter()
        .flatten()
    }

    pub fn any_fail(&self) -> bool {
        self.methods().any(|(_, v)| v == AuthVerdict::Fail)
    }

    /// Parse unfolded `(name, value)` header fields.
    ///
    /// First result for each method wins. `Authentication-Results` is
    /// preferred over ARC, then `X-Authentication-Results`, then
    /// `Received-SPF` (SPF only).
    pub fn from_header_fields<I, N, V>(fields: I) -> Self
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: AsRef<str>,
    {
        let mut authres = Vec::new();
        let mut arc = Vec::new();
        let mut x_authres = Vec::new();
        let mut received_spf = Vec::new();
        for (name, value) in fields {
            match classify_header(name.as_ref()) {
                Some(HeaderKind::AuthRes) => authres.push(value),
                Some(HeaderKind::Arc) => arc.push(value),
                Some(HeaderKind::XAuthRes) => x_authres.push(value),
                Some(HeaderKind::ReceivedSpf) => received_spf.push(value),
                None => {}
            }
        }

        let mut out = Self::default();
        for value in authres
            .iter()
            .chain(arc.iter())
            .chain(x_authres.iter())
            .map(AsRef::as_ref)
        {
            apply_authres_value(&mut out, value);
        }
        for value in received_spf {
            apply_received_spf(&mut out, value.as_ref());
        }
        out
    }

    /// Parse a raw RFC 5322 header block (`BODY.PEEK[HEADER]`).
    pub fn from_header_block(raw: &str) -> Self {
        Self::from_header_fields(unfold_header_fields(raw))
    }

    /// Same as [`Self::from_header_block`], with Latin-1 fallback for invalid UTF-8.
    pub fn from_header_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(s) => Self::from_header_block(s),
            Err(_) => {
                let raw: String = bytes.iter().map(|&b| b as char).collect();
                Self::from_header_block(&raw)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum HeaderKind {
    AuthRes,
    Arc,
    XAuthRes,
    ReceivedSpf,
}

fn classify_header(name: &str) -> Option<HeaderKind> {
    if name.eq_ignore_ascii_case("Authentication-Results")
        || name.eq_ignore_ascii_case("Auth-Results")
    {
        Some(HeaderKind::AuthRes)
    } else if name.eq_ignore_ascii_case("ARC-Authentication-Results") {
        Some(HeaderKind::Arc)
    } else if name.eq_ignore_ascii_case("X-Authentication-Results") {
        Some(HeaderKind::XAuthRes)
    } else if name.eq_ignore_ascii_case("Received-SPF") {
        Some(HeaderKind::ReceivedSpf)
    } else {
        None
    }
}

fn apply_authres_value(out: &mut AuthResults, value: &str) {
    let parsed = parse_authres_methods(value);
    set_if_absent(&mut out.spf, reduce_verdicts(&parsed.spf));
    set_if_absent(&mut out.dkim, reduce_verdicts(&parsed.dkim));
    set_if_absent(&mut out.dmarc, reduce_verdicts(&parsed.dmarc));
}

fn apply_received_spf(out: &mut AuthResults, value: &str) {
    if out.spf.is_some() {
        return;
    }
    let i = skip_cfws(value, 0);
    if let Some((_, token)) = read_token(value, i) {
        out.spf = Some(AuthVerdict::from_result(token));
    }
}

fn set_if_absent(slot: &mut Option<AuthVerdict>, next: Option<AuthVerdict>) {
    if slot.is_none() {
        *slot = next;
    }
}

/// One valid pass is enough; else any fail; else neutral.
fn reduce_verdicts(vs: &[AuthVerdict]) -> Option<AuthVerdict> {
    if vs.is_empty() {
        return None;
    }
    if vs.contains(&AuthVerdict::Pass) {
        return Some(AuthVerdict::Pass);
    }
    if vs.contains(&AuthVerdict::Fail) {
        return Some(AuthVerdict::Fail);
    }
    Some(AuthVerdict::Neutral)
}

#[derive(Default)]
struct MethodLists {
    spf: Vec<AuthVerdict>,
    dkim: Vec<AuthVerdict>,
    dmarc: Vec<AuthVerdict>,
}

fn parse_authres_methods(value: &str) -> MethodLists {
    let mut i = skip_cfws(value, 0);
    if i >= value.len() {
        return MethodLists::default();
    }
    if !looks_like_methodspec(value, i) {
        // authserv-id [version]
        if let Some((j, _)) = read_token(value, i) {
            i = skip_cfws(value, j);
        }
        if value.as_bytes().get(i).is_some_and(u8::is_ascii_digit) {
            if let Some((j, _)) = read_token(value, i) {
                i = skip_cfws(value, j);
            }
        }
        if token_is(value, i, "none") {
            return MethodLists::default();
        }
    }
    parse_resinfo_list(value, i)
}

fn looks_like_methodspec(s: &str, i: usize) -> bool {
    let i = skip_cfws(s, i);
    let Some((mut j, _)) = read_token(s, i) else {
        return false;
    };
    j = skip_cfws(s, j);
    if s.as_bytes().get(j) == Some(&b'/') {
        j = skip_cfws(s, j + 1);
        let Some((k, _)) = read_token(s, j) else {
            return false;
        };
        j = skip_cfws(s, k);
    }
    s.as_bytes().get(j) == Some(&b'=')
}

fn parse_resinfo_list(s: &str, mut i: usize) -> MethodLists {
    let mut out = MethodLists::default();
    i = skip_cfws(s, i);
    if token_is(s, i, "none") && !looks_like_methodspec(s, i) {
        return out;
    }
    if s.as_bytes().get(i) == Some(&b';') {
        i += 1;
    }
    while i < s.len() {
        i = skip_cfws(s, i);
        if i >= s.len() {
            break;
        }
        if s.as_bytes()[i] == b';' {
            i += 1;
            continue;
        }
        if let Some((j, method, verdict)) = parse_methodspec(s, i) {
            record_method(&mut out, method, verdict);
            i = skip_until_semi(s, j);
        } else {
            i = skip_until_semi(s, i);
            if s.as_bytes().get(i) == Some(&b';') {
                i += 1;
            } else {
                break;
            }
        }
    }
    out
}

fn parse_methodspec(s: &str, i: usize) -> Option<(usize, &str, AuthVerdict)> {
    let i = skip_cfws(s, i);
    let (mut i, method) = read_token(s, i)?;
    i = skip_cfws(s, i);
    if s.as_bytes().get(i) == Some(&b'/') {
        i = skip_cfws(s, i + 1);
        let (j, _) = read_token(s, i)?;
        i = skip_cfws(s, j);
    }
    if s.as_bytes().get(i) != Some(&b'=') {
        return None;
    }
    i = skip_cfws(s, i + 1);
    let (i, result) = read_token(s, i)?;
    Some((i, method, AuthVerdict::from_result(result)))
}

fn record_method(out: &mut MethodLists, method: &str, verdict: AuthVerdict) {
    match method.to_ascii_lowercase().as_str() {
        "spf" => out.spf.push(verdict),
        "dkim" => out.dkim.push(verdict),
        "dmarc" => out.dmarc.push(verdict),
        _ => {}
    }
}

fn skip_cfws(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'(' {
            i = skip_comment(s, i);
            continue;
        }
        break;
    }
    i
}

fn skip_comment(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    if i >= b.len() || b[i] != b'(' {
        return i;
    }
    let mut depth = 0;
    while i < b.len() {
        match b[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            b'\\' => i = i.saturating_add(2),
            _ => i += 1,
        }
    }
    i.min(b.len())
}

fn skip_quoted(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    if i >= b.len() || b[i] != b'"' {
        return i;
    }
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i = i.saturating_add(2),
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

fn skip_until_semi(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    while i < b.len() {
        match b[i] {
            b';' => return i,
            b'(' => i = skip_comment(s, i),
            b'"' => i = skip_quoted(s, i),
            _ => i += 1,
        }
    }
    i
}

fn read_token(s: &str, start: usize) -> Option<(usize, &str)> {
    let b = s.as_bytes();
    if start >= b.len() || !is_token_byte(b[start]) {
        return None;
    }
    let mut i = start + 1;
    while i < b.len() && is_token_byte(b[i]) {
        i += 1;
    }
    Some((i, &s[start..i]))
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')
}

fn token_is(s: &str, i: usize, want: &str) -> bool {
    match read_token(s, i) {
        Some((_, tok)) => tok.eq_ignore_ascii_case(want),
        None => false,
    }
}

/// Unfold a raw header block into `(name, value)` pairs. Stops at the
/// blank line that ends the headers.
fn unfold_header_fields(raw: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut name = String::new();
    let mut value = String::new();
    let mut have = false;

    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        let folded = line.starts_with([' ', '\t']);
        if folded {
            if have {
                value.push(' ');
                value.push_str(line.trim_start());
            }
            continue;
        }
        if have {
            fields.push((std::mem::take(&mut name), std::mem::take(&mut value)));
            have = false;
        }
        let Some((n, v)) = line.split_once(':') else {
            continue;
        };
        name = n.trim().to_string();
        value = v.trim_start().to_string();
        have = true;
    }
    if have {
        fields.push((name, value));
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(block: &str) -> AuthResults {
        AuthResults::from_header_block(block)
    }

    #[test]
    fn empty_and_missing_are_quiet() {
        assert!(parse("").is_empty());
        assert!(parse("From: a@b.com\r\nSubject: hi\r\n\r\n").is_empty());
        assert!(parse("Authentication-Results: example.com; none\n").is_empty());
        assert!(parse("Authentication-Results: example.com\n").is_empty());
    }

    #[test]
    fn gmail_style_folded_header() {
        let block = "\
Authentication-Results: mx.google.com;\r
       dkim=pass header.i=@github.com header.s=pf2023 header.b=abcd;\r
       dkim=pass header.i=@github.com header.s=pf2023 header.b=efgh;\r
       spf=pass (google.com: domain of noreply@github.com designates 192.30.252.1 as permitted sender) smtp.mailfrom=noreply@github.com;\r
       dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=github.com\r
From: GitHub <noreply@github.com>\r
\r
";
        let auth = parse(block);
        assert_eq!(auth.spf, Some(AuthVerdict::Pass));
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
        assert_eq!(auth.dmarc, Some(AuthVerdict::Pass));
        assert!(!auth.any_fail());
        assert_eq!(
            auth.methods().collect::<Vec<_>>(),
            [
                ("SPF", AuthVerdict::Pass),
                ("DKIM", AuthVerdict::Pass),
                ("DMARC", AuthVerdict::Pass)
            ]
        );
    }

    #[test]
    fn microsoft_omits_authserv_id() {
        let block = "\
Authentication-Results: spf=pass (sender IP is 1.2.3.4)\r
 smtp.mailfrom=notify.example.com; dkim=pass (signature was\r
 verified) header.d=example.com;dmarc=fail action=none\r
 header.from=example.com;compauth=pass reason=100\r
";
        let auth = parse(block);
        assert_eq!(auth.spf, Some(AuthVerdict::Pass));
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
        assert_eq!(auth.dmarc, Some(AuthVerdict::Fail));
        assert!(auth.any_fail());
    }

    #[test]
    fn dkim_pass_wins_over_fail_in_same_header() {
        let auth = parse("Authentication-Results: ex.com; dkim=fail; dkim=pass; dkim=neutral\n");
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
    }

    #[test]
    fn dkim_fail_when_no_pass() {
        let auth = parse("Authentication-Results: ex.com; dkim=fail; dkim=neutral\n");
        assert_eq!(auth.dkim, Some(AuthVerdict::Fail));
    }

    #[test]
    fn first_authres_wins_per_method() {
        let block = "\
Authentication-Results: inbound.example; spf=fail; dkim=pass\r
Authentication-Results: forwarder.example; spf=pass; dkim=fail; dmarc=pass\r
";
        let auth = parse(block);
        assert_eq!(auth.spf, Some(AuthVerdict::Fail));
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
        assert_eq!(auth.dmarc, Some(AuthVerdict::Pass));
    }

    #[test]
    fn arc_and_received_spf_fill_gaps() {
        let block = "\
ARC-Authentication-Results: i=1; mx.example.com; dkim=pass header.i=@ex.com\r
Received-SPF: softfail (example.com: domain of a@b.com is not permitted)\r
";
        let auth = parse(block);
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
        assert_eq!(auth.spf, Some(AuthVerdict::Fail));
        assert!(auth.dmarc.is_none());
    }

    #[test]
    fn authres_preferred_over_received_spf() {
        let block = "\
Authentication-Results: ex.com; spf=pass\r
Received-SPF: fail\r
";
        assert_eq!(parse(block).spf, Some(AuthVerdict::Pass));
    }

    #[test]
    fn result_keywords_bucket() {
        assert_eq!(AuthVerdict::from_result("PASS"), AuthVerdict::Pass);
        assert_eq!(AuthVerdict::from_result("softfail"), AuthVerdict::Fail);
        assert_eq!(AuthVerdict::from_result("permerror"), AuthVerdict::Fail);
        assert_eq!(AuthVerdict::from_result("temperror"), AuthVerdict::Neutral);
        assert_eq!(AuthVerdict::from_result("none"), AuthVerdict::Neutral);
        assert_eq!(AuthVerdict::from_result("policy"), AuthVerdict::Neutral);
        assert_eq!(AuthVerdict::from_result("neutral"), AuthVerdict::Neutral);
    }

    #[test]
    fn versioned_method_and_quoted_reason() {
        let auth =
            parse("Authentication-Results: ex.com 1; dkim/1=pass reason=\"ok;\"; spf=neutral\n");
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
        assert_eq!(auth.spf, Some(AuthVerdict::Neutral));
    }

    #[test]
    fn auth_results_alias_and_x_header() {
        let auth = parse("Auth-Results: ex.com; dmarc=fail\n");
        assert_eq!(auth.dmarc, Some(AuthVerdict::Fail));

        let x = parse("X-Authentication-Results: ex.com; spf=pass\n");
        assert_eq!(x.spf, Some(AuthVerdict::Pass));
    }

    #[test]
    fn serde_skips_empty() {
        let empty = AuthResults::default();
        assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
        let parsed: AuthResults = serde_json::from_str("{}").unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn latin1_header_bytes_do_not_panic() {
        let mut raw = b"Authentication-Results: ex.com; dkim=pass\r\nSubject: caf".to_vec();
        raw.push(0xE9);
        raw.extend_from_slice(b"\r\n\r\n");
        let auth = AuthResults::from_header_bytes(&raw);
        assert_eq!(auth.dkim, Some(AuthVerdict::Pass));
    }
}
