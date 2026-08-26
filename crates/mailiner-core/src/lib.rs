pub mod body;
pub mod connector;
pub mod error;
pub mod ids;
pub mod models;
pub mod submit;

pub use body::{BodyPart, ContentDisposition};
pub use connector::{
    mock_multipart_structure, mock_text_part, EmailConnector, MockConnector, PartStream,
};
pub use error::{MailinerError, Result};
pub use ids::{AccountId, FolderId, MessageId, MessagePartId};
pub use models::{
    Account, EmailAddr, EmailAddress, Envelope, Folder, FolderCounts, FolderListState, Group,
    LoadedMessage, MailboxRole, MessageContent, MessagePart, MessageSort, PartChunk, PartKind,
    TransferEncoding,
};
pub use submit::{SendErrorKind, SubmitReceipt, SubmitRequest};
