use mailiner_core::body::BodyPart;
use mailiner_core::models::{MessagePart, PartKind};

use super::{leaf_part, PartParser, ParseContext};

pub struct ImageParser;

impl PartParser for ImageParser {
    fn mime_types(&self) -> &[&str] {
        &["image/*"]
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
            &format!("{part_id}.image"),
            path,
            PartKind::Image,
            Some(true),
            None, // leaf_part applies cid-hidden rule for Image
        )]
    }
}
