//! User-visible actions and the toast they produce.
//!
//! Every notice is a [`ToastAction`]. The toast host only asks the action for
//! its message, timeout, optional undo, and optional dismiss-commit — it does
//! not special-case move vs delete vs “Sent”.

use std::sync::Arc;

use crate::account::AccountId;
use crate::mailbox::MailboxId;
use crate::message::{Message, MessageId};

pub const TOAST_TIMEOUT_MS: u32 = 5_000;
pub const TOAST_ERROR_TIMEOUT_MS: u32 = 8_000;
pub const TOAST_TICK_MS: u32 = 50;

/// One toast currently shown (or last shown). `id` changes on every push so a
/// stale timer cannot close a newer notice.
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub action: ToastAction,
}

/// A completed (or about-to-commit) user-visible action.
#[derive(Clone, Debug)]
pub enum ToastAction {
    Info {
        message: String,
    },
    Error {
        message: String,
    },
    Moved {
        dest_label: String,
        undo: MoveUndo,
    },
    Trashed {
        undo: MoveUndo,
    },
    /// Permanent delete. IMAP runs when the toast dismisses unless undone.
    Deleted {
        account_id: AccountId,
        mailbox_id: MailboxId,
        snapshots: Vec<RemovedMessage>,
    },
    Sent,
}

/// Inverse of a move that already succeeded on the server (new dest UIDs).
#[derive(Clone, Debug)]
pub struct MoveUndo {
    pub account_id: AccountId,
    pub from: MailboxId,
    pub to: MailboxId,
    pub dest_ids: Vec<MessageId>,
    pub snapshots: Vec<RemovedMessage>,
}

/// A row taken out of the sparse list, with its newest-first index.
#[derive(Clone, Debug)]
pub struct RemovedMessage {
    pub index: usize,
    pub message: Arc<Message>,
}

/// Inverse work to run when the user clicks Undo.
#[derive(Clone, Debug)]
pub enum UndoRequest {
    ReverseMove(MoveUndo),
    RestoreLocal {
        account_id: AccountId,
        mailbox_id: MailboxId,
        snapshots: Vec<RemovedMessage>,
    },
}

/// Work that still has to run if the toast expires or is dismissed.
#[derive(Clone, Debug)]
pub enum DismissCommit {
    Delete {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
}

impl ToastAction {
    pub fn info(message: impl Into<String>) -> Self {
        Self::Info {
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }

    pub fn moved(dest_label: impl Into<String>, undo: MoveUndo) -> Self {
        Self::Moved {
            dest_label: dest_label.into(),
            undo,
        }
    }

    pub fn trashed(undo: MoveUndo) -> Self {
        Self::Trashed { undo }
    }

    pub fn deleted(
        account_id: AccountId,
        mailbox_id: MailboxId,
        snapshots: Vec<RemovedMessage>,
    ) -> Self {
        Self::Deleted {
            account_id,
            mailbox_id,
            snapshots,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Info { message } | Self::Error { message } => message.clone(),
            Self::Moved { dest_label, .. } => format!("Moved to {dest_label}"),
            Self::Trashed { .. } => "Moved to Trash".into(),
            Self::Deleted { .. } => "Deleted".into(),
            Self::Sent => "Sent".into(),
        }
    }

    pub fn timeout_ms(&self) -> u32 {
        match self {
            Self::Error { .. } => TOAST_ERROR_TIMEOUT_MS,
            _ => TOAST_TIMEOUT_MS,
        }
    }

    /// Central undo hook. `None` means this action cannot be undone.
    pub fn undo(&self) -> Option<UndoRequest> {
        match self {
            Self::Moved { undo, .. } | Self::Trashed { undo } => {
                Some(UndoRequest::ReverseMove(undo.clone()))
            }
            Self::Deleted {
                account_id,
                mailbox_id,
                snapshots,
            } => Some(UndoRequest::RestoreLocal {
                account_id: account_id.clone(),
                mailbox_id: mailbox_id.clone(),
                snapshots: snapshots.clone(),
            }),
            Self::Info { .. } | Self::Error { .. } | Self::Sent => None,
        }
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo().map(|_| "Undo")
    }

    /// IMAP (or other) work held until the toast goes away without Undo.
    pub fn on_dismiss(&self) -> Option<DismissCommit> {
        match self {
            Self::Deleted {
                account_id,
                mailbox_id,
                snapshots,
            } => Some(DismissCommit::Delete {
                account_id: account_id.clone(),
                mailbox_id: mailbox_id.clone(),
                message_ids: snapshots.iter().map(|s| s.message.id.clone()).collect(),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageId;
    use chrono::Utc;
    use mailiner_core::{AccountId, EmailAddr, EmailAddress, Envelope, FolderId};

    fn dummy_msg(id: &str) -> Arc<Message> {
        let now = Utc::now();
        let envelope = Envelope {
            id: mailiner_core::MessageId::new(FolderId::new("INBOX"), id),
            account_id: AccountId::new("a"),
            folder_id: FolderId::new("INBOX"),
            subject: Some("s".into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: None,
                email: Some("a@b.c".into()),
            }])),
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            date: now,
            is_read: true,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            has_attachments: false,
            size: None,
            created_at: now,
            updated_at: now,
        };
        Arc::new(envelope.into())
    }

    #[test]
    fn info_has_no_undo() {
        let a = ToastAction::info("hello");
        assert_eq!(a.message(), "hello");
        assert!(a.undo().is_none());
        assert!(a.on_dismiss().is_none());
    }

    #[test]
    fn moved_pairs_with_reverse_move() {
        let undo = MoveUndo {
            account_id: AccountId::new("a"),
            from: MailboxId::from("Trash".to_string()),
            to: MailboxId::from("INBOX".to_string()),
            dest_ids: vec![MessageId::new(FolderId::new("INBOX"), "9")],
            snapshots: vec![RemovedMessage {
                index: 0,
                message: dummy_msg("1"),
            }],
        };
        let a = ToastAction::moved("Trash", undo);
        assert_eq!(a.message(), "Moved to Trash");
        assert_eq!(a.undo_label(), Some("Undo"));
        assert!(matches!(a.undo(), Some(UndoRequest::ReverseMove(_))));
        assert!(a.on_dismiss().is_none());
    }

    #[test]
    fn deleted_undo_is_local_and_dismiss_commits() {
        let a = ToastAction::deleted(
            AccountId::new("a"),
            MailboxId::from("Trash".to_string()),
            vec![RemovedMessage {
                index: 2,
                message: dummy_msg("4"),
            }],
        );
        assert_eq!(a.message(), "Deleted");
        assert!(matches!(a.undo(), Some(UndoRequest::RestoreLocal { .. })));
        assert!(matches!(a.on_dismiss(), Some(DismissCommit::Delete { .. })));
    }
}
