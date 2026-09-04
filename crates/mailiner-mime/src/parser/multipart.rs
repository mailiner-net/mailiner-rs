use mailiner_core::body::BodyPart;
use mailiner_core::models::MessagePart;

use super::{ParseContext, PartParser, ATTACHMENT_MIME};
use crate::heuristics::{is_attachment, is_rich_part};

pub struct MultipartAlternativeParser;
pub struct MultipartMixedParser;
pub struct MultipartRelatedParser;
pub struct MultipartEncryptedParser;
pub struct MultipartSignedParser;

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
            } else if sub.is_calendar() {
                out.extend(ctx.registry.parse_as(
                    ctx.envelope_id,
                    sub,
                    "text/calendar",
                    &sub_id,
                    &sub_path,
                ));
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

impl PartParser for MultipartEncryptedParser {
    fn mime_types(&self) -> &[&str] {
        &["multipart/encrypted"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        // RFC 3156: version part + ciphertext. Hide both; hidden parts are prefetched.
        parse_pgp_container(ctx, part, part_id, path, "encrypted")
    }
}

impl PartParser for MultipartSignedParser {
    fn mime_types(&self) -> &[&str] {
        &["multipart/signed"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        // RFC 3156: signed body + detached signature. Hide only the signature.
        let protocol = part
            .parameters
            .get("PROTOCOL")
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if protocol.contains("pkcs7") {
            return MultipartMixedParser.parse(ctx, part, part_id, path);
        }
        let mut out = Vec::new();
        for (i, sub) in part.subparts.iter().enumerate() {
            let mut sub_path = path.to_vec();
            sub_path.push((i + 1).to_string());
            let sub_id = format!("{part_id}.signed.{i}");
            let ct = sub.content_type();
            if ct.eq_ignore_ascii_case("application/pgp-signature")
                || ct.eq_ignore_ascii_case("application/pgp-encrypted")
            {
                out.extend(hidden_crypto_part(ctx, sub, &sub_id, &sub_path));
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

fn parse_pgp_container(
    ctx: &ParseContext<'_>,
    part: &BodyPart,
    part_id: &str,
    path: &[String],
    label: &str,
) -> Vec<MessagePart> {
    let mut out = Vec::new();
    for (i, sub) in part.subparts.iter().enumerate() {
        let mut sub_path = path.to_vec();
        sub_path.push((i + 1).to_string());
        let sub_id = format!("{part_id}.{label}.{i}");
        out.extend(hidden_crypto_part(ctx, sub, &sub_id, &sub_path));
    }
    out
}

fn hidden_crypto_part(
    ctx: &ParseContext<'_>,
    part: &BodyPart,
    part_id: &str,
    path: &[String],
) -> Vec<MessagePart> {
    use super::leaf_part;
    use mailiner_core::models::PartKind;
    vec![leaf_part(
        ctx.envelope_id,
        part,
        part_id,
        path,
        PartKind::Attachment,
        Some(true),
        Some(true),
    )]
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
            let force_att = !sub.is_rfc822()
                && !sub.is_calendar()
                && is_attachment(sub)
                && (sub.id.is_none() || !has_rich);
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
