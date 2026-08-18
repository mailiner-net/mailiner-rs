pub use mailiner_core::ids::AccountId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    /// IMAP hostname (non-secret display field for account list).
    pub host: String,
}
