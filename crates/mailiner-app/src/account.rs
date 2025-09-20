use uuid::Uuid;

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct AccountId(Uuid);

impl AccountId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for AccountId {
    fn from(value: Uuid) -> Self {
        AccountId(value)
    }
}

pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
}

