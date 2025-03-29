use thiserror::Error;

#[derive(Error, Debug)]
pub enum MailinerError {
    #[error("Storage error: {0}")]
    Storage(String),
    
    #[error("Connector error: {0}")]
    Connector(String),
    
    #[error("Invalid data: {0}")]
    InvalidData(String),
    
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, MailinerError>; 