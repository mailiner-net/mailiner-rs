use mailiner_core::body::BodyPart;
use mailiner_core::models::MessagePart;

use super::{ParseContext, PartParser, ATTACHMENT_MIME};
use crate::heuristics::{is_attachment, is_rich_part};

pub struct MultipartAlternativeParser;
pub struct MultipartMixedParser;
pub struct MultipartRelatedParser;

impl PartParser for MultipartAlternativeParser {
    fn mime_types(&self) -> &[&str] {
        &["multipart/alternative"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        // Keep every parseable alternative so the viewer can toggle HTML / plain.
        // Default rendering still prefers HTML via FormatOptions.prefer_plain.
        let mut out = Vec::new();
        for (i, sub) in part.subparts.iter().enumerate() {
            let ct = sub.content_type();
            if !ctx.registry.can_parse(&ct) {
                continue;
            }
            let mut sub_path = path.to_vec();
            sub_path.push((i + 1).to_string());
            let sub_id = format!("{part_id}.alternative.{i}");
            out.extend(
                ctx.registry
                    .parse_part(ctx.envelope_id, sub, &sub_id, &sub_path),
            );
        }
        out
    }
}

impl PartParser for MultipartMixedParser {
    fn mime_types(&self) -> &[&str] {
        // Unknown multipart/* treated as mixed (registered after related/alternative exact).
        &["multipart/mixed", "multipart/*"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        let mut out = Vec::new();
        for (i, sub) in part.subparts.iter().enumerate() {
            let mut sub_path = path.to_vec();
            sub_path.push((i + 1).to_string());
            let sub_id = format!("{part_id}.mixed.{i}");
            if sub.is_rfc822() {
                out.extend(
                    ctx.registry
                        .parse_part(ctx.envelope_id, sub, &sub_id, &sub_path),
                );
            } else if is_attachment(sub) {
                out.extend(ctx.registry.parse_as(
                    ctx.envelope_id,
                    sub,
                    ATTACHMENT_MIME,
                    &sub_id,
                    &sub_path,
                ));
            } else {
                out.extend(
                    ctx.registry
                        .parse_part(ctx.envelope_id, sub, &sub_id, &sub_path),
                );
            }
        }
        out
    }
}

impl PartParser for MultipartRelatedParser {
    fn mime_types(&self) -> &[&str] {
        &["multipart/related"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        let has_rich = part.subparts.iter().any(is_rich_part);
        let mut out = Vec::new();
        for (i, sub) in part.subparts.iter().enumerate() {
            let mut sub_path = path.to_vec();
            sub_path.push((i + 1).to_string());
            let sub_id = format!("{part_id}.related.{i}");
            // Attachment only if is_attachment && (!cid || !has_rich).
            // message/rfc822 is always parsed so the nested body can be opened.
            let force_att =
                !sub.is_rfc822() && is_attachment(sub) && (sub.id.is_none() || !has_rich);
            if force_att {
                out.extend(ctx.registry.parse_as(
                    ctx.envelope_id,
                    sub,
                    ATTACHMENT_MIME,
                    &sub_id,
                    &sub_path,
                ));
            } else {
                out.extend(
                    ctx.registry
                        .parse_part(ctx.envelope_id, sub, &sub_id, &sub_path),
                );
            }
        }
        out
    }
}
