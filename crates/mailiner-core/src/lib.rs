pub mod body;
pub mod connector;
pub mod error;
pub mod ids;
pub mod models;
pub mod storage;

pub use body::{BodyPart, ContentDisposition};
pub use connector::{
    mock_multipart_structure, mock_text_part, EmailConnector, MockConnector, PartStream,
};
pub use error::{MailinerError, Result};
pub use ids::{AccountId, FolderId, MessageId, MessagePartId};
pub use models::{
    Account, AccountMetadata, EmailAddr, EmailAddress, Envelope, Folder, FolderMetadata, Group,
    LoadedMessage, MessageContent, MessagePart, PartChunk, PartKind, TransferEncoding,
};
pub use storage::{InMemoryStorage, Storage};
