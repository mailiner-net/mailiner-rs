use thiserror::Error;

use crate::ids::MessageId;

#[derive(Error, Debug)]
pub enum MailinerError {
    #[error("Connector error: {0}")]
    Connector(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Not found: {0}")]
    NotFound(String),

    /// COPY (or equivalent) succeeded; removing the source UIDs failed.
    #[error("Copied to the destination but could not remove the originals: {message}")]
    PartialMove {
        message: String,
        dest_ids: Vec<MessageId>,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, MailinerError>;
