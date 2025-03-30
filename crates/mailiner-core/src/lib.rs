pub mod error;
pub mod ids;
pub mod models;
pub mod storage;
pub mod connector;

pub use error::{MailinerError, Result};
pub use ids::{AccountId, FolderId, MessageId, MessagePartId};
pub use models::{
    Account, AccountMetadata, Envelope, Folder, FolderMetadata,
    MessagePart, MessageContent,
    EmailAddress, EmailAddr, Group,
};
pub use storage::{Storage, InMemoryStorage};
pub use connector::{EmailConnector, MockConnector};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
