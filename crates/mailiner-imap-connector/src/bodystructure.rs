use std::fmt;
use thiserror::Error;

use mailiner_core::{MessageContent, MessageId, MessagePart, MessagePartId, MessageStructure};

#[derive(Error, Debug)]
pub enum BodystructureError {
    #[error("Invalid bodystructure format: {0}")]
    InvalidFormat(String),
    #[error("Missing required field: {0}")]
    MissingField(String),
    #[error("Invalid content type: {0}")]
    InvalidContentType(String),
    #[error("Invalid encoding: {0}")]
    InvalidEncoding(String),
    #[error("Invalid disposition: {0}")]
    InvalidDisposition(String),
}

#[derive(Debug, Clone)]
pub struct BodystructurePart {
    pub content_type: String,
    pub content_subtype: String,
    pub parameters: Vec<(String, String)>,
    pub id: Option<String>,
    pub description: Option<String>,
    pub encoding: Option<String>,
    pub size: Option<usize>,
    pub lines: Option<usize>,
    pub parts: Vec<BodystructurePart>,
}

impl BodystructurePart {
    pub fn is_multipart(&self) -> bool {
        self.content_type.to_lowercase() == "multipart"
    }

    pub fn is_attachment(&self) -> bool {
        if let Some(disposition) = self.parameters.iter().find(|(k, _)| k == "disposition") {
            disposition.1.to_lowercase() == "attachment"
        } else {
            false
        }
    }

    pub fn get_filename(&self) -> Option<String> {
        if let Some(filename) = self.parameters.iter().find(|(k, _)| k == "filename") {
            Some(filename.1.clone())
        } else if let Some(name) = self.parameters.iter().find(|(k, _)| k == "name") {
            Some(name.1.clone())
        } else {
            None
        }
    }

    pub fn to_message_part(&self, message_id: &MessageId, part_number: &str) -> MessagePart {
        let id = MessagePartId::new(format!("{}-{}", message_id.as_str(), part_number));
        let content_type = format!("{}/{}", self.content_type, self.content_subtype);
        let filename = self.get_filename();
        let is_attachment = self.is_attachment();
        let size = self.size.unwrap_or(0) as u64;

        MessagePart {
            id,
            envelope_id: message_id.clone(),
            content_type,
            filename,
            size,
            is_attachment,
            content: MessageContent::Text(String::new()), // Content will be fetched on demand
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    pub fn to_message_structure(&self, message_id: &MessageId) -> MessageStructure {
        if self.is_multipart() {
            let mut parts = Vec::new();
            for (i, part) in self.parts.iter().enumerate() {
                let part_number = format!("{}", i + 1);
                parts.push(part.to_message_part(message_id, &part_number));
            }
            MessageStructure::Multipart(parts)
        } else {
            MessageStructure::Simple(self.to_message_part(message_id, "1").id)
        }
    }
}

impl fmt::Display for BodystructurePart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({} {})", self.content_type, self.content_subtype)?;
        if !self.parameters.is_empty() {
            write!(f, " (")?;
            for (i, (k, v)) in self.parameters.iter().enumerate() {
                if i > 0 {
                    write!(f, " ")?;
                }
                write!(f, "{} {}", k, v)?;
            }
            write!(f, ")")?;
        }
        if let Some(id) = &self.id {
            write!(f, " ID {}", id)?;
        }
        if let Some(desc) = &self.description {
            write!(f, " DESCRIPTION {}", desc)?;
        }
        if let Some(enc) = &self.encoding {
            write!(f, " ENCODING {}", enc)?;
        }
        if let Some(size) = self.size {
            write!(f, " SIZE {}", size)?;
        }
        if let Some(lines) = self.lines {
            write!(f, " LINES {}", lines)?;
        }
        if !self.parts.is_empty() {
            write!(f, " (")?;
            for (i, part) in self.parts.iter().enumerate() {
                if i > 0 {
                    write!(f, " ")?;
                }
                write!(f, "{}", part)?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn expect(&mut self, c: char) -> Result<(), BodystructureError> {
        if self.next() != Some(c) {
            return Err(BodystructureError::InvalidFormat(format!(
                "Expected '{}' at position {}",
                c, self.pos
            )));
        }
        Ok(())
    }

    fn parse_string(&mut self) -> Result<String, BodystructureError> {
        match self.peek() {
            Some('"') => {
                self.next(); // Skip opening quote
                let mut s = String::new();
                while let Some(c) = self.peek() {
                    match c {
                        '"' => {
                            self.next(); // Skip closing quote
                            return Ok(s);
                        }
                        '\\' => {
                            self.next(); // Skip backslash
                            if let Some(c) = self.next() {
                                s.push(c);
                            }
                        }
                        _ => {
                            s.push(c);
                            self.next();
                        }
                    }
                }
                Err(BodystructureError::InvalidFormat("Unterminated string".to_string()))
            }
            Some('{') => {
                self.next(); // Skip opening brace
                let mut size = String::new();
                while let Some(c) = self.peek() {
                    if c == '}' {
                        self.next(); // Skip closing brace
                        break;
                    }
                    if c.is_ascii_digit() {
                        size.push(c);
                        self.next();
                    } else {
                        return Err(BodystructureError::InvalidFormat(
                            "Invalid literal size".to_string(),
                        ));
                    }
                }
                let size: usize = size.parse().map_err(|_| {
                    BodystructureError::InvalidFormat("Invalid literal size".to_string())
                })?;
                self.expect('\r')?;
                self.expect('\n')?;
                let s = &self.input[self.pos..self.pos + size];
                self.pos += size;
                Ok(s.to_string())
            }
            Some('N') => {
                // Check for NIL
                if self.input[self.pos..].starts_with("NIL") {
                    self.pos += 3;
                    Ok(String::new())
                } else {
                    Err(BodystructureError::InvalidFormat("Expected NIL".to_string()))
                }
            }
            _ => Err(BodystructureError::InvalidFormat("Expected string".to_string())),
        }
    }

    fn parse_list(&mut self) -> Result<Vec<String>, BodystructureError> {
        self.expect('(')?;
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                ')' => {
                    self.next();
                    return Ok(items);
                }
                ' ' => {
                    self.next();
                }
                _ => {
                    items.push(self.parse_string()?);
                }
            }
        }
        Err(BodystructureError::InvalidFormat("Unterminated list".to_string()))
    }

    fn parse_parameters(&mut self) -> Result<Vec<(String, String)>, BodystructureError> {
        let mut params = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                ' ' => {
                    self.next();
                }
                '(' => {
                    let items = self.parse_list()?;
                    if items.len() % 2 != 0 {
                        return Err(BodystructureError::InvalidFormat(
                            "Invalid parameter list".to_string(),
                        ));
                    }
                    for i in (0..items.len()).step_by(2) {
                        params.push((items[i].clone(), items[i + 1].clone()));
                    }
                }
                _ => break,
            }
        }
        Ok(params)
    }

    fn parse_part(&mut self) -> Result<BodystructurePart, BodystructureError> {
        self.expect('(')?;
        let content_type = self.parse_string()?;
        self.expect(' ')?;
        let content_subtype = self.parse_string()?;

        let mut part = BodystructurePart {
            content_type,
            content_subtype,
            parameters: Vec::new(),
            id: None,
            description: None,
            encoding: None,
            size: None,
            lines: None,
            parts: Vec::new(),
        };

        // Parse parameters
        part.parameters = self.parse_parameters()?;

        // Parse optional fields
        while let Some(c) = self.peek() {
            match c {
                ' ' => {
                    self.next();
                }
                ')' => {
                    self.next();
                    return Ok(part);
                }
                _ => {
                    let field = self.parse_string()?;
                    self.expect(' ')?;
                    match field.to_uppercase().as_str() {
                        "ID" => part.id = Some(self.parse_string()?),
                        "DESCRIPTION" => part.description = Some(self.parse_string()?),
                        "ENCODING" => part.encoding = Some(self.parse_string()?),
                        "SIZE" => {
                            let size = self.parse_string()?;
                            part.size = Some(size.parse().map_err(|_| {
                                BodystructureError::InvalidFormat("Invalid size".to_string())
                            })?);
                        }
                        "LINES" => {
                            let lines = self.parse_string()?;
                            part.lines = Some(lines.parse().map_err(|_| {
                                BodystructureError::InvalidFormat("Invalid lines".to_string())
                            })?);
                        }
                        _ => return Err(BodystructureError::InvalidFormat(format!(
                            "Unknown field: {}",
                            field
                        ))),
                    }
                }
            }
        }

        Err(BodystructureError::InvalidFormat("Unterminated part".to_string()))
    }

    fn parse(&mut self) -> Result<BodystructurePart, BodystructureError> {
        let mut part = self.parse_part()?;

        // If this is a multipart message, parse the parts
        if part.is_multipart() {
            while let Some(c) = self.peek() {
                match c {
                    ' ' => {
                        self.next();
                    }
                    '(' => {
                        part.parts.push(self.parse_part()?);
                    }
                    ')' => {
                        self.next();
                        return Ok(part);
                    }
                    _ => break,
                }
            }
        }

        Ok(part)
    }
}

pub fn parse_bodystructure(input: &str) -> Result<BodystructurePart, BodystructureError> {
    let mut parser = Parser::new(input);
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_text() {
        let input = r#"("text" "plain" NIL NIL NIL "7BIT" 123 4)"#;
        let part = parse_bodystructure(input).unwrap();
        assert_eq!(part.content_type, "text");
        assert_eq!(part.content_subtype, "plain");
        assert_eq!(part.size, Some(123));
        assert_eq!(part.lines, Some(4));
        assert_eq!(part.encoding, Some("7BIT".to_string()));
        assert!(part.parts.is_empty());
    }

    #[test]
    fn test_multipart() {
        let input = r#"("multipart" "mixed" (("boundary" "----=_Part_123_456.789")) NIL NIL NIL 123 4 (("text" "plain" NIL NIL NIL "7BIT" 45 2) ("application" "pdf" ("name" "document.pdf" "disposition" "attachment") NIL NIL "BASE64" 678 NIL)))"#;
        let part = parse_bodystructure(input).unwrap();
        assert_eq!(part.content_type, "multipart");
        assert_eq!(part.content_subtype, "mixed");
        assert_eq!(part.size, Some(123));
        assert_eq!(part.lines, Some(4));
        assert_eq!(part.parts.len(), 2);

        let text_part = &part.parts[0];
        assert_eq!(text_part.content_type, "text");
        assert_eq!(text_part.content_subtype, "plain");
        assert_eq!(text_part.size, Some(45));
        assert_eq!(text_part.lines, Some(2));
        assert_eq!(text_part.encoding, Some("7BIT".to_string()));

        let pdf_part = &part.parts[1];
        assert_eq!(pdf_part.content_type, "application");
        assert_eq!(pdf_part.content_subtype, "pdf");
        assert_eq!(pdf_part.size, Some(678));
        assert_eq!(pdf_part.encoding, Some("BASE64".to_string()));
        assert!(pdf_part.is_attachment());
        assert_eq!(pdf_part.get_filename(), Some("document.pdf".to_string()));
    }

    #[test]
    fn test_literal_string() {
        let input = r#"("text" "plain" ("charset" "UTF-8") "message-id@example.com" "Subject" "7BIT" 123 4)"#;
        let part = parse_bodystructure(input).unwrap();
        assert_eq!(part.content_type, "text");
        assert_eq!(part.content_subtype, "plain");
        assert_eq!(part.id, Some("message-id@example.com".to_string()));
        assert_eq!(part.description, Some("Subject".to_string()));
        assert_eq!(part.encoding, Some("7BIT".to_string()));
        assert_eq!(part.size, Some(123));
        assert_eq!(part.lines, Some(4));
    }

    #[test]
    fn test_quoted_string() {
        let input = r#"("text" "plain" ("charset" "UTF-8" "name" "Hello World.pdf") NIL NIL "7BIT" 123 4)"#;
        let part = parse_bodystructure(input).unwrap();
        assert_eq!(part.content_type, "text");
        assert_eq!(part.content_subtype, "plain");
        assert_eq!(part.get_filename(), Some("Hello World.pdf".to_string()));
    }

    #[test]
    fn test_escaped_quotes() {
        let input = r#"("text" "plain" ("name" "Hello \"World\".pdf") NIL NIL "7BIT" 123 4)"#;
        let part = parse_bodystructure(input).unwrap();
        assert_eq!(part.get_filename(), Some("Hello \"World\".pdf".to_string()));
    }
} 