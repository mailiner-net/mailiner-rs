use std::{convert::Infallible, str::FromStr};

use crate::kernel::backend::BackendType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub String);

impl Into<String> for AccountId {
    fn into(self) -> String {
        self.0
    }
}

impl FromStr for AccountId {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AccountId(s.to_string()))
    }
}

impl ToString for AccountId {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    pub backend_type: BackendType,
    // Other account metadata
}
