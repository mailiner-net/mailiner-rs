use mailiner_core::body::BodyPart;
use mailiner_core::models::{MessagePart, PartKind};

use super::{leaf_part, PartParser, ParseContext};

pub struct TextPlainParser;
pub struct TextHtmlParser;

impl PartParser for TextPlainParser {
    fn mime_types(&self) -> &[&str] {
        &["text/plain"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        vec![leaf_part(
            ctx.envelope_id,
            part,
            &format!("{part_id}.plain_text"),
            path,
            PartKind::TextPlain,
            None,
            None,
        )]
    }
}

impl PartParser for TextHtmlParser {
    fn mime_types(&self) -> &[&str] {
        &["text/html"]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        vec![leaf_part(
            ctx.envelope_id,
            part,
            &format!("{part_id}.html"),
            path,
            PartKind::TextHtml,
            None,
            None,
        )]
    }
}
