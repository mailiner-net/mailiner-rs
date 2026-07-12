//! Attachment / rich-part heuristics (TS messageview/parser/utils.ts parity).

use mailiner_core::body::BodyPart;

/// Exact TS parity (`isAttachment`).
///
/// Only disposition `FILENAME` is considered for text parts — Content-Type `NAME`
/// alone does **not** make a text part an attachment.
pub fn is_attachment(part: &BodyPart) -> bool {
    if let Some(d) = &part.disposition {
        let t = d.type_.to_ascii_uppercase();
        if t == "ATTACHMENT" {
            return true;
        }
        if t == "INLINE" {
            return false;
        }
    }
    if part.type_ == "multipart" {
        return false;
    }
    if part.type_ == "text" && part.disposition_filename().is_none() {
        return false;
    }
    true
}

pub fn is_rich_part(part: &BodyPart) -> bool {
    part.type_ == "text" && part.subtype != "plain" && part.subtype != "calendar"
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::body::ContentDisposition;
    use std::collections::BTreeMap;

    fn text_plain() -> BodyPart {
        BodyPart {
            type_: "text".into(),
            subtype: "plain".into(),
            ..Default::default()
        }
    }

    #[test]
    fn disposition_attachment() {
        let mut p = text_plain();
        p.disposition = Some(ContentDisposition {
            type_: "attachment".into(),
            attributes: BTreeMap::new(),
        });
        assert!(is_attachment(&p));
    }

    #[test]
    fn disposition_inline() {
        let mut p = text_plain();
        p.disposition = Some(ContentDisposition {
            type_: "INLINE".into(),
            attributes: BTreeMap::new(),
        });
        assert!(!is_attachment(&p));
    }

    #[test]
    fn text_with_name_only_not_attachment() {
        let mut p = text_plain();
        p.parameters.insert("NAME".into(), "readme.txt".into());
        assert!(
            !is_attachment(&p),
            "NAME alone must not force attachment (TS parity)"
        );
    }

    #[test]
    fn text_with_disposition_filename_is_attachment() {
        let mut p = text_plain();
        p.disposition = Some(ContentDisposition {
            type_: "attachment".into(),
            attributes: [("FILENAME".into(), "readme.txt".into())].into(),
        });
        assert!(is_attachment(&p));
    }

    #[test]
    fn application_pdf_without_disposition_is_attachment() {
        let p = BodyPart {
            type_: "application".into(),
            subtype: "pdf".into(),
            ..Default::default()
        };
        assert!(is_attachment(&p));
    }

    #[test]
    fn multipart_never_attachment() {
        let p = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            ..Default::default()
        };
        assert!(!is_attachment(&p));
    }

    #[test]
    fn rich_html() {
        let p = BodyPart {
            type_: "text".into(),
            subtype: "html".into(),
            ..Default::default()
        };
        assert!(is_rich_part(&p));
        assert!(!is_rich_part(&text_plain()));
    }
}
