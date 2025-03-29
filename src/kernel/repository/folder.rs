use crate::kernel::model::AccountId;
use crate::kernel::model::{Folder, FolderId};
use crate::kernel::Result;
use async_trait::async_trait;

#[async_trait]
pub trait FolderRepository: Send + Sync + 'static {
    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>>;
    async fn get_folder(&self, id: &FolderId) -> Result<Option<Folder>>;
    async fn create_folder(&self, folder: Folder) -> Result<Folder>;
    async fn update_folder(&self, folder: Folder) -> Result<Folder>;
    async fn delete_folder(&self, id: &FolderId) -> Result<()>;
    async fn sync_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>>;
}
