//! Parse a decrypted MIME entity into viewer parts.

use chrono::Utc;
use mailiner_core::ids::{MessageId, MessagePartId};
use mailiner_core::models::{
    primary_mime, MessageContent, MessagePart, PartKind, TransferEncoding,
};
use mailiner_mime::{decode_content, DecodedContent};

/// Turn decrypted octets into display parts (text/plain + text/html).
///
/// If the payload does not look like a MIME entity, it is treated as UTF-8
/// `text/plain`.
pub fn parts_from_decrypted(envelope_id: &MessageId, data: &[u8]) -> Vec<MessagePart> {
    let entities = parse_entities(data);
    if entities.is_empty() {
        return vec![plain_part(envelope_id, 0, bytes_as_text(data))];
    }
    let mut out = Vec::new();
    collect_display(envelope_id, &entities, &mut out, 0);
    if out.is_empty() {
        out.push(plain_part(envelope_id, 0, bytes_as_text(data)));
    }
    out
}

struct Entity {
    content_type: String,
    charset: Option<String>,
    encoding: String,
    body: Vec<u8>,
    children: Vec<Entity>,
}

fn parse_entities(data: &[u8]) -> Vec<Entity> {
    match parse_entity(data) {
        Some(e) => vec![e],
        None => Vec::new(),
    }
}

fn parse_entity(data: &[u8]) -> Option<Entity> {
    let (headers, body) = split_headers_body(data)?;
    if !looks_like_headers(&headers) {
        return None;
    }
    let content_type =
        header_value(&headers, "content-type").unwrap_or_else(|| "text/plain".into());
    let encoding =
        header_value(&headers, "content-transfer-encoding").unwrap_or_else(|| "7bit".into());
    let charset = content_type_param(&content_type, "charset");
    let mime = primary_mime(&content_type).to_ascii_lowercase();
    if mime.starts_with("multipart/") {
        let boundary = content_type_param(&content_type, "boundary")?;
        let children = split_multipart(body, &boundary)
            .into_iter()
            .filter_map(parse_entity)
            .collect();
        return Some(Entity {
            content_type,
            charset,
            encoding,
            body: Vec::new(),
            children,
        });
    }
    Some(Entity {
        content_type,
        charset,
        encoding,
        body: body.to_vec(),
        children: Vec::new(),
    })
}

fn collect_display(
    envelope_id: &MessageId,
    entities: &[Entity],
    out: &mut Vec<MessagePart>,
    mut idx: usize,
) -> usize {
    for ent in entities {
        if !ent.children.is_empty() {
            idx = collect_display(envelope_id, &ent.children, out, idx);
            continue;
        }
        let mime = primary_mime(&ent.content_type).to_ascii_lowercase();
        let decoded = decode_content(
            &ent.body,
            &ent.encoding,
            &ent.content_type,
            ent.charset.as_deref(),
        );
        match (mime.as_str(), decoded) {
            ("text/html", Ok(DecodedContent::Text(t))) => {
                out.push(display_part(
                    envelope_id,
                    idx,
                    PartKind::TextHtml,
                    "text/html",
                    ent.charset.clone(),
                    MessageContent::Text(t),
                ));
                idx += 1;
            }
            ("text/plain", Ok(DecodedContent::Text(t))) => {
                out.push(display_part(
                    envelope_id,
                    idx,
                    PartKind::TextPlain,
                    "text/plain",
                    ent.charset.clone(),
                    MessageContent::Text(t),
                ));
                idx += 1;
            }
            (_, Ok(DecodedContent::Text(t))) if mime.starts_with("text/") => {
                out.push(display_part(
                    envelope_id,
                    idx,
                    PartKind::TextPlain,
                    &ent.content_type,
                    ent.charset.clone(),
                    MessageContent::Text(t),
                ));
                idx += 1;
            }
            _ => {}
        }
    }
    idx
}

fn display_part(
    envelope_id: &MessageId,
    idx: usize,
    kind: PartKind,
    content_type: &str,
    charset: Option<String>,
    content: MessageContent,
) -> MessagePart {
    let now = Utc::now();
    let size = match &content {
        MessageContent::Text(t) => t.len() as u64,
        MessageContent::Binary(b) => b.len() as u64,
        MessageContent::Empty => 0,
    };
    MessagePart {
        id: MessagePartId::new(format!(".pgp.{idx}")),
        envelope_id: envelope_id.clone(),
        path: vec!["PGP".into(), (idx + 1).to_string()],
        kind,
        content_type: content_type.to_string(),
        charset,
        content_id: None,
        description: None,
        filename: None,
        encoding: TransferEncoding::SevenBit,
        original_size: Some(size),
        size,
        is_attachment: false,
        is_hidden: false,
        nested_in: None,
        nested_headers: None,
        content,
        created_at: now,
        updated_at: now,
    }
}

fn plain_part(envelope_id: &MessageId, idx: usize, text: String) -> MessagePart {
    display_part(
        envelope_id,
        idx,
        PartKind::TextPlain,
        "text/plain",
        Some("utf-8".into()),
        MessageContent::Text(text),
    )
}

fn split_headers_body(data: &[u8]) -> Option<(String, &[u8])> {
    let text = std::str::from_utf8(data).ok()?;
    if let Some(i) = text.find("\r\n\r\n") {
        return Some((text[..i].to_string(), &data[i + 4..]));
    }
    if let Some(i) = text.find("\n\n") {
        return Some((text[..i].to_string(), &data[i + 2..]));
    }
    // Header-only entity (empty body).
    if looks_like_headers(text) {
        return Some((text.to_string(), b""));
    }
    None
}

fn looks_like_headers(headers: &str) -> bool {
    let first = headers.lines().next().unwrap_or("").trim();
    first.contains(':') && first.len() < 200
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    header_value_owned(headers, name)
}

fn header_value_owned(headers: &str, name: &str) -> Option<String> {
    let mut current_name = String::new();
    let mut current_value = String::new();
    let mut found = None;
    for line in headers.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with([' ', '\t']) {
            if !current_name.is_empty() {
                current_value.push(' ');
                current_value.push_str(line.trim_start());
            }
            continue;
        }
        if !current_name.is_empty() && current_name.eq_ignore_ascii_case(name) {
            found = Some(std::mem::take(&mut current_value));
        }
        current_name.clear();
        current_value.clear();
        if let Some((n, v)) = line.split_once(':') {
            current_name = n.trim().to_string();
            current_value = v.trim_start().to_string();
        }
    }
    if !current_name.is_empty() && current_name.eq_ignore_ascii_case(name) {
        found = Some(current_value);
    }
    found.filter(|s| !s.is_empty())
}

fn content_type_param(content_type: &str, name: &str) -> Option<String> {
    for param in content_type.split(';').skip(1) {
        let param = param.trim();
        let Some((n, v)) = param.split_once('=') else {
            continue;
        };
        if n.trim().eq_ignore_ascii_case(name) {
            let v = v.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delim = format!("--{boundary}");
    let text = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut parts = Vec::new();
    let mut rest = text;
    // Skip preamble.
    if let Some(i) = rest.find(&delim) {
        rest = &rest[i..];
    }
    loop {
        if rest.starts_with(&format!("{delim}--")) {
            break;
        }
        if !rest.starts_with(&delim) {
            break;
        }
        rest = &rest[delim.len()..];
        if rest.starts_with("\r\n") {
            rest = &rest[2..];
        } else if rest.starts_with('\n') {
            rest = &rest[1..];
        }
        let next = rest.find(&delim).unwrap_or(rest.len());
        let mut chunk = &rest[..next];
        if let Some(stripped) = chunk.strip_suffix("\r\n") {
            chunk = stripped;
        } else if let Some(stripped) = chunk.strip_suffix('\n') {
            chunk = stripped;
        }
        if !chunk.is_empty() {
            // Map back to the original body slice.
            let start = chunk.as_ptr() as usize - body.as_ptr() as usize;
            parts.push(&body[start..start + chunk.len()]);
        }
        rest = &rest[next..];
    }
    parts
}

fn bytes_as_text(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => data.iter().map(|&b| b as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::ids::FolderId;

    #[test]
    fn plain_payload_without_headers() {
        let id = MessageId::new(FolderId::new("INBOX"), "1");
        let parts = parts_from_decrypted(&id, b"hello secret");
        assert_eq!(parts.len(), 1);
        match &parts[0].content {
            MessageContent::Text(t) => assert_eq!(t, "hello secret"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mime_text_plain() {
        let id = MessageId::new(FolderId::new("INBOX"), "1");
        let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nhello mime\r\n";
        let parts = parts_from_decrypted(&id, raw);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::TextPlain);
        match &parts[0].content {
            MessageContent::Text(t) => assert!(t.contains("hello mime")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn multipart_alternative() {
        let id = MessageId::new(FolderId::new("INBOX"), "1");
        let raw = b"Content-Type: multipart/alternative; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nplain\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n\
--b--\r\n";
        let parts = parts_from_decrypted(&id, raw);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().any(|p| p.kind == PartKind::TextPlain));
        assert!(parts.iter().any(|p| p.kind == PartKind::TextHtml));
    }
}
