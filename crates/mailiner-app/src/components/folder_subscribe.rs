//! Subscribe / unsubscribe manager: pick which folders appear in the tree.

use dioxus::html::Key;
use dioxus::prelude::*;

use mailiner_core::MailboxRole;

use super::icons::{Icon, IconButton, IconKind};
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::{
    collect_all_mailbox_entries, filter_mailbox_entries, mailbox_is_action_target,
};

fn role_icon(role: MailboxRole) -> IconKind {
    match role {
        MailboxRole::Inbox => IconKind::Inbox,
        MailboxRole::Archive => IconKind::ArchiveBox,
        MailboxRole::Drafts => IconKind::PencilSquare,
        MailboxRole::Sent => IconKind::PaperAirplane,
        MailboxRole::Outbox => IconKind::InboxStack,
        MailboxRole::Trash => IconKind::Trash,
        MailboxRole::Junk => IconKind::NoSymbol,
        MailboxRole::Other => IconKind::Folder,
    }
}

#[component]
pub fn FolderSubscribeHost() -> Element {
    let ctx = use_context::<AppContext>();
    if !*ctx.folder_subscribe_open.read() {
        return rsx! {};
    }
    rsx! { FolderSubscribeDialog {} }
}

#[component]
fn FolderSubscribeDialog() -> Element {
    let mut ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let mut query = use_signal(String::new);
    let entries = {
        let nodes = ctx.mailbox_nodes.read();
        let roots = ctx.mailbox_roots.read();
        collect_all_mailbox_entries(&roots, &nodes)
    };
    let filtered = filter_mailbox_entries(&entries, &query.read());
    let mut show_all = ctx.show_all_folders;

    let mut close = move |_| {
        ctx.folder_subscribe_open.set(false);
    };

    rsx! {
        div {
            class: "picker-backdrop",
            onclick: move |_| close(()),
            div {
                class: "ui-dialog picker-dialog subscribe-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_label: "Folder subscriptions",
                onclick: move |evt| evt.stop_propagation(),
                onkeydown: move |evt: KeyboardEvent| {
                    if evt.key() == Key::Escape {
                        evt.prevent_default();
                        ctx.folder_subscribe_open.set(false);
                    }
                },

                div {
                    class: "ui-dialog-head",
                    h2 { class: "ui-dialog-title", "Folder subscriptions" }
                    IconButton {
                        class: "flat ui-icon-btn",
                        title: "Close",
                        size: 20,
                        icon: IconKind::XMark,
                        onclick: move |_| close(()),
                    }
                }

                p {
                    class: "picker-hint",
                    "Choose which folders appear in the tree. Inbox stays subscribed."
                }

                input {
                    class: "ui-input picker-filter",
                    r#type: "text",
                    value: "{query}",
                    placeholder: "Filter folders",
                    aria_label: "Filter folders",
                    onmounted: move |evt| {
                        let data = evt.data();
                        spawn(async move {
                            let _ = data.set_focus(true).await;
                        });
                    },
                    oninput: move |evt| query.set(evt.value()),
                }

                ul {
                    class: "picker-list subscribe-list",
                    if filtered.is_empty() {
                        li {
                            class: "picker-empty",
                            "No matching folders"
                        }
                    } else {
                        for entry in filtered.iter() {
                            SubscribeRow {
                                key: "{entry.id.as_str()}",
                                entry: (*entry).clone(),
                            }
                        }
                    }
                }

                label {
                    class: "subscribe-show-all",
                    input {
                        r#type: "checkbox",
                        checked: *show_all.read(),
                        onchange: move |_| {
                            let next = !*show_all.peek();
                            crate::ui_prefs::save_show_all_folders(next);
                            show_all.set(next);
                            if !next {
                                hide_unsubscribed_selection(&ctx, &core);
                            }
                        },
                    }
                    "Show unsubscribed folders in the tree"
                }
            }
        }
    }
}

#[component]
fn SubscribeRow(entry: crate::mailbox::MailboxEntry) -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let account_id = ctx.selected_account.read().clone();
    let indent = "\u{00a0}\u{00a0}".repeat(entry.depth);
    let label = format!("{indent}{}", entry.title);
    let toggle_label = if entry.subscribed {
        format!("Unsubscribe from {}", entry.title)
    } else {
        format!("Subscribe to {}", entry.title)
    };
    rsx! {
        li {
            class: "picker-row subscribe-row",
            class: if !entry.subscribed { "unsubscribed" },
            label {
                class: "subscribe-row-label",
                input {
                    r#type: "checkbox",
                    checked: entry.subscribed,
                    disabled: entry.role == MailboxRole::Inbox,
                    aria_label: "{toggle_label}",
                    onchange: {
                        let id = entry.id.clone();
                        let next = !entry.subscribed;
                        let account_id = account_id.clone();
                        move |_| {
                            let Some(account_id) = account_id.clone() else {
                                return;
                            };
                            let _ = core.send(CoreEvent::SetFolderSubscribed {
                                account_id,
                                mailbox_id: id.clone(),
                                subscribed: next,
                            });
                        }
                    },
                }
                span {
                    class: "picker-row-icon",
                    Icon { size: 16, icon: role_icon(entry.role) }
                }
                span {
                    class: "picker-row-label",
                    "{label}"
                }
            }
        }
    }
}

fn hide_unsubscribed_selection(ctx: &AppContext, core: &Coroutine<CoreEvent>) {
    let Some(selected) = ctx.selected_mailbox.peek().clone() else {
        return;
    };
    let nodes = ctx.mailbox_nodes.peek();
    let still_ok = nodes
        .get(&selected)
        .is_some_and(|n| mailbox_is_action_target(n, false));
    if still_ok {
        return;
    }
    if let Some(inbox) = crate::mailbox::find_mailbox_with_role(&nodes, MailboxRole::Inbox) {
        let _ = core.send(CoreEvent::SelectMailbox(inbox));
    }
}
