//! Outbox list (queued / failed / sending).

use dioxus::prelude::*;

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::outbox_store::OutboxItemState;

#[component]
pub fn OutboxPanel() -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let items = ctx.outbox.read().clone();
    if items.is_empty() {
        return rsx! {};
    }

    rsx! {
        section {
            class: "outbox-panel",
            h3 { "Outbox ({items.len()})" }
            ul {
                class: "outbox-list",
                for item in items {
                    li {
                        key: "{item.id.as_str()}",
                        class: "outbox-item",
                        div {
                            class: "outbox-item-main",
                            strong {
                                if item.subject.is_empty() { "(no subject)" } else { "{item.subject}" }
                            }
                            span { class: "outbox-to", " → {item.to_preview}" }
                            span {
                                class: "outbox-state",
                                "{state_label(item.state)}"
                            }
                        }
                        if let Some(err) = &item.last_error {
                            p { class: "outbox-error", "{err}" }
                        }
                        div {
                            class: "outbox-actions",
                            button {
                                onclick: {
                                    let id = item.id.clone();
                                    move |_| {
                                        core.send(CoreEvent::RetryOutboxItem { id: id.clone() });
                                    }
                                },
                                "Retry"
                            }
                            button {
                                onclick: {
                                    let id = item.id.clone();
                                    move |_| {
                                        core.send(CoreEvent::DeleteOutboxItem { id: id.clone() });
                                    }
                                },
                                "Delete"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn state_label(state: OutboxItemState) -> &'static str {
    match state {
        OutboxItemState::Queued => "Queued",
        OutboxItemState::Sending => "Sending",
        OutboxItemState::Failed => "Failed",
    }
}
