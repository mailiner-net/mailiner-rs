pub use mailiner_core::ids::AccountId;

use crate::account_config::AccountIdentity;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    /// IMAP hostname (non-secret display field for account list).
    pub host: String,
    /// Optional plain-text signature (not a secret).
    pub signature: Option<String>,
    /// Extra From identities (name + email aliases). Primary is `name` + `email`.
    pub identities: Vec<AccountIdentity>,
}

impl Account {
    /// Primary From identity (`name` + `email`).
    pub fn primary_identity(&self) -> AccountIdentity {
        AccountIdentity::new(self.name.clone(), self.email.clone())
    }

    /// Primary first, then extras stored on the account.
    pub fn all_identities(&self) -> Vec<AccountIdentity> {
        crate::account_config::account_identities(&self.name, &self.email, &self.identities)
    }
}
