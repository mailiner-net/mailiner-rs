use dioxus::prelude::*;

#[derive(Clone)]
pub struct AppContext {
    core_tx: UnboundedSender<CoreEvent>
    mailboxes: Signal<Vec<Mailbox>>,
    messages: Signal<Vec<Message>>,
    selected_mailbox: Signal<Option<Mailbox>>,
    selected_message: Signal<Option<Message>>,
}