use thiserror::Error;

pub mod backend;
pub mod cache;
pub mod model;
pub mod repository;
pub mod service;

#[derive(Debug, Error)]
pub enum MailinerError {
    #[error("Backend error: {0}")]
    Backend(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Not found")]
    NotFound,

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Invalid state: {0}")]
    InvalidState(String),
}

pub type Result<T> = std::result::Result<T, MailinerError>;
