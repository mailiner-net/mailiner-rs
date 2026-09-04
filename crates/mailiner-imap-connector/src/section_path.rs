//! Parse IMAP section path strings into `imap_proto::SectionPath`.

use imap_proto::types::{MessageSection, SectionPath};

use crate::ImapError;

/// Parse a section string used in `BODY.PEEK[section]` into a [`SectionPath`].
///
/// Supported:
/// - `TEXT`, `HEADER`, `MIME` (full message sections)
/// - `1`, `1.2`, `1.2.3` (part numbers)
/// - `1.2.MIME`, `1.HEADER`, `1.TEXT` (part + message section)
pub fn parse_section_path(section: &str) -> Result<SectionPath, ImapError> {
    let s = section.trim();
    if s.is_empty() {
        return Err(ImapError::InvalidData("empty section path".into()));
    }

    let upper = s.to_ascii_uppercase();
    match upper.as_str() {
        "TEXT" => return Ok(SectionPath::Full(MessageSection::Text)),
        "HEADER" => return Ok(SectionPath::Full(MessageSection::Header)),
        "MIME" => return Ok(SectionPath::Full(MessageSection::Mime)),
        _ => {}
    }

    let parts: Vec<&str> = s.split('.').collect();
    let mut nums: Vec<u32> = Vec::new();
    let mut msg_section: Option<MessageSection> = None;

    for (i, p) in parts.iter().enumerate() {
        let pu = p.to_ascii_uppercase();
        match pu.as_str() {
            "TEXT" | "HEADER" | "MIME" => {
                if i != parts.len() - 1 {
                    return Err(ImapError::InvalidData(format!(
                        "section keyword {p} not at end of path {section}"
                    )));
                }
                msg_section = Some(match pu.as_str() {
                    "TEXT" => MessageSection::Text,
                    "HEADER" => MessageSection::Header,
                    "MIME" => MessageSection::Mime,
                    _ => unreachable!(),
                });
            }
            _ => {
                let n: u32 = p.parse().map_err(|_| {
                    ImapError::InvalidData(format!("invalid section path segment: {p}"))
                })?;
                if n == 0 {
                    return Err(ImapError::InvalidData(
                        "section path numbers are 1-based".into(),
                    ));
                }
                nums.push(n);
            }
        }
    }

    if nums.is_empty() {
        return Err(ImapError::InvalidData(format!(
            "invalid section path: {section}"
        )));
    }

    Ok(SectionPath::Part(nums, msg_section))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_full() {
        assert_eq!(
            parse_section_path("TEXT").unwrap(),
            SectionPath::Full(MessageSection::Text)
        );
        assert_eq!(
            parse_section_path("text").unwrap(),
            SectionPath::Full(MessageSection::Text)
        );
    }

    #[test]
    fn header_full() {
        assert_eq!(
            parse_section_path("HEADER").unwrap(),
            SectionPath::Full(MessageSection::Header)
        );
        assert_eq!(
            parse_section_path("header").unwrap(),
            SectionPath::Full(MessageSection::Header)
        );
    }

    #[test]
    fn single_part() {
        assert_eq!(
            parse_section_path("1").unwrap(),
            SectionPath::Part(vec![1], None)
        );
    }

    #[test]
    fn nested() {
        assert_eq!(
            parse_section_path("1.2").unwrap(),
            SectionPath::Part(vec![1, 2], None)
        );
        assert_eq!(
            parse_section_path("1.2.3").unwrap(),
            SectionPath::Part(vec![1, 2, 3], None)
        );
    }

    #[test]
    fn part_mime() {
        assert_eq!(
            parse_section_path("1.2.MIME").unwrap(),
            SectionPath::Part(vec![1, 2], Some(MessageSection::Mime))
        );
    }

    #[test]
    fn rejects_zero() {
        assert!(parse_section_path("0").is_err());
    }
}
