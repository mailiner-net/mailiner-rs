//! Attachments bar and per-item download UI.

use std::collections::HashMap;

use dioxus::prelude::*;
use mailiner_core::models::TransferEncoding;

use crate::context::{AppContext, MessageViewState};
use crate::core_event::CoreEvent;
use crate::download::{DownloadStatus, size_to_human};
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

    let (message_id, mailbox_id, rows) = match (view, mailbox) {
        (MessageViewState::Ready { message_id, loaded }, Some(mailbox_id)) => {
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
                return rsx! {};
            }
            (message_id, mailbox_id, rows)
        }
        _ => return rsx! {},
    };

    let count = rows.len();
    let summary = if count == 1 {
        "1 attachment".to_string()
    } else {
        format!("{count} attachments")
    };
    let any_busy = {
        let map = ctx.download_status.read();
        any_download_in_progress(&map, rows.iter().map(|r| r.section.as_str()))
    };
    let save_all_rows = rows.clone();
    let save_all_mailbox = mailbox_id.clone();
    let save_all_message = message_id.clone();

    rsx! {
        footer {
            class: "message-attachments",
            details {
                class: "message-attachments-details",
                summary {
                    class: "message-attachments-summary",
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
                                // Core loop is serial; queued events run one IMAP fetch at a time.
                                // A failure leaves later items queued so remaining files still save.
                                for event in save_all_events(
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
                ul {
                    class: "message-attachments-list",
                    for row in rows {
                        AttachmentItem {
                            key: "{row.section}",
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

fn save_all_events(
    mailbox_id: &MailboxId,
    message_id: &MessageId,
    rows: &[AttachmentRow],
) -> Vec<CoreEvent> {
    rows.iter()
        .map(|row| attachment_download_event(mailbox_id, message_id, row))
        .collect()
}

fn attachment_download_event(
    mailbox_id: &MailboxId,
    message_id: &MessageId,
    row: &AttachmentRow,
) -> CoreEvent {
    CoreEvent::DownloadAttachment {
        mailbox_id: mailbox_id.clone(),
        message_id: message_id.clone(),
        section: row.section.clone(),
        filename: row.filename.clone(),
        content_type: row.content_type.clone(),
        encoding: row.encoding,
        size_hint: row.wire_size,
    }
}

fn any_download_in_progress<'a>(
    status: &HashMap<String, DownloadStatus>,
    sections: impl IntoIterator<Item = &'a str>,
) -> bool {
    sections
        .into_iter()
        .any(|section| matches!(status.get(section), Some(DownloadStatus::InProgress { .. })))
}

#[component]
fn AttachmentItem(message_id: MessageId, mailbox_id: MailboxId, row: AttachmentRow) -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let section = row.section.clone();
    let status = {
        let map = ctx.download_status.read();
        map.get(&section).cloned().unwrap_or(DownloadStatus::Idle)
    };

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

    let busy = matches!(status, DownloadStatus::InProgress { .. });
    let progress_pct = match &status {
        DownloadStatus::InProgress {
            received,
            total: Some(t),
        } if *t > 0 => ((*received as f64 / *t as f64) * 100.0).clamp(0.0, 100.0),
        DownloadStatus::InProgress { .. } => 0.0,
        DownloadStatus::Finished => 100.0,
        _ => 0.0,
    };
    let show_progress = !matches!(status, DownloadStatus::Idle);
    let err_msg = match &status {
        DownloadStatus::Error(e) => Some(e.clone()),
        _ => None,
    };

    let row_for_click = row.clone();

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
                button {
                    class: "attachment-download-btn",
                    disabled: busy,
                    onclick: move |_| {
                        let _ = core_tx.send(attachment_download_event(
                            &mailbox_id,
                            &message_id,
                            &row_for_click,
                        ));
                    },
                    if matches!(status, DownloadStatus::InProgress { .. }) {
                        "Downloading…"
                    } else if matches!(status, DownloadStatus::Finished) {
                        "Done"
                    } else {
                        "Download"
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

    fn ids() -> (MailboxId, MessageId) {
        let mailbox = MailboxId::from("INBOX".to_string());
        let message = MessageId::new(FolderId::new("INBOX"), "42");
        (mailbox, message)
    }

    #[test]
    fn save_all_queues_one_event_per_row_in_order() {
        let (mailbox, message) = ids();
        let rows = vec![row("2", "a.pdf"), row("3", "b.txt")];
        let events = save_all_events(&mailbox, &message, &rows);
        assert_eq!(events.len(), 2);
        match &events[0] {
            CoreEvent::DownloadAttachment {
                section, filename, ..
            } => {
                assert_eq!(section, "2");
                assert_eq!(filename, "a.pdf");
            }
            _ => panic!("expected DownloadAttachment"),
        }
        match &events[1] {
            CoreEvent::DownloadAttachment {
                section, filename, ..
            } => {
                assert_eq!(section, "3");
                assert_eq!(filename, "b.txt");
            }
            _ => panic!("expected DownloadAttachment"),
        }
    }

    #[test]
    fn any_in_progress_only_for_listed_sections() {
        let mut status = HashMap::new();
        status.insert(
            "2".into(),
            DownloadStatus::InProgress {
                received: 1,
                total: Some(10),
            },
        );
        assert!(any_download_in_progress(&status, ["2"]));
        assert!(!any_download_in_progress(&status, ["3"]));
        status.insert("2".into(), DownloadStatus::Finished);
        assert!(!any_download_in_progress(&status, ["2"]));
        status.insert("2".into(), DownloadStatus::Error("boom".into()));
        assert!(!any_download_in_progress(&status, ["2"]));
    }
}
