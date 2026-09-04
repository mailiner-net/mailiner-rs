//! Import/export of `.eml` and mbox, plus a STORE-only zip writer.
//!
//! Parsing and packing stay in Rust so host unit tests cover the format
//! without WASM. IMAP APPEND wants CRLF; browsers often hand us LF-only files.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::download::eml_filename;

/// Hard cap on messages unpacked from one import batch (all files combined).
pub const MAX_IMPORT_MESSAGES: usize = 200;
/// Hard cap on messages in one export (zip or mbox).
pub const MAX_EXPORT_MESSAGES: usize = 200;

/// One RFC 822 message ready for IMAP APPEND or a zip/mbox entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rfc822Message {
    pub filename: String,
    pub bytes: Vec<u8>,
}

/// How to pack a multi-message export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailExportFormat {
    /// One `.eml` per message, zipped when there is more than one.
    EmlZip,
    /// Classic mboxrd (`From ` separators, `>From ` escape).
    Mbox,
}

/// One selected message to include in an export.
#[derive(Clone, Debug)]
pub struct ExportMessageItem {
    pub message_id: crate::message::MessageId,
    pub filename: String,
    pub size_hint: Option<u64>,
}

/// Unique `.eml` names in list order for the given subjects and ids.
pub fn export_items_from(
    ordered: impl IntoIterator<Item = (crate::message::MessageId, String, Option<u64>)>,
) -> Vec<ExportMessageItem> {
    let rows: Vec<_> = ordered.into_iter().collect();
    let names = unique_eml_filenames(rows.iter().map(|(_, subject, _)| subject.as_str()));
    rows.into_iter()
        .zip(names)
        .map(|((message_id, _, size_hint), filename)| ExportMessageItem {
            message_id,
            filename,
            size_hint,
        })
        .collect()
}

/// Filename extension used to pick a parser (content can still override).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportKind {
    Eml,
    Mbox,
}

/// Split uploaded bytes into RFC 822 messages.
///
/// `.mbox` (or a `From ` envelope with another separator) is split. Anything
/// else is one message. Leading `From ` envelope lines are stripped.
pub fn parse_import_file(filename: &str, bytes: &[u8]) -> Result<Vec<Rfc822Message>, String> {
    if bytes.is_empty() {
        return Err(format!("\"{filename}\" is empty"));
    }
    let kind = import_kind(filename, bytes);
    match kind {
        ImportKind::Mbox => parse_mbox(filename, bytes),
        ImportKind::Eml => {
            let rfc822 = strip_from_envelope(bytes);
            let normalized = normalize_rfc822(rfc822)?;
            Ok(vec![Rfc822Message {
                filename: eml_filename_from_import(filename, &normalized),
                bytes: normalized,
            }])
        }
    }
}

/// Unpack every file; stop before exceeding [`MAX_IMPORT_MESSAGES`].
pub fn parse_import_files(
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<Vec<Rfc822Message>, String> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        let parsed = parse_import_file(&name, &bytes)?;
        if out.len() + parsed.len() > MAX_IMPORT_MESSAGES {
            return Err(format!(
                "Import is limited to {MAX_IMPORT_MESSAGES} messages"
            ));
        }
        out.extend(parsed);
    }
    if out.is_empty() {
        return Err("No messages found in the selected files".into());
    }
    Ok(out)
}

/// Unique `.eml` names from subjects (or a fallback stem). Collisions get `-2`.
pub fn unique_eml_filenames<I, S>(subjects: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut used = HashSet::new();
    subjects
        .into_iter()
        .map(|s| unique_name(&mut used, &eml_filename(s.as_ref())))
        .collect()
}

/// mboxrd bytes for `messages` (already CRLF RFC 822).
pub fn encode_mbox(messages: &[Rfc822Message]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(mbox_from_line(&msg.bytes).as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend(escape_mbox_body(&msg.bytes));
        if !msg.bytes.ends_with(b"\r\n") {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

/// Suggested download name for a packed export.
pub fn export_archive_filename(folder: &str, format: MailExportFormat) -> String {
    let stem = archive_stem(folder);
    match format {
        MailExportFormat::EmlZip => format!("{stem}-messages.zip"),
        MailExportFormat::Mbox => format!("{stem}.mbox"),
    }
}

/// Uncompressed ZIP (STORE). Emails are already encoded; skip a zip crate.
pub fn zip_store(files: &[(String, &[u8])]) -> Result<Vec<u8>, String> {
    if files.is_empty() {
        return Err("no files to zip".into());
    }
    let mut local = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        if name.len() > u16::MAX as usize {
            return Err("zip entry name is too long".into());
        }
        if data.len() > u32::MAX as usize {
            return Err("zip entry is too large".into());
        }
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let size = data.len() as u32;
        let offset = local.len() as u32;

        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let cd_offset = local.len() as u32;
    let cd_size = central.len() as u32;
    let n = files.len() as u16;
    local.extend_from_slice(&central);
    local.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&n.to_le_bytes());
    local.extend_from_slice(&n.to_le_bytes());
    local.extend_from_slice(&cd_size.to_le_bytes());
    local.extend_from_slice(&cd_offset.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    Ok(local)
}

/// Pack fetched messages for download. One `.eml` stays a single file.
pub fn pack_export(
    messages: &[Rfc822Message],
    format: MailExportFormat,
) -> Result<(String, &'static str, Vec<u8>), String> {
    if messages.is_empty() {
        return Err("no messages to export".into());
    }
    match format {
        MailExportFormat::EmlZip if messages.len() == 1 => Ok((
            messages[0].filename.clone(),
            "message/rfc822",
            messages[0].bytes.clone(),
        )),
        MailExportFormat::EmlZip => {
            let files: Vec<(String, &[u8])> = messages
                .iter()
                .map(|m| (m.filename.clone(), m.bytes.as_slice()))
                .collect();
            let zip = zip_store(&files)?;
            Ok(("messages.zip".into(), "application/zip", zip))
        }
        MailExportFormat::Mbox => Ok((
            "messages.mbox".into(),
            "application/mbox",
            encode_mbox(messages),
        )),
    }
}

/// Same as [`pack_export`], but names the archive after `folder`.
pub fn pack_export_named(
    messages: &[Rfc822Message],
    format: MailExportFormat,
    folder: &str,
) -> Result<(String, &'static str, Vec<u8>), String> {
    let (name, mime, bytes) = pack_export(messages, format)?;
    if messages.len() == 1 && format == MailExportFormat::EmlZip {
        return Ok((name, mime, bytes));
    }
    Ok((export_archive_filename(folder, format), mime, bytes))
}

fn import_kind(filename: &str, bytes: &[u8]) -> ImportKind {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".mbox") {
        return ImportKind::Mbox;
    }
    if lower.ends_with(".eml") {
        return ImportKind::Eml;
    }
    if looks_like_mbox(bytes) {
        ImportKind::Mbox
    } else {
        ImportKind::Eml
    }
}

fn looks_like_mbox(bytes: &[u8]) -> bool {
    let text = normalize_newlines(bytes);
    if !text.starts_with(b"From ") {
        return false;
    }
    text.windows(6).skip(1).any(|w| w == b"\nFrom ")
}

fn parse_mbox(filename: &str, bytes: &[u8]) -> Result<Vec<Rfc822Message>, String> {
    let text = normalize_newlines(bytes);
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < text.len() {
        if is_from_separator(&text, i) && i > start {
            parts.push(&text[start..i]);
            start = i;
        }
        i = match text[i..].iter().position(|&b| b == b'\n') {
            Some(n) => i + n + 1,
            None => text.len(),
        };
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    if parts.is_empty() {
        return Err(format!("\"{filename}\" contains no mbox messages"));
    }
    let mut out = Vec::with_capacity(parts.len());
    let mut used = HashSet::new();
    for (idx, part) in parts.iter().enumerate() {
        let body = unescape_mbox_body(strip_from_envelope(part));
        if body.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let normalized = normalize_rfc822(&body).map_err(|e| format!("\"{filename}\": {e}"))?;
        let fallback = format!("{}-{}", import_stem(filename), idx + 1);
        let name = unique_name(&mut used, &eml_filename_from_import(&fallback, &normalized));
        out.push(Rfc822Message {
            filename: name,
            bytes: normalized,
        });
    }
    if out.is_empty() {
        return Err(format!("\"{filename}\" contains no mbox messages"));
    }
    Ok(out)
}

fn is_from_separator(text: &[u8], i: usize) -> bool {
    if i > 0 && text[i - 1] != b'\n' {
        return false;
    }
    text[i..].starts_with(b"From ")
}

fn strip_from_envelope(bytes: &[u8]) -> &[u8] {
    let text = bytes;
    let line_end = text.iter().position(|&b| b == b'\n').unwrap_or(text.len());
    let first = &text[..line_end];
    let first_trim = first.strip_suffix(b"\r").unwrap_or(first);
    if first_trim.starts_with(b"From ") {
        let rest = if line_end < text.len() {
            &text[line_end + 1..]
        } else {
            b""
        };
        return rest;
    }
    text
}

/// LF / mixed newlines → CRLF. Rejects empty or header-less payloads.
pub fn normalize_rfc822(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(bytes.len() + 16);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push(b'\r');
            out.push(b'\n');
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'\n' {
            out.extend_from_slice(b"\r\n");
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    while out.ends_with(b"\r\n") {
        let keep = out.len() - 2;
        if out[..keep].ends_with(b"\r\n") {
            out.truncate(keep);
        } else {
            break;
        }
    }
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    if out.iter().all(|b| b.is_ascii_whitespace()) {
        return Err("message is empty".into());
    }
    if !has_header_line(&out) {
        return Err("file does not look like an email message".into());
    }
    Ok(out)
}

fn has_header_line(bytes: &[u8]) -> bool {
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(bytes.len());
    let headers = &bytes[..split.min(bytes.len())];
    headers.split(|&b| b == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        header_name_end(line).is_some()
    })
}

fn header_name_end(line: &[u8]) -> Option<usize> {
    if line.is_empty() || line[0].is_ascii_whitespace() {
        return None;
    }
    let colon = line.iter().position(|&b| b == b':')?;
    if colon == 0 {
        return None;
    }
    if line[..colon]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        Some(colon)
    } else {
        None
    }
}

fn import_stem(filename: &str) -> &str {
    let stem = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    stem.rsplit_once('.')
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty())
        .unwrap_or("message")
}

fn header_value_owned(bytes: &[u8], name: &str) -> Option<String> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"\r\n\r\n") || bytes[i..].starts_with(b"\n\n") {
            break;
        }
        let eol = bytes[i..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|n| i + n)
            .unwrap_or(bytes.len());
        let line = bytes[i..eol].strip_suffix(b"\r").unwrap_or(&bytes[i..eol]);
        if let Some(colon) = header_name_end(line)
            && line[..colon].eq_ignore_ascii_case(name.as_bytes())
        {
            let mut val = line[colon + 1..].to_vec();
            let mut j = if eol < bytes.len() {
                eol + 1
            } else {
                bytes.len()
            };
            while j < bytes.len() {
                let next_eol = bytes[j..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|n| j + n)
                    .unwrap_or(bytes.len());
                let next = bytes[j..next_eol]
                    .strip_suffix(b"\r")
                    .unwrap_or(&bytes[j..next_eol]);
                if next.first().is_some_and(|b| b.is_ascii_whitespace()) {
                    val.push(b' ');
                    val.extend_from_slice(next.trim_ascii_start());
                    j = if next_eol < bytes.len() {
                        next_eol + 1
                    } else {
                        bytes.len()
                    };
                } else {
                    break;
                }
            }
            return Some(String::from_utf8_lossy(&val).trim().to_string());
        }
        i = if eol < bytes.len() {
            eol + 1
        } else {
            bytes.len()
        };
    }
    None
}

fn eml_filename_from_import(filename: &str, rfc822: &[u8]) -> String {
    if let Some(subject) = header_value_owned(rfc822, "Subject")
        && !subject.is_empty()
    {
        return eml_filename(&subject);
    }
    let stem = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let stem = stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem);
    eml_filename(stem)
}

fn unique_name(used: &mut HashSet<String>, name: &str) -> String {
    if used.insert(name.to_string()) {
        return name.to_string();
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let mut n = 2u32;
    loop {
        let candidate = if ext.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n = n.saturating_add(1);
        if n == u32::MAX {
            return format!("{stem}-{n}.{ext}");
        }
    }
}

fn archive_stem(folder: &str) -> String {
    let name = eml_filename(folder);
    let stem = name
        .strip_suffix(".eml")
        .unwrap_or(&name)
        .trim()
        .to_string();
    if stem.is_empty() || stem == "message" {
        "mailbox".into()
    } else {
        stem
    }
}

fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push(b'\n');
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn escape_mbox_body(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 8);
    let mut start = 0usize;
    while start < bytes.len() {
        let rel = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(bytes.len() - start);
        let line = &bytes[start..start + rel];
        let content = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(line);
        if content.starts_with(b"From ") || content.starts_with(b">From ") {
            out.push(b'>');
        }
        out.extend_from_slice(line);
        start += rel;
    }
    out
}

fn unescape_mbox_body(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut start = 0usize;
    while start < bytes.len() {
        let rel = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(bytes.len() - start);
        let line = &bytes[start..start + rel];
        let content = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(line);
        if content.starts_with(b">From ") {
            out.extend_from_slice(&line[1..]);
        } else {
            out.extend_from_slice(line);
        }
        start += rel;
    }
    out
}

fn mbox_from_line(rfc822: &[u8]) -> String {
    let addr = header_value_owned(rfc822, "From")
        .and_then(|v| extract_email(&v))
        .unwrap_or_else(|| "MAILER-DAEMON".into());
    let date = header_value_owned(rfc822, "Date")
        .and_then(|v| DateTime::parse_from_rfc2822(&v).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    format!("From {addr} {}", date.format("%a %b %e %H:%M:%S %Y"))
}

fn extract_email(from: &str) -> Option<String> {
    if let Some(start) = from.rfind('<')
        && let Some(end) = from[start + 1..].find('>')
    {
        let email = from[start + 1..start + 1 + end].trim();
        if email.contains('@') {
            return Some(email.to_string());
        }
    }
    let token = from.split_whitespace().last()?.trim();
    if token.contains('@') {
        Some(
            token
                .trim_matches(|c| c == '<' || c == '>' || c == ',')
                .to_string(),
        )
    } else {
        None
    }
}

const CRC_TABLE: [u32; 256] = crc32_table();

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = 0xEDB8_8320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = CRC_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    const EML: &[u8] = b"From: a@b.com\r\nTo: c@d.com\r\nSubject: Hello\r\n\r\nHi\r\n";
    const EML_LF: &[u8] = b"From: a@b.com\nTo: c@d.com\nSubject: Hello\n\nHi\n";

    #[test]
    fn normalize_lf_to_crlf() {
        let got = normalize_rfc822(EML_LF).unwrap();
        assert_eq!(got, EML);
    }

    #[test]
    fn normalize_rejects_empty_and_non_mail() {
        assert!(normalize_rfc822(b"").is_err());
        assert!(normalize_rfc822(b"   \n\n").is_err());
        assert!(normalize_rfc822(b"not an email\n\nbody\n").is_err());
    }

    #[test]
    fn parse_eml_strips_from_envelope() {
        let raw =
            b"From MAILER-DAEMON Fri Sep  4 00:00:00 2026\nFrom: a@b.com\nSubject: X\n\nbody\n";
        let msgs = parse_import_file("note.eml", raw).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].filename, "X.eml");
        assert!(msgs[0].bytes.starts_with(b"From: a@b.com\r\n"));
        assert!(!msgs[0].bytes.starts_with(b"From MAILER"));
    }

    #[test]
    fn parse_mbox_splits_and_unescapes() {
        let raw = b"From a@b.com Fri Sep  4 00:00:00 2026\n\
From: a@b.com\nSubject: One\n\n>From the top\n\n\
From c@d.com Fri Sep  4 00:00:01 2026\n\
From: c@d.com\nSubject: Two\n\nsecond\n";
        let msgs = parse_import_file("inbox.mbox", raw).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].filename, "One.eml");
        assert_eq!(msgs[1].filename, "Two.eml");
        let body = std::str::from_utf8(&msgs[0].bytes).unwrap();
        assert!(body.contains("From the top"));
        assert!(!body.contains(">From the top"));
    }

    #[test]
    fn mbox_round_trip() {
        let msgs = vec![
            Rfc822Message {
                filename: "One.eml".into(),
                bytes: b"From: a@b.com\r\nSubject: One\r\n\r\nFrom the top\r\n".to_vec(),
            },
            Rfc822Message {
                filename: "Two.eml".into(),
                bytes: b"From: c@d.com\r\nSubject: Two\r\n\r\nsecond\r\n".to_vec(),
            },
        ];
        let packed = encode_mbox(&msgs);
        let text = String::from_utf8_lossy(&packed);
        assert!(text.starts_with("From a@b.com "));
        assert!(text.contains("\r\n>From the top\r\n"));
        let parsed = parse_import_file("round.mbox", &packed).unwrap();
        assert_eq!(parsed.len(), 2);
        let first = std::str::from_utf8(&parsed[0].bytes).unwrap();
        assert!(first.contains("From the top"));
        assert!(!first.contains(">From the top"));
        assert!(
            std::str::from_utf8(&parsed[1].bytes)
                .unwrap()
                .contains("second")
        );
    }

    #[test]
    fn unique_filenames_disambiguate() {
        let names = unique_eml_filenames(["Hello", "Hello", "Other"]);
        assert_eq!(names, ["Hello.eml", "Hello-2.eml", "Other.eml"]);
    }

    #[test]
    fn zip_store_contains_entries() {
        let a = b"aaa";
        let b = b"bbbb";
        let zip = zip_store(&[
            ("a.eml".into(), a.as_slice()),
            ("b.eml".into(), b.as_slice()),
        ])
        .unwrap();
        assert_eq!(&zip[0..4], b"PK\x03\x04");
        assert!(zip.windows(5).any(|w| w == b"a.eml"));
        assert!(zip.windows(5).any(|w| w == b"b.eml"));
        assert_eq!(&zip[zip.len() - 22..zip.len() - 18], b"PK\x05\x06");
        let n = u16::from_le_bytes(zip[zip.len() - 14..zip.len() - 12].try_into().unwrap());
        assert_eq!(n, 2);
    }

    #[test]
    fn pack_single_eml_is_not_zipped() {
        let msgs = [Rfc822Message {
            filename: "Hello.eml".into(),
            bytes: EML.to_vec(),
        }];
        let (name, mime, bytes) = pack_export(&msgs, MailExportFormat::EmlZip).unwrap();
        assert_eq!(name, "Hello.eml");
        assert_eq!(mime, "message/rfc822");
        assert_eq!(bytes, EML);
    }

    #[test]
    fn pack_multi_eml_is_zip_named_after_folder() {
        let msgs = [
            Rfc822Message {
                filename: "A.eml".into(),
                bytes: EML.to_vec(),
            },
            Rfc822Message {
                filename: "B.eml".into(),
                bytes: EML.to_vec(),
            },
        ];
        let (name, mime, bytes) =
            pack_export_named(&msgs, MailExportFormat::EmlZip, "INBOX").unwrap();
        assert_eq!(name, "INBOX-messages.zip");
        assert_eq!(mime, "application/zip");
        assert_eq!(&bytes[0..2], b"PK");
    }

    #[test]
    fn import_cap() {
        let files: Vec<_> = (0..MAX_IMPORT_MESSAGES + 1)
            .map(|i| (format!("{i}.eml"), EML.to_vec()))
            .collect();
        let err = parse_import_files(files).unwrap_err();
        assert!(err.contains("limited"));
    }

    #[test]
    fn unknown_extension_sniffs_mbox() {
        let raw = b"From a@b.com Fri Sep  4 00:00:00 2026\nFrom: a@b.com\nSubject: One\n\nhi\n\nFrom c@d.com Fri Sep  4 00:00:01 2026\nFrom: c@d.com\nSubject: Two\n\nbye\n";
        let msgs = parse_import_file("export", raw).unwrap();
        assert_eq!(msgs.len(), 2);
    }
}
