//! Attachments bar, per-item download, and inline image/PDF preview.

use std::collections::HashMap;

use dioxus::html::Key;
use dioxus::prelude::*;
use mailiner_core::models::TransferEncoding;

use super::icons::{IconButton, IconKind};
use crate::account::AccountId;
use crate::context::{AppContext, MessageViewState};
use crate::core_event::CoreEvent;
use crate::download::{DownloadStatus, PreviewKind, preview_kind, size_to_human};
use crate::mailbox::MailboxId;
use crate::message::MessageId;

#[derive(Clone, PartialEq)]
struct AttachmentRow {
    section: String,
    filename: String,
    content_type: String,
    size: u64,
    wire_size: Option<u64>,
    encoding: TransferEncoding,
    description: Option<String>,
}

#[component]
pub fn AttachmentsFooter() -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let view = ctx.message_view.read().clone();
    let mailbox = ctx.selected_mailbox.read().clone();

    let account = ctx.selected_account.read().clone();
    let prepared = match (view, mailbox, account) {
        (MessageViewState::Ready { message_id, loaded }, Some(mailbox_id), Some(account_id)) => {
            let rows: Vec<AttachmentRow> = loaded
                .parts
                .iter()
                .filter(|p| p.is_attachment && !p.is_hidden)
                .map(|p| AttachmentRow {
                    section: p.section(),
                    filename: crate::download::attachment_filename(
                        &p.filename,
                        &p.description,
                        &p.content_type,
                    ),
                    content_type: p.content_type.clone(),
                    size: p.size,
                    wire_size: p.original_size,
                    encoding: p.encoding,
                    description: p.description.clone(),
                })
                .collect();
            if rows.is_empty() {
                None
            } else {
                Some((account_id, message_id, mailbox_id, rows))
            }
        }
        _ => None,
    };

    rsx! {
        if let Some((account_id, message_id, mailbox_id, rows)) = prepared {
            {
                let count = rows.len();
                let summary = if count == 1 {
                    "1 attachment".to_string()
                } else {
                    format!("{count} attachments")
                };
                let any_busy = {
                    let map = ctx.download_status.read();
                    any_download_busy(&map, rows.iter().map(|r| r.section.as_str()))
                };
                let save_all_rows = rows.clone();
                let save_all_mailbox = mailbox_id.clone();
                let save_all_message = message_id.clone();
                let save_all_account = account_id.clone();
                let mut download_status = ctx.download_status;
                rsx! {
                    footer {
                        class: "message-attachments",
                        details {
                            class: "message-attachments-details",
                            summary {
                                class: "message-attachments-summary",
                                div {
                                    class: "message-attachments-summary-inner",
                                    span { "{summary}" }
                                    if count > 1 {
                                        button {
                                            class: "attachment-download-btn",
                                            r#type: "button",
                                            disabled: any_busy,
                                            title: "Download every attachment",
                                            aria_label: "Save all attachments",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                evt.prevent_default();
                                                // Hold the button disabled across the serial queue gap
                                                // (Finished of N before InProgress of N+1).
                                                mark_pending_downloads(
                                                    &mut download_status.write(),
                                                    &save_all_rows,
                                                );
                                                // Core loop is serial; queued events run one IMAP fetch at a time.
                                                // A failure leaves later items queued so remaining files still save.
                                                for event in save_all_events(
                                                    &save_all_account,
                                                    &save_all_mailbox,
                                                    &save_all_message,
                                                    &save_all_rows,
                                                ) {
                                                    let _ = core_tx.send(event);
                                                }
                                            },
                                            if any_busy {
                                                "Saving…"
                                            } else {
                                                "Save all"
                                            }
                                        }
                                    }
                                }
                            }
                            ul {
                                class: "message-attachments-list",
                                for row in rows {
                                    AttachmentItem {
                                        key: "{row.section}",
                                        account_id: account_id.clone(),
                                        message_id: message_id.clone(),
                                        mailbox_id: mailbox_id.clone(),
                                        row: row,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        AttachmentPreviewDialog {}
    }
}

fn save_all_events(
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    message_id: &MessageId,
    rows: &[AttachmentRow],
) -> Vec<CoreEvent> {
    rows.iter()
        .map(|row| attachment_download_event(account_id, mailbox_id, message_id, row))
        .collect()
}

fn attachment_download_event(
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    message_id: &MessageId,
    row: &AttachmentRow,
) -> CoreEvent {
    CoreEvent::DownloadAttachment {
        account_id: account_id.clone(),
        mailbox_id: mailbox_id.clone(),
        message_id: message_id.clone(),
        section: row.section.clone(),
        filename: row.filename.clone(),
        content_type: row.content_type.clone(),
        encoding: row.encoding,
        size_hint: row.wire_size,
    }
}

fn mark_pending_downloads(status: &mut HashMap<String, DownloadStatus>, rows: &[AttachmentRow]) {
    for row in rows {
        status.insert(row.section.clone(), DownloadStatus::Queued);
    }
}

fn any_download_busy<'a>(
    status: &HashMap<String, DownloadStatus>,
    sections: impl IntoIterator<Item = &'a str>,
) -> bool {
    sections
        .into_iter()
        .any(|section| status.get(section).is_some_and(DownloadStatus::is_busy))
}

#[component]
fn AttachmentItem(
    account_id: AccountId,
    message_id: MessageId,
    mailbox_id: MailboxId,
    row: AttachmentRow,
) -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let section = row.section.clone();
    let status = {
        let map = ctx.download_status.read();
        map.get(&section).cloned().unwrap_or(DownloadStatus::Idle)
    };
    let any_busy = ctx
        .download_status
        .read()
        .values()
        .any(|s| matches!(s, DownloadStatus::InProgress { .. }));

    let title = if let Some(desc) = &row.description {
        if desc != &row.filename {
            format!("{} ({desc})", row.filename)
        } else {
            row.filename.clone()
        }
    } else {
        row.filename.clone()
    };
    let meta = format!(
        "Size: {} · Type: {}",
        size_to_human(row.size),
        row.content_type
    );


    let progress_pct = match &status {
        DownloadStatus::InProgress {
            received,
            total: Some(t),
        } if *t > 0 => ((*received as f64 / *t as f64) * 100.0).clamp(0.0, 100.0),
        DownloadStatus::InProgress { .. } => 0.0,
        DownloadStatus::Finished => 100.0,
        _ => 0.0,
    };
    let show_progress = !matches!(status, DownloadStatus::Idle | DownloadStatus::Queued);
    let err_msg = match &status {
        DownloadStatus::Error(e) => Some(e.clone()),
        _ => None,
    };

    let row_for_click = row.clone();
    let can_preview = preview_kind(&row.content_type).is_some();
    let section_for_preview = row.section.clone();
    let filename_for_preview = row.filename.clone();
    let content_type_for_preview = row.content_type.clone();
    let account_for_preview = account_id.clone();
    let mailbox_for_preview = mailbox_id.clone();
    let message_for_preview = message_id.clone();
    let encoding = row.encoding;
    let size_hint = row.wire_size;

    rsx! {
        li {
            class: "attachment-item",

            div {
                class: "attachment-item-main",
                div {
                    class: "attachment-item-info",
                    div { class: "attachment-item-title", "{title}" }
                    div { class: "attachment-item-meta", "{meta}" }
                    if let Some(err) = err_msg {
                        div { class: "attachment-item-error", "{err}" }
                    }
                }
                div {
                    class: "attachment-item-actions",
                    if can_preview {
                        button {
                            class: "attachment-preview-btn",
                            disabled: any_busy,
                            onclick: move |_| {
                                let _ = core_tx.send(CoreEvent::PreviewAttachment {
                                    account_id: account_for_preview.clone(),
                                    mailbox_id: mailbox_for_preview.clone(),
                                    message_id: message_for_preview.clone(),
                                    section: section_for_preview.clone(),
                                    filename: filename_for_preview.clone(),
                                    content_type: content_type_for_preview.clone(),
                                    encoding,
                                    size_hint,
                                });
                            },
                            if matches!(status, DownloadStatus::InProgress { .. }) {
                                "Loading…"
                            } else {
                                "Preview"
                            }
                        }
                    }
                    button {
                        class: "attachment-download-btn",
                        disabled: any_busy,
                        onclick: move |_| {
                            let _ = core_tx.send(attachment_download_event(
                                &account_id,
                                &mailbox_id,
                                &message_id,
                                &row_for_click,
                            ));
                        },
                        if matches!(status, DownloadStatus::InProgress { .. }) {
                            "Downloading…"
                        } else if matches!(status, DownloadStatus::Queued) {
                            "Waiting…"
                        } else if matches!(status, DownloadStatus::Finished) {
                            "Done"
                        } else {
                            "Download"
                        }
                    }
                }
            }

            if show_progress {
                div {
                    class: "attachment-progress-track",
                    div {
                        class: "attachment-progress-bar",
                        style: "width: {progress_pct}%",
                    }
                }
            }
        }
    }
}

#[component]
fn AttachmentPreviewDialog() -> Element {
    let ctx = use_context::<AppContext>();
    let Some(preview) = ctx.attachment_preview.read().clone() else {
        return rsx! {};
    };
    let kind = preview_kind(&preview.content_type);
    let filename = preview.filename.clone();
    let url = preview.object_url.clone();
    rsx! {
        div {
            class: "attachment-preview-backdrop",
            onclick: {
                let ctx = ctx.clone();
                move |_| ctx.close_attachment_preview()
            },
            div {
                class: "ui-dialog attachment-preview-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_label: "Preview {filename}",
                tabindex: "0",
                onclick: move |evt| evt.stop_propagation(),
                onkeydown: {
                    let ctx = ctx.clone();
                    move |evt: KeyboardEvent| {
                        if evt.key() == Key::Escape {
                            evt.prevent_default();
                            ctx.close_attachment_preview();
                        }
                    }
                },
                div {
                    class: "ui-dialog-head",
                    h2 { class: "ui-dialog-title", "{filename}" }
                    IconButton {
                        class: "flat ui-icon-btn",
                        title: "Close",
                        size: 20,
                        icon: IconKind::XMark,
                        onclick: {
                            let ctx = ctx.clone();
                            move |_| ctx.close_attachment_preview()
                        },
                    }
                }
                div {
                    class: "attachment-preview-body",
                    match kind {
                        Some(PreviewKind::Image) => rsx! {
                            img {
                                class: "attachment-preview-image",
                                src: "{url}",
                                alt: "{filename}",
                                referrerpolicy: "no-referrer",
                            }
                        },
                        Some(PreviewKind::Pdf) => rsx! {
                            iframe {
                                class: "attachment-preview-frame",
                                src: "{url}",
                                title: "{filename}",
                            }
                        },
                        None => rsx! {
                            p { class: "attachment-preview-unsupported", "This type cannot be previewed." }
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::FolderId;

    fn row(section: &str, filename: &str) -> AttachmentRow {
        AttachmentRow {
            section: section.into(),
            filename: filename.into(),
            content_type: "application/pdf".into(),
            size: 12,
            wire_size: Some(16),
            encoding: TransferEncoding::Base64,
            description: None,
        }
    }

    fn ids() -> (AccountId, MailboxId, MessageId) {
        let account = AccountId::new("acct-1");
        let mailbox = MailboxId::from("INBOX".to_string());
        let message = MessageId::new(FolderId::new("INBOX"), "42");
        (account, mailbox, message)
    }

    #[test]
    fn save_all_queues_one_event_per_row_in_order() {
        let (account, mailbox, message) = ids();
        let rows = vec![row("2", "a.pdf"), row("3", "b.txt")];
        let events = save_all_events(&account, &mailbox, &message, &rows);
        assert_eq!(events.len(), 2);
        match &events[0] {
            CoreEvent::DownloadAttachment {
                account_id,
                section,
                filename,
                ..
            } => {
                assert_eq!(account_id, &account);
                assert_eq!(section, "2");
                assert_eq!(filename, "a.pdf");
            }
            _ => panic!("expected DownloadAttachment"),
        }
        match &events[1] {
            CoreEvent::DownloadAttachment {
                account_id,
                section,
                filename,
                ..
            } => {
                assert_eq!(account_id, &account);
                assert_eq!(section, "3");
                assert_eq!(filename, "b.txt");
            }
            _ => panic!("expected DownloadAttachment"),
        }
    }

    #[test]
    fn mark_pending_keeps_save_all_busy_after_first_finishes() {
        let rows = vec![row("2", "a.pdf"), row("3", "b.txt")];
        let mut status = HashMap::new();
        mark_pending_downloads(&mut status, &rows);
        status.insert("2".into(), DownloadStatus::Finished);
        assert!(any_download_busy(&status, ["2", "3"]));
        assert!(!matches!(
            status.get("3"),
            Some(DownloadStatus::InProgress { .. })
        ));
    }

    #[test]
    fn any_busy_only_for_listed_sections() {
        let mut status = HashMap::new();
        status.insert(
            "2".into(),
            DownloadStatus::InProgress {
                received: 1,
                total: Some(10),
            },
        );
        assert!(any_download_busy(&status, ["2"]));
        assert!(!any_download_busy(&status, ["3"]));
        status.insert("2".into(), DownloadStatus::Queued);
        assert!(any_download_busy(&status, ["2"]));
        status.insert("2".into(), DownloadStatus::Finished);
        assert!(!any_download_busy(&status, ["2"]));
        status.insert("2".into(), DownloadStatus::Error("boom".into()));
        assert!(!any_download_busy(&status, ["2"]));
    }
}
