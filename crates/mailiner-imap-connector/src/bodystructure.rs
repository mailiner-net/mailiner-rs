use std::collections::HashMap;
use std::fmt;

use imap_proto::types::{BodyStructure, BodyContentCommon, BodyContentSinglePart, ContentType, ContentDisposition, ContentEncoding};
use thiserror::Error;

use mailiner_core::{MessageContent, MessageId, MessagePart, MessagePartId, MessageStructure};
use chrono::Utc;

#[derive(Error, Debug)]
pub enum BodystructureError {
    #[error("Invalid BODYSTRUCTURE: {0}")]
    InvalidData(String),
}

#[derive(Debug)]
pub struct BodystructurePart {
    pub content_type: String,
    pub content_subtype: String,
    pub parameters: HashMap<String, String>,
    pub id: Option<String>,
    pub description: Option<String>,
    pub encoding: Option<String>,
    pub size: Option<u32>,
    pub lines: Option<u32>,
    pub parts: Vec<BodystructurePart>,
}

impl BodystructurePart {
    pub fn is_multipart(&self) -> bool {
        self.content_type == "multipart"
    }

    pub fn is_attachment(&self) -> bool {
        if let Some(disposition) = self.parameters.get("disposition") {
            disposition == "attachment"
        } else {
            false
        }
    }

    pub fn get_filename(&self) -> Option<String> {
        if let Some(filename) = self.parameters.get("filename") {
            Some(filename.clone())
        } else if let Some(name) = self.parameters.get("name") {
            Some(name.clone())
        } else {
            None
        }
    }

    pub fn to_message_part(&self, message_id: &MessageId, part_number: &str) -> MessagePart {
        MessagePart {
            id: MessagePartId::new(format!("{}-{}", message_id.as_str(), part_number)),
            envelope_id: message_id.clone(),
            content_type: format!("{}/{}", self.content_type, self.content_subtype),
            filename: self.get_filename(),
            size: self.size.unwrap_or(0) as u64,
            is_attachment: self.is_attachment(),
            content: MessageContent::Text(String::new()), // Content will be fetched separately
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn to_message_structure(&self, message_id: &MessageId) -> MessageStructure {
        if self.is_multipart() {
            MessageStructure::Multipart {
                parts: self.parts
                    .iter()
                    .enumerate()
                    .map(|(i, part)| part.to_message_part(message_id, &(i + 1).to_string()))
                    .collect(),
                boundary: self.parameters.get("boundary").unwrap_or(&String::new()).clone(),
            }
        } else {
            MessageStructure::Simple(self.to_message_part(message_id, "1").id)
        }
    }
}

impl TryFrom<&BodyStructure<'_>> for BodystructurePart {
    type Error = BodystructureError;

    fn try_from(bs: &BodyStructure<'_>) -> Result<Self, Self::Error> {
        match bs {
            BodyStructure::Basic { common, other, .. } => {
                let mut parameters = HashMap::new();
                for (key, value) in common.ty.params.iter() {
                    parameters.insert(key.to_string(), value.to_string());
                }

                Ok(BodystructurePart {
                    content_type: common.ty.ty.to_string(),
                    content_subtype: common.ty.subtype.to_string(),
                    parameters,
                    id: other.id.as_ref().map(|s| s.to_string()),
                    description: other.description.as_ref().map(|s| s.to_string()),
                    encoding: Some(match &other.transfer_encoding {
                        ContentEncoding::SevenBit => "7BIT".to_string(),
                        ContentEncoding::EightBit => "8BIT".to_string(),
                        ContentEncoding::Binary => "BINARY".to_string(),
                        ContentEncoding::Base64 => "BASE64".to_string(),
                        ContentEncoding::QuotedPrintable => "QUOTED-PRINTABLE".to_string(),
                        ContentEncoding::Other(s) => s.to_string(),
                    }),
                    size: Some(other.octets),
                    lines: None,
                    parts: Vec::new(),
                })
            }
            BodyStructure::Text { common, other, lines, .. } => {
                let mut parameters = HashMap::new();
                for (key, value) in common.ty.params.iter() {
                    parameters.insert(key.to_string(), value.to_string());
                }

                Ok(BodystructurePart {
                    content_type: common.ty.ty.to_string(),
                    content_subtype: common.ty.subtype.to_string(),
                    parameters,
                    id: other.id.as_ref().map(|s| s.to_string()),
                    description: other.description.as_ref().map(|s| s.to_string()),
                    encoding: Some(match &other.transfer_encoding {
                        ContentEncoding::SevenBit => "7BIT".to_string(),
                        ContentEncoding::EightBit => "8BIT".to_string(),
                        ContentEncoding::Binary => "BINARY".to_string(),
                        ContentEncoding::Base64 => "BASE64".to_string(),
                        ContentEncoding::QuotedPrintable => "QUOTED-PRINTABLE".to_string(),
                        ContentEncoding::Other(s) => s.to_string(),
                    }),
                    size: Some(other.octets),
                    lines: Some(*lines),
                    parts: Vec::new(),
                })
            }
            BodyStructure::Message { common, other, body, lines, .. } => {
                let mut parameters = HashMap::new();
                for (key, value) in common.ty.params.iter() {
                    parameters.insert(key.to_string(), value.to_string());
                }

                Ok(BodystructurePart {
                    content_type: common.ty.ty.to_string(),
                    content_subtype: common.ty.subtype.to_string(),
                    parameters,
                    id: other.id.as_ref().map(|s| s.to_string()),
                    description: other.description.as_ref().map(|s| s.to_string()),
                    encoding: Some(match &other.transfer_encoding {
                        ContentEncoding::SevenBit => "7BIT".to_string(),
                        ContentEncoding::EightBit => "8BIT".to_string(),
                        ContentEncoding::Binary => "BINARY".to_string(),
                        ContentEncoding::Base64 => "BASE64".to_string(),
                        ContentEncoding::QuotedPrintable => "QUOTED-PRINTABLE".to_string(),
                        ContentEncoding::Other(s) => s.to_string(),
                    }),
                    size: Some(other.octets),
                    lines: Some(*lines),
                    parts: vec![BodystructurePart::try_from(body.as_ref())?],
                })
            }
            BodyStructure::Multipart { common, bodies, .. } => {
                let mut parameters = HashMap::new();
                for (key, value) in common.ty.params.iter() {
                    parameters.insert(key.to_string(), value.to_string());
                }

                Ok(BodystructurePart {
                    content_type: common.ty.ty.to_string(),
                    content_subtype: common.ty.subtype.to_string(),
                    parameters,
                    id: None,
                    description: None,
                    encoding: None,
                    size: None,
                    lines: None,
                    parts: bodies.iter().map(BodystructurePart::try_from).collect::<Result<Vec<_>, _>>()?,
                })
            }
        }
    }
}

pub fn parse_bodystructure(bs: &BodyStructure<'_>) -> Result<BodystructurePart, BodystructureError> {
    BodystructurePart::try_from(bs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn test_parse_basic_text() {
        let bs = BodyStructure::Text {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: Cow::from("text"),
                    subtype: Cow::from("plain"),
                    params: Some(vec![(Cow::from("charset"), Cow::from("utf-8"))]),
                },
                disposition: None,
                language: None,
                location: None,
            },
            other: BodyContentSinglePart {
                id: Some(Cow::from("123@example.com")),
                description: Some(Cow::from("Test message")),
                transfer_encoding: ContentEncoding::SevenBit,
                octets: 100,
                md5: None,
            },
            lines: 10,
            extension: None,
        };

        let part = parse_bodystructure(&bs).unwrap();
        assert_eq!(part.content_type, "text");
        assert_eq!(part.content_subtype, "plain");
        assert_eq!(part.parameters.get("charset").unwrap(), "utf-8");
        assert_eq!(part.id.unwrap(), "123@example.com");
        assert_eq!(part.description.unwrap(), "Test message");
        assert_eq!(part.encoding.unwrap(), "7BIT");
        assert_eq!(part.size.unwrap(), 100);
        assert_eq!(part.lines.unwrap(), 10);
    }

    #[test]
    fn test_parse_multipart() {
        let bs = BodyStructure::Multipart {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: Cow::from("multipart"),
                    subtype: Cow::from("mixed"),
                    params: Some(vec![(Cow::from("boundary"), Cow::from("boundary123"))]),
                },
                disposition: None,
                language: None,
                location: None,
            },
            bodies: vec![
                BodyStructure::Text {
                    common: BodyContentCommon {
                        ty: ContentType {
                            ty: Cow::from("text"),
                            subtype: Cow::from("plain"),
                            params: Some(vec![(Cow::from("charset"), Cow::from("utf-8"))]),
                        },
                        disposition: None,
                        language: None,
                        location: None,
                    },
                    other: BodyContentSinglePart {
                        id: None,
                        description: None,
                        transfer_encoding: ContentEncoding::SevenBit,
                        octets: 100,
                        md5: None,
                    },
                    lines: 10,
                    extension: None,
                },
                BodyStructure::Basic {
                    common: BodyContentCommon {
                        ty: ContentType {
                            ty: Cow::from("application"),
                            subtype: Cow::from("pdf"),
                            params: Some(vec![(Cow::from("name"), Cow::from("document.pdf"))]),
                        },
                        disposition: Some(ContentDisposition {
                            ty: Cow::from("attachment"),
                            params: Some(vec![]),
                        }),
                        language: None,
                        location: None,
                    },
                    other: BodyContentSinglePart {
                        id: None,
                        description: None,
                        transfer_encoding: ContentEncoding::Base64,
                        octets: 200,
                        md5: None,
                    },
                    extension: None,
                },
            ],
            extension: None,
        };

        let part = parse_bodystructure(&bs).unwrap();
        assert_eq!(part.content_type, "multipart");
        assert_eq!(part.content_subtype, "mixed");
        assert_eq!(part.parameters.get("boundary").unwrap(), "boundary123");
        assert_eq!(part.parts.len(), 2);

        let text_part = &part.parts[0];
        assert_eq!(text_part.content_type, "text");
        assert_eq!(text_part.content_subtype, "plain");
        assert_eq!(text_part.parameters.get("charset").unwrap(), "utf-8");

        let attachment = &part.parts[1];
        assert_eq!(attachment.content_type, "application");
        assert_eq!(attachment.content_subtype, "pdf");
        assert_eq!(attachment.parameters.get("name").unwrap(), "document.pdf");
        assert!(attachment.is_attachment());
    }
} 