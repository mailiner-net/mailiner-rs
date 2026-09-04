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
            aria_label: crate::i18n::t("outbox.title"),
            h3 { {crate::i18n::t_args("outbox.heading", &[("n", &items.len().to_string())])} }
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
                                r#type: "button",
                                aria_label: "Retry {item.subject}",
                                onclick: {
                                    let id = item.id.clone();
                                    move |_| {
                                        core.send(CoreEvent::RetryOutboxItem { id: id.clone() });
                                    }
                                },
                                {crate::i18n::t("outbox.retry")}
                            }
                            button {
                                r#type: "button",
                                aria_label: "Delete {item.subject}",
                                onclick: {
                                    let id = item.id.clone();
                                    move |_| {
                                        core.send(CoreEvent::DeleteOutboxItem { id: id.clone() });
                                    }
                                },
                                {crate::i18n::t("outbox.delete")}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn state_label(state: OutboxItemState) -> String {
    match state {
        OutboxItemState::Queued => crate::i18n::t("outbox.queued"),
        OutboxItemState::Sending => crate::i18n::t("outbox.sending"),
        OutboxItemState::Failed => crate::i18n::t("outbox.failed"),
    }
}
