use mailiner_core::body::BodyPart;
use mailiner_core::models::{MessagePart, PartKind};

use super::{leaf_part, ParseContext, PartParser};

pub struct MessageRfc822Parser;

impl PartParser for MessageRfc822Parser {
    fn mime_types(&self) -> &[&str] {
        &["message/rfc822"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        let is_root = path.is_empty();
        let mut out = Vec::new();

        if !is_root {
            let mut wrapper = leaf_part(
                ctx.envelope_id,
                part,
                &format!("{part_id}.rfc822"),
                path,
                PartKind::Attachment,
                Some(true),
                Some(false),
            );
            wrapper.nested_headers = part.nested_headers.clone();
            if wrapper.filename.is_none() {
                wrapper.filename = rfc822_filename(part);
            }
            out.push(wrapper);
        }

        let Some(nested) = part.nested_message.as_deref() else {
            return out;
        };

        let child_path = nested_child_path(path, nested);
        let child_id = format!("{part_id}.rfc822.body");
        let mut children = ctx
            .registry
            .parse_part(ctx.envelope_id, nested, &child_id, &child_path);

        if !is_root {
            let section = path.join(".");
            for child in &mut children {
                if child.nested_in.is_none() {
                    child.nested_in = Some(section.clone());
                }
            }
        }

        out.extend(children);
        out
    }
}

/// IMAP section for the nested body.
///
/// Multipart children are numbered `N.1`, `N.2`, … from the rfc822 path.
/// A single-part nested message is `N.1` (`BODY[N]` is the whole .eml).
fn nested_child_path(path: &[String], nested: &BodyPart) -> Vec<String> {
    if nested.type_ == "multipart" {
        path.to_vec()
    } else {
        let mut child = path.to_vec();
        child.push("1".into());
        child
    }
}

fn rfc822_filename(part: &BodyPart) -> Option<String> {
    if let Some(name) = part.filename() {
        if !name.trim().is_empty() {
            return Some(name.to_string());
        }
    }
    if let Some(subject) = part
        .nested_headers
        .as_ref()
        .and_then(|h| h.subject.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(format!("{subject}.eml"));
    }
    Some(
        part.description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Forwarded message.eml")
            .to_string(),
    )
}
