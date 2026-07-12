//! Attachments bar and per-item download UI.

use dioxus::prelude::*;
use mailiner_core::models::TransferEncoding;

use crate::context::{AppContext, MessageViewState};
use crate::core_event::CoreEvent;
use crate::download::{size_to_human, DownloadStatus};
use crate::mailbox::MailboxId;
use crate::message::MessageId;

#[derive(Clone, PartialEq)]
struct AttachmentRow {
    section: String,
    filename: String,
    content_type: String,
    size: u64,
    encoding: TransferEncoding,
    description: Option<String>,
}

#[component]
pub fn AttachmentsFooter() -> Element {
    let ctx = use_context::<AppContext>();
    let view = ctx.message_view.read().clone();
    let mailbox = ctx.selected_mailbox.read().clone();

    let (message_id, mailbox_id, rows) = match (view, mailbox) {
        (
            MessageViewState::Ready {
                message_id,
                loaded,
            },
            Some(mailbox_id),
        ) => {
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

    rsx! {
        footer {
            class: "message-attachments",
            details {
                class: "message-attachments-details",
                open: true,
                summary {
                    class: "message-attachments-summary",
                    "{summary}"
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

#[component]
fn AttachmentItem(
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

    let section_for_click = row.section.clone();
    let filename = row.filename.clone();
    let content_type = row.content_type.clone();
    let encoding = row.encoding;
    let size_hint = Some(row.size);

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
                        let _ = core_tx.send(CoreEvent::DownloadAttachment {
                            mailbox_id: mailbox_id.clone(),
                            message_id: message_id.clone(),
                            section: section_for_click.clone(),
                            filename: filename.clone(),
                            content_type: content_type.clone(),
                            encoding,
                            size_hint,
                        });
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
