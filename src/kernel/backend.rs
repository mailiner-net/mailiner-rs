use serde::{Deserialize, Serialize};

mod mock;

pub use mock::MockBackend;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BackendType {
    Imap,
    Jmap,
    Mock,
}

