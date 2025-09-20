use std::collections::HashMap;

use dioxus::prelude::*;

use crate::mailbox::{MailboxId, MailboxNode};

#[derive(Clone)]
pub struct AppContext {
    pub mailbox_nodes: Signal<HashMap<MailboxId, MailboxNode>>,
    pub mailbox_roots: Signal<Vec<MailboxId>>,

    pub selected_mailbox: Signal<Option<MailboxId>>,
}
