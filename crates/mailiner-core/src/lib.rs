pub mod auth;
pub mod body;
pub mod connector;
pub mod error;
pub mod folder_name;
pub mod ids;
pub mod imap_search;
pub mod models;
pub mod submit;

pub use auth::{AuthResults, AuthVerdict};
pub use body::{BodyPart, ContentDisposition};
pub use connector::{
    mock_multipart_structure, mock_rfc822, mock_text_part, EmailConnector, MockConnector,
    PartStream, MOCK_RFC822_HEADERS,
};
pub use error::{MailinerError, Result};
pub use folder_name::{
    is_inbox_mailbox, join_mailbox_path, mailbox_parent_and_leaf, rename_mailbox_path,
    validate_folder_name, FolderNameError,
};
pub use ids::{AccountId, EmptyMessageId, FolderId, MessageId, MessagePartId};
pub use imap_search::{
    compile_list_search, compile_unread_sort_extra, join_search_keys, mailbox_search_is_active,
    CompiledSearch, MailboxSearch, SearchTerm,
};
pub use models::{
    is_calendar_mime, is_rfc822_mime, primary_mime, EmailAddr, EmailAddress, Envelope,
    EnvelopeFlag, Folder, FolderCounts, FolderListState, Group, ImapKeyword, LoadedMessage,
    MailboxQuota, MailboxRole, MessageContent, MessageListFilter, MessagePart, MessageSort,
    NestedMessageHeaders, PartChunk, PartKind, TextPrefix, TransferEncoding,
};
pub use submit::{
    sanitize_envid, DsnRequest, DsnReturn, SendErrorKind, SubmitReceipt, SubmitRequest,
};
