use mailiner_core::body::BodyPart;
use mailiner_core::models::{MessagePart, PartKind};

use super::{leaf_part, PartParser, ParseContext, ATTACHMENT_MIME};

pub struct AttachmentParser;

impl PartParser for AttachmentParser {
    fn mime_types(&self) -> &[&str] {
        &[ATTACHMENT_MIME]
    }

    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        let mut mp = leaf_part(
            ctx.envelope_id,
            part,
            &format!("{part_id}.attachment"),
            path,
            PartKind::Attachment,
            Some(true),
            Some(false),
        );
        if mp.filename.is_none() {
            mp.filename = Some(
                part.description
                    .clone()
                    .unwrap_or_else(|| "attachment.dat".into()),
            );
        }
        vec![mp]
    }
}
