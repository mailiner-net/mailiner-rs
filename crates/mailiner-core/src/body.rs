//! IMAP BODYSTRUCTURE-derived part tree (structure only, no payload).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Owned, serializable form of an IMAP BODYSTRUCTURE node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BodyPart {
    /// Lowercased media type, e.g. `"text"`, `"multipart"`.
    pub type_: String,
    /// Lowercased subtype, e.g. `"html"`, `"alternative"`.
    pub subtype: String,
    /// Parameter keys uppercased (`CHARSET`, `NAME`, …).
    pub parameters: BTreeMap<String, String>,
    /// Content-ID, often `"<foo@bar>"`.
    pub id: Option<String>,
    pub description: Option<String>,
    /// Transfer encoding as reported, e.g. `"BASE64"`, `"QUOTED-PRINTABLE"`.
    pub encoding: Option<String>,
    /// Octets as reported by BODYSTRUCTURE.
    pub size: Option<u64>,
    pub md5: Option<String>,
    pub disposition: Option<ContentDisposition>,
    pub location: Option<String>,
    /// Empty for non-multipart.
    pub subparts: Vec<BodyPart>,
    /// Nested structure for `message/rfc822` when present.
    pub nested_message: Option<Box<BodyPart>>,
    /// IMAP ENVELOPE of a nested `message/rfc822` (From/To/Subject/Date).
    #[serde(default)]
    pub nested_headers: Option<crate::models::NestedMessageHeaders>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContentDisposition {
    /// Disposition type as stored (compare with `to_ascii_uppercase()`).
    pub type_: String,
    /// Attribute keys uppercased (`FILENAME`, …).
    pub attributes: BTreeMap<String, String>,
}

impl BodyPart {
    pub fn content_type(&self) -> String {
        format!("{}/{}", self.type_, self.subtype)
    }

    pub fn charset(&self) -> Option<&str> {
        self.parameters.get("CHARSET").map(|s| s.as_str())
    }

    /// Display / download name. Prefers disposition `FILENAME`, then Content-Type `NAME`.
    ///
    /// **Not** used by `is_attachment` (which checks disposition `FILENAME` only).
    pub fn filename(&self) -> Option<&str> {
        self.disposition
            .as_ref()
            .and_then(|d| d.attributes.get("FILENAME"))
            .map(|s| s.as_str())
            .or_else(|| self.parameters.get("NAME").map(|s| s.as_str()))
    }

    /// Disposition `FILENAME` only (TS `isAttachment` parity).
    pub fn disposition_filename(&self) -> Option<&str> {
        self.disposition
            .as_ref()
            .and_then(|d| d.attributes.get("FILENAME"))
            .map(|s| s.as_str())
    }

    pub fn is_rfc822(&self) -> bool {
        self.type_ == "message" && self.subtype == "rfc822"
    }

    /// `text/calendar` / `application/ics`, or a `*.ics` filename.
    pub fn is_calendar(&self) -> bool {
        crate::models::is_calendar_mime(&self.content_type())
            || self
                .filename()
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".ics"))
    }

    /// CMS S/MIME part (`pkcs7-mime` / `pkcs7-signature`).
    pub fn is_smime(&self) -> bool {
        crate::models::is_smime_mime(&self.content_type())
    }
}
