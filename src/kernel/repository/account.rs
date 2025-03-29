use crate::kernel::model::{Account, AccountId};
use crate::kernel::Result;
use async_trait::async_trait;

#[async_trait]
pub trait AccountRepository: Send + Sync + 'static {
    async fn list_accounts(&self) -> Result<Vec<Account>>;
    async fn get_account(&self, id: &AccountId) -> Result<Option<Account>>;
    async fn create_account(&self, account: Account) -> Result<Account>;
    async fn update_account(&self, account: Account) -> Result<Account>;
    async fn delete_account(&self, id: &AccountId) -> Result<()>;
}

