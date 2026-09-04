use std::ops::Range;
use std::sync::Arc;

use dioxus::html::Key;
use dioxus::prelude::*;

use crate::account::AccountId;
use crate::components::emailnavigation::navigationheader::{Mode, NavigationHeader};
use crate::components::icons::{Icon, IconKind};
use crate::components::virtual_scroll::{SparseList, VirtualScroll};
use crate::context::{AppContext, MessageDrag};
use crate::conversation::{
    ConversationId, ConversationRow, flatten_conversations, group_conversations,
    row_index_for_message,
};
use crate::core_event::CoreEvent;
use crate::download::{DownloadStatus, EML_DOWNLOAD_KEY, MAIL_EXPORT_KEY};
use crate::keywords::MessageKeywordChips;
use crate::mail_file::{MailExportFormat, export_items_from};
use crate::mailbox::MailboxId;
use crate::message::Message;
use crate::message_list_filter::message_matches_filter;
use crate::selection::{drag_message_ids, export_selection};
use crate::ui_prefs::MessageListView;
use crate::unified_inbox::is_unified_mailbox;
use chrono::{DateTime, Utc};

fn send_export(
    ctx: &AppContext,
    core_tx: &Coroutine<CoreEvent>,
    account_id: Option<AccountId>,
    mailbox_id: Option<MailboxId>,
    folder_label: String,
    format: MailExportFormat,
) {
    let (Some(account_id), Some(mailbox_id)) = (account_id, mailbox_id) else {
        return;
    };
    let items = export_items_from(export_selection(
        &ctx.selection.read(),
        ctx.messages.read().iter().map(|m| m.as_ref()),
    ));
    if items.is_empty() {
        return;
    }
    let _ = core_tx.send(CoreEvent::ExportMessages {
        account_id,
        mailbox_id,
        items,
        format,
        folder_label,
    });
}

fn list_date(dt: &DateTime<Utc>) -> String {
    crate::i18n::format_list_date(dt, &Utc::now())
}

const BUFFER_SIZE: usize = 5;
const MAX_CACHED: usize = 500;

#[component]
pub fn MessageList() -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let selected_mailbox = ctx.selected_mailbox.read().clone();
    let unified_selected = selected_mailbox.as_ref().is_some_and(is_unified_mailbox);
    let loading = *ctx.messages_loading.read();
    let total = ctx.messages.read().total_count();
    let density = *ctx.message_list_density.read();
    let list_view = *ctx.message_list_view.read();
    let conversation_mode = list_view == MessageListView::Conversations;
    let filter = *ctx.message_list_filter.read();
    let cached = ctx.messages.read().cached_count();
    let fully_loaded = cached >= total;
    let selected_n = ctx.selection.read().len();
    let export_busy = matches!(
        ctx.download_status.read().get(MAIL_EXPORT_KEY),
        Some(DownloadStatus::Queued | DownloadStatus::InProgress { .. })
    ) || matches!(
        ctx.download_status.read().get(EML_DOWNLOAD_KEY),
        Some(DownloadStatus::Queued | DownloadStatus::InProgress { .. })
    );
    let selected_account = ctx.selected_account.read().clone();
    let export_folder_label = selected_mailbox
        .as_ref()
        .and_then(|id| ctx.mailbox_nodes.read().get(id).map(|n| n.display_title()))
        .unwrap_or_else(|| "mailbox".into());
    let mut list_text_filter = ctx.list_text_filter;
    let filter_query = list_text_filter.read().clone();
    let search_query = ctx.list_search_query.read().clone();
    let search_active = mailiner_core::mailbox_search_is_active(&search_query);
    let filtering = filter.has_attachment;
    let filtered_matches: Vec<Arc<Message>> = if filtering {
        let pinned_uids = ctx.pinned_uids.read().clone();
        let mut matches: Vec<Arc<Message>> = ctx
            .messages
            .read()
            .iter()
            .filter(|m| message_matches_filter(m, filter))
            .cloned()
            .collect();
        crate::pin::sort_pinned_first(&mut matches, &pinned_uids, |m| m.id.as_uid());
        matches
    } else {
        Vec::new()
    };
    let match_count = filtered_matches.len();
    let has_known = if filtering {
        match_count > 0
    } else {
        cached > 0
    };
    let no_loaded_matches = filtering && match_count == 0 && (fully_loaded || total == 0);

    let mut filtered_items = use_signal(|| SparseList::<Arc<Message>>::new(0));
    if filtering {
        let mut next = SparseList::new(filtered_matches.len());
        next.insert_batch(0, filtered_matches.clone());
        if *filtered_items.peek() != next {
            filtered_items.set(next);
        }
    } else if filtered_items.peek().total_count() != 0 {
        filtered_items.set(SparseList::new(0));
    }

    let conversation_source: Vec<Arc<Message>> = if conversation_mode {
        if filtering {
            filtered_matches.clone()
        } else {
            ctx.messages.read().iter().cloned().collect()
        }
    } else {
        Vec::new()
    };
    let conversations = if conversation_mode {
        group_conversations(conversation_source, ctx.pinned_uids.read().as_slice())
    } else {
        Vec::new()
    };
    let conversation_count = conversations.len();
    let conversation_rows = if conversation_mode {
        flatten_conversations(&conversations, &ctx.expanded_conversations.read())
    } else {
        Vec::new()
    };

    let mut conversation_items = use_signal(|| SparseList::<ConversationRow>::new(0));
    if conversation_mode {
        let mut next = SparseList::new(conversation_rows.len());
        next.insert_batch(0, conversation_rows.clone());
        if *conversation_items.peek() != next {
            conversation_items.set(next);
        }
    } else if conversation_items.peek().total_count() != 0 {
        conversation_items.set(SparseList::new(0));
    }

    use_effect(move || {
        let filter = *ctx.message_list_filter.read();
        let total = ctx.messages.read().total_count();
        let cached = ctx.messages.read().cached_count();
        let loading = *ctx.messages_loading.read();
        if !filter.has_attachment || cached >= total || total == 0 || loading {
            return;
        }
        let Some(range) = ctx
            .messages
            .read()
            .missing_ranges(0, total)
            .into_iter()
            .next()
        else {
            return;
        };
        let end = (range.start + 50).min(range.end);
        if let Some(mailbox_id) = ctx.selected_mailbox.peek().clone() {
            let _ = core_tx.send(CoreEvent::FetchMessageRange {
                mailbox_id,
                range: range.start..end,
            });
        }
    });

    let on_need_range = move |range: Range<usize>| {
        if let Some(mailbox_id) = ctx.selected_mailbox.peek().clone() {
            let _ = core_tx.send(CoreEvent::FetchMessageRange { mailbox_id, range });
        }
    };

    let render_item = move |args: (usize, Arc<Message>)| -> Element {
        let (index, message) = args;
        rsx! { MessageListItem { index, message } }
    };

    let render_filtered_item = move |args: (usize, Arc<Message>)| -> Element {
        let message = args.1;
        let index = ctx
            .messages
            .read()
            .position(|m| m.id == message.id)
            .unwrap_or(0);
        rsx! { MessageListItem { index, message } }
    };

    let render_conversation_item = move |args: (usize, ConversationRow)| -> Element {
        let row = args.1;
        let focus = ctx.selection.read().focus().cloned();
        let selection = ctx.selection.read();
        match row {
            ConversationRow::Thread {
                conversation,
                expanded,
            } => {
                let message = Arc::clone(conversation.open_target());
                let index = ctx
                    .messages
                    .read()
                    .position(|m| m.id == message.id)
                    .unwrap_or(0);
                let row_focused =
                    !expanded && focus.as_ref().is_some_and(|id| conversation.contains(id));
                let row_selected = conversation
                    .members
                    .iter()
                    .any(|m| selection.contains(&m.id));
                rsx! {
                    MessageListItem {
                        index,
                        message,
                        conversation_id: Some(conversation.id.clone()),
                        thread_count: conversation.count(),
                        thread_expanded: expanded,
                        row_focused: Some(row_focused),
                        row_selected: Some(row_selected),
                    }
                }
            }
            ConversationRow::Message {
                conversation_id,
                message,
                indented,
            } => {
                let index = ctx
                    .messages
                    .read()
                    .position(|m| m.id == message.id)
                    .unwrap_or(0);
                rsx! {
                    MessageListItem {
                        index,
                        message,
                        conversation_id: Some(conversation_id),
                        indented,
                    }
                }
            }
        }
    };

    rsx! {
        aside {
            id: "messagelist",
            class: "{density.css_class()}",
            aria_label: "Message list",

            NavigationHeader {
                mode: Mode::MessageList,
            }

            if selected_mailbox.is_some() && !unified_selected {
                div {
                    class: "message-list-filter",
                    input {
                        class: "ui-input",
                        r#type: "search",
                        id: "mailbox-search",
                        value: "{filter_query}",
                        placeholder: "Search this folder",
                        aria_label: "Search this folder",
                        title: "IMAP search (Enter). from: to: subject: body: after: before: is:unread is:flagged has:attachment",
                        autocomplete: "off",
                        spellcheck: false,
                        oninput: move |evt| list_text_filter.set(evt.value()),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter {
                                evt.prevent_default();
                                let query = list_text_filter.peek().clone();
                                let _ = core_tx.send(CoreEvent::ApplyMailboxSearch { query });
                            } else if evt.key() == Key::Escape
                                && (!list_text_filter.peek().is_empty()
                                    || mailiner_core::mailbox_search_is_active(
                                        ctx.list_search_query.peek().as_str(),
                                    ))
                            {
                                evt.prevent_default();
                                list_text_filter.set(String::new());
                                let _ = core_tx.send(CoreEvent::ApplyMailboxSearch {
                                    query: String::new(),
                                });
                            }
                        },
                    }
                    button {
                        r#type: "button",
                        class: "message-list-search-btn",
                        title: "Search this folder (Enter)",
                        onclick: move |_| {
                            let query = list_text_filter.peek().clone();
                            let _ = core_tx.send(CoreEvent::ApplyMailboxSearch { query });
                        },
                        "Search"
                    }
                    if mailiner_core::mailbox_search_is_active(&filter_query)
                        || search_active
                    {
                        button {
                            r#type: "button",
                            class: "message-list-search-btn",
                            title: "Save this search in the folder tree",
                            onclick: move |_| {
                                let query = {
                                    let draft = list_text_filter.peek().clone();
                                    if mailiner_core::mailbox_search_is_active(&draft) {
                                        draft
                                    } else {
                                        ctx.list_search_query.peek().clone()
                                    }
                                };
                                if !mailiner_core::mailbox_search_is_active(&query) {
                                    return;
                                }
                                let Some(name) = super::mailboxtreeview::prompt_folder_name(
                                    "Save search as",
                                    &query,
                                ) else {
                                    return;
                                };
                                let _ = core_tx.send(CoreEvent::SaveMailboxSearch { name, query });
                            },
                            "Save"
                        }
                    }
                    if conversation_mode && has_known {
                        span {
                            class: "message-list-filter-count",
                            aria_live: "polite",
                            if fully_loaded || filtering {
                                "{conversation_count} conversations"
                            } else {
                                "{conversation_count} conversations · {cached} loaded"
                            }
                        }
                    } else if search_active && !filter.has_attachment {
                        span {
                            class: "message-list-filter-count",
                            aria_live: "polite",
                            "{total} results"
                        }
                    } else if filtering {
                        span {
                            class: "message-list-filter-count",
                            aria_live: "polite",
                            "{match_count} of {cached} loaded"
                        }
                    }
                }
            }

            if unified_selected && !ctx.unified_inbox_notes.read().is_empty() {
                ul {
                    class: "unified-inbox-notes",
                    for note in ctx.unified_inbox_notes.read().iter() {
                        {
                            let label = ctx
                                .accounts
                                .read()
                                .get(&note.account_id)
                                .map(|a| {
                                    if a.name.is_empty() {
                                        a.email.clone()
                                    } else {
                                        a.name.clone()
                                    }
                                })
                                .unwrap_or_else(|| note.account_id.to_string());
                            let text = note.message(&label);
                            rsx! {
                                li {
                                    class: "unified-inbox-note",
                                    "{text}"
                                }
                            }
                        }
                    }
                }
            }

            if selected_mailbox.is_some() && !loading && total > 0 {
                div {
                    class: "message-list-selection",
                    button {
                        r#type: "button",
                        class: "message-list-select-action",
                        title: if filtering {
                            "Select all matching loaded messages (Ctrl+A)"
                        } else {
                            "Select all loaded messages (Ctrl+A)"
                        },
                        disabled: !has_known,
                        onclick: move |_| {
                            let _ = core_tx.send(CoreEvent::SelectAllKnown);
                        },
                        "Select all"
                    }
                    button {
                        r#type: "button",
                        class: "message-list-select-action",
                        title: if filtering {
                            "Select unread matching loaded messages"
                        } else {
                            "Select unread loaded messages"
                        },
                        disabled: !has_known,
                        onclick: move |_| {
                            let _ = core_tx.send(CoreEvent::SelectUnreadKnown);
                        },
                        "Unread"
                    }
                    button {
                        r#type: "button",
                        class: "message-list-select-action",
                        title: if filtering {
                            "Invert selection among matching loaded messages"
                        } else {
                            "Invert selection among loaded messages"
                        },
                        disabled: !has_known,
                        onclick: move |_| {
                            let _ = core_tx.send(CoreEvent::InvertSelection);
                        },
                        "Invert"
                    }
                    if selected_n > 0 {
                        span {
                            class: "message-list-selected-count",
                            aria_live: "polite",
                            "{selected_n} selected"
                        }
                        button {
                            r#type: "button",
                            class: "message-list-select-action",
                            title: if selected_n == 1 {
                                "Download selected message as .eml"
                            } else {
                                "Download selected messages as a zip of .eml files"
                            },
                            disabled: export_busy || selected_account.is_none(),
                            onclick: {
                                let mailbox_id = selected_mailbox.clone();
                                let account_id = selected_account.clone();
                                let folder_label = export_folder_label.clone();
                                let ctx = ctx.clone();
                                move |_| {
                                    send_export(
                                        &ctx,
                                        &core_tx,
                                        account_id.clone(),
                                        mailbox_id.clone(),
                                        folder_label.clone(),
                                        MailExportFormat::EmlZip,
                                    );
                                }
                            },
                            if export_busy { "Exporting…" } else if selected_n == 1 { "Export .eml" } else { "Export .eml zip" }
                        }
                        button {
                            r#type: "button",
                            class: "message-list-select-action",
                            title: "Download selected messages as mbox",
                            disabled: export_busy || selected_account.is_none(),
                            onclick: {
                                let mailbox_id = selected_mailbox.clone();
                                let account_id = selected_account.clone();
                                let folder_label = export_folder_label.clone();
                                let ctx = ctx.clone();
                                move |_| {
                                    send_export(
                                        &ctx,
                                        &core_tx,
                                        account_id.clone(),
                                        mailbox_id.clone(),
                                        folder_label.clone(),
                                        MailExportFormat::Mbox,
                                    );
                                }
                            },
                            "Export mbox"
                        }
                    }
                }
            }

            div {
                class: "message-list-body",
                role: "listbox",
                aria_label: crate::i18n::t("list.messages_aria"),
                aria_multiselectable: "true",

                if selected_mailbox.is_none() {
                    div {
                        class: "message-list-empty",
                        {crate::i18n::t("list.select_mailbox")}
                    }
                } else if loading {
                    div {
                        class: "message-list-empty",
                        if search_active { {crate::i18n::t("list.searching")} } else { {crate::i18n::t("list.loading")} }
                    }
                } else if total == 0 && filter.is_empty() && !search_active {
                    div {
                        class: "message-list-empty",
                        {crate::i18n::t("list.no_messages")}
                    }
                } else if total == 0 && (search_active || !filter.is_empty()) && !filter.has_attachment {
                    div {
                        class: "message-list-empty",
                        {crate::i18n::t("list.no_match")}
                    }
                } else if no_loaded_matches {
                    div {
                        class: "message-list-empty",
                        {crate::i18n::t("list.no_loaded_match")}
                    }
                } else if conversation_mode {
                    VirtualScroll {
                        key: "conv-{search_query}-{filter.has_attachment}",
                        items: conversation_items,
                        item_height: density.item_height(),
                        buffer_size: BUFFER_SIZE,
                        debounce_ms: Some(100),
                        max_cached: None,
                        reveal_index: ctx.selection.read().focus().and_then(|id| {
                            let rows: Vec<ConversationRow> = conversation_items
                                .read()
                                .iter()
                                .cloned()
                                .collect();
                            row_index_for_message(&rows, id)
                        }),
                        on_need_range: move |_: Range<usize>| {},
                        render_item: render_conversation_item,
                    }
                } else if filtering {
                    // Remount when the attachment chip changes so a prior
                    // scroll offset cannot sit below the new list.
                    VirtualScroll {
                        key: "attach-{filter.has_attachment}",
                        items: filtered_items,
                        item_height: density.item_height(),
                        buffer_size: BUFFER_SIZE,
                        debounce_ms: Some(100),
                        max_cached: None,
                        reveal_index: ctx.selection.read().focus().and_then(|id| {
                            filtered_items.read().position(|m| m.id == *id)
                        }),
                        on_need_range: move |_: Range<usize>| {},
                        render_item: render_filtered_item,
                    }
                } else {
                    VirtualScroll {
                        key: "search-{search_query}",
                        items: ctx.messages,
                        item_height: density.item_height(),
                        buffer_size: BUFFER_SIZE,
                        debounce_ms: Some(100),
                        max_cached: Some(MAX_CACHED),
                        reveal_index: ctx.selection.read().focus().and_then(|id| {
                            ctx.messages.read().position(|m| m.id == *id)
                        }),
                        on_need_range: on_need_range,
                        render_item: render_item,
                    }
                }
            }
        }
    }
}

#[component]
fn MessageListItem(
    index: usize,
    message: Arc<Message>,
    #[props(default)] indented: bool,
    #[props(default)] thread_count: usize,
    #[props(default)] thread_expanded: bool,
    #[props(default)] conversation_id: Option<ConversationId>,
    #[props(default)] row_focused: Option<bool>,
    #[props(default)] row_selected: Option<bool>,
) -> Element {
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let ctx = use_context::<AppContext>();
    let mut expanded_conversations = ctx.expanded_conversations;
    let mut message_drag = ctx.message_drag;
    let selection = ctx.selection.read();
    let is_selected = row_selected.unwrap_or_else(|| selection.contains(&message.id));
    let is_focused = row_focused.unwrap_or_else(|| selection.focus() == Some(&message.id));
    let is_dragging = message_drag
        .read()
        .as_ref()
        .is_some_and(|d| d.message_ids.contains(&message.id));
    let avatar = message.avatar_color();
    let message_id = message.id.clone();
    let star_id = message.id.clone();
    let flag_id = message.id.clone();
    let pin_id = message.id.clone();
    let drag_id = message.id.clone();
    let unified_row = ctx
        .selected_mailbox
        .peek()
        .as_ref()
        .is_some_and(is_unified_mailbox);
    let row_account = if unified_row {
        Some(message.envelope.account_id.clone())
    } else {
        ctx.selected_account.peek().clone()
    };
    let row_mailbox = MailboxId::from(message.id.folder_id().clone());
    let account_label = if unified_row {
        ctx.accounts
            .read()
            .get(&message.envelope.account_id)
            .map(|a| {
                if a.name.trim().is_empty() {
                    a.email.clone()
                } else {
                    a.name.clone()
                }
            })
            .unwrap_or_else(|| message.envelope.account_id.to_string())
    } else {
        String::new()
    };
    let star_account = row_account.clone();
    let flag_account = row_account.clone();
    let pin_account = row_account;
    let star_mailbox = row_mailbox.clone();
    let flag_mailbox = row_mailbox.clone();
    let pin_mailbox = row_mailbox;
    let is_starred = message.is_starred;
    let is_flagged = message.is_flagged;
    let is_pinned = ctx
        .pinned_uids
        .read()
        .iter()
        .any(|uid| uid == message.id.as_uid());
    let is_thread = thread_count > 1;
    let thread_id = conversation_id.clone();
    let expand_id = conversation_id.clone();

    rsx! {
        div {
            class: "message-list-item",
            class: if is_selected { "selected" },
            class: if is_focused { "focused" },
            class: if !message.is_read { "unread" },
            class: if is_dragging { "dragging" },
            class: if is_pinned { "is-pinned" },
            class: if indented { "is-thread-member" },
            class: if is_thread { "is-thread" },
            role: "option",
            aria_selected: if is_selected { "true" } else { "false" },
            aria_expanded: if is_thread {
                Some(if thread_expanded { "true" } else { "false" })
            } else {
                None
            },
            aria_label: if is_thread {
                format!(
                    "{}, {}, {} messages",
                    message.from_preview(),
                    message.subject,
                    thread_count
                )
            } else {
                format!("{}, {}", message.from_preview(), message.subject)
            },
            draggable: if unified_row { "false" } else { "true" },

            onmousedown: move |evt: MouseEvent| {
                if evt.modifiers().shift() || evt.modifiers().ctrl() || evt.modifiers().meta() {
                    evt.prevent_default();
                }
            },

            onclick: move |evt: MouseEvent| {
                evt.prevent_default();
                if is_thread && !thread_expanded {
                    if let Some(id) = thread_id.clone() {
                        expanded_conversations.write().insert(id);
                    }
                }
                let _ = core_tx.send(CoreEvent::SelectListClick {
                    message_id: message_id.clone(),
                    index,
                    extend: evt.modifiers().shift(),
                    toggle: evt.modifiers().ctrl() || evt.modifiers().meta(),
                });
            },

            ondragstart: move |evt: DragEvent| {
                if unified_row {
                    evt.prevent_default();
                    return;
                }
                let Some(source_mailbox) = ctx.selected_mailbox.peek().clone() else {
                    return;
                };
                let ids = drag_message_ids(&ctx.selection.peek(), &drag_id);
                let dt = evt.data_transfer();
                let _ = dt.set_data("text/plain", "mailiner-messages");
                dt.set_effect_allowed("move");
                message_drag.set(Some(MessageDrag {
                    message_ids: ids,
                    source_mailbox,
                    over: None,
                }));
            },

            ondragend: move |_| {
                message_drag.set(None);
            },

            if is_thread {
                button {
                    class: "message-thread-toggle",
                    r#type: "button",
                    aria_expanded: if thread_expanded { "true" } else { "false" },
                    aria_label: if thread_expanded { "Collapse conversation" } else { "Expand conversation" },
                    title: if thread_expanded { "Collapse conversation" } else { "Expand conversation" },
                    onmousedown: move |evt: MouseEvent| {
                        evt.stop_propagation();
                    },
                    onclick: move |evt: MouseEvent| {
                        evt.stop_propagation();
                        evt.prevent_default();
                        let Some(id) = expand_id.clone() else {
                            return;
                        };
                        let mut open = expanded_conversations.write();
                        if !open.remove(&id) {
                            open.insert(id);
                        }
                    },
                    Icon {
                        size: 14,
                        icon: if thread_expanded { IconKind::ChevronDown } else { IconKind::ChevronRight },
                    }
                }
            }

            div {
                class: "message-avatar",
                style: "background-color: {avatar}",
                aria_hidden: "true",
            }

            div {
                class: "message-list-item-content",

                div {
                    class: "message-list-item-top",
                    div {
                        class: "message-from",
                        "{message.from_preview()}"
                        if is_thread {
                            span {
                                class: "message-thread-count",
                                title: "{thread_count} messages in this conversation",
                                "{thread_count}"
                            }
                        }
                    }
                    if unified_row && !account_label.is_empty() {
                        span {
                            class: "message-account",
                            title: "{account_label}",
                            "{account_label}"
                        }
                    }
                    div {
                        class: "message-list-item-meta",
                        if message.is_answered {
                            span {
                                class: "message-answered-indicator",
                                role: "img",
                                aria_label: "Replied",
                                title: "Replied",
                                Icon { size: 14, icon: IconKind::ArrowUturnLeft }
                            }
                        }
                        button {
                            class: "message-star-indicator",
                            class: if is_starred { "is-on" },
                            r#type: "button",
                            aria_pressed: if is_starred { "true" } else { "false" },
                            aria_label: if is_starred { "Unstar" } else { "Star" },
                            title: if is_starred { "Unstar" } else { "Star" },
                            onmousedown: move |evt: MouseEvent| {
                                evt.stop_propagation();
                            },
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                evt.prevent_default();
                                let Some(account_id) = star_account.clone() else {
                                    return;
                                };
                                let _ = core_tx.send(CoreEvent::ToggleStar {
                                    account_id,
                                    mailbox_id: star_mailbox.clone(),
                                    message_ids: vec![star_id.clone()],
                                });
                            },
                            Icon { size: 14, icon: IconKind::Star }
                        }
                        button {
                            class: "message-flag-indicator",
                            class: if is_flagged { "is-on" },
                            r#type: "button",
                            aria_pressed: if is_flagged { "true" } else { "false" },
                            aria_label: if is_flagged { "Unflag" } else { "Flag" },
                            title: if is_flagged { "Unflag" } else { "Flag" },
                            onmousedown: move |evt: MouseEvent| {
                                evt.stop_propagation();
                            },
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                evt.prevent_default();
                                let Some(account_id) = flag_account.clone() else {
                                    return;
                                };
                                let _ = core_tx.send(CoreEvent::ToggleFlag {
                                    account_id,
                                    mailbox_id: flag_mailbox.clone(),
                                    message_ids: vec![flag_id.clone()],
                                });
                            },
                            Icon { size: 14, icon: IconKind::Flag }
                        }
                        button {
                            class: "message-pin-indicator",
                            class: if is_pinned { "is-on" },
                            r#type: "button",
                            aria_pressed: if is_pinned { "true" } else { "false" },
                            aria_label: if is_pinned { "Unpin" } else { "Pin" },
                            title: if is_pinned { "Unpin" } else { "Pin" },
                            onmousedown: move |evt: MouseEvent| {
                                evt.stop_propagation();
                            },
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                evt.prevent_default();
                                let Some(account_id) = pin_account.clone() else {
                                    return;
                                };
                                let _ = core_tx.send(CoreEvent::TogglePin {
                                    account_id,
                                    mailbox_id: pin_mailbox.clone(),
                                    message_ids: vec![pin_id.clone()],
                                });
                            },
                            Icon { size: 14, icon: IconKind::Pin }
                        }
                        if message.has_attachments {
                            span {
                                class: "message-attachment-indicator",
                                role: "img",
                                aria_label: "Has attachments",
                                title: "Has attachments",
                                Icon { size: 14, icon: IconKind::PaperClip }
                            }
                        }
                        div {
                            class: "message-date",
                            "{list_date(&message.date)}"
                        }
                    }
                }

                div {
                    class: "message-subject",
                    div {
                        class: "message-subject-line",
                        if message.subject.trim().is_empty() {
                            span { class: "message-subject-empty", "(no subject)" }
                        } else {
                            span { class: "message-subject-text", "{message.subject}" }
                        }
                        MessageKeywordChips {
                            atoms: message.envelope.keywords.clone(),
                            compact: true,
                        }
                    }
                    if let Some(snippet) = message.snippet.as_deref().filter(|s| !s.is_empty()) {
                        div {
                            class: "message-snippet",
                            "{snippet}"
                        }
                    }
                }
            }
        }
    }
}
