//! Browser notifications and tab unread badge.
//!
//! Desktop alerts are opt-in ([`crate::ui_prefs::load_notify_inbox`]) and stay
//! off until the Notification permission is granted. The tab title always
//! reflects Inbox unread from existing folder counts — no IDLE required.

use std::cell::RefCell;
use std::collections::HashMap;

use mailiner_core::MailboxRole;
use mailiner_core::ids::AccountId;

use crate::mailbox::{MailboxId, MailboxNode, find_mailbox_with_role};

/// Document title with no unread badge.
pub const APP_TITLE: &str = "Mailiner";

/// Last live Inbox unread observed for one account this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxUnreadBaseline {
    pub account_id: AccountId,
    pub mailbox_id: MailboxId,
    pub unread: usize,
}

thread_local! {
    static LAST_INBOX_UNREAD: RefCell<Option<InboxUnreadBaseline>> = const { RefCell::new(None) };
}

/// Browser Notification permission, plus “API missing”.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyPermission {
    Unsupported,
    Prompt,
    Denied,
    Granted,
}

/// How an Inbox unread sample was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxCountEvent {
    /// IMAP `STATUS` / live folder counts. May notify after a baseline exists.
    Remote,
    /// Folder open or a local read/unread change. Baseline only; never notify.
    Local,
}

/// Inbox folder id and its unread count, if the tree has a selectable Inbox.
pub fn inbox_unread(nodes: &HashMap<MailboxId, MailboxNode>) -> Option<(MailboxId, usize)> {
    let id = find_mailbox_with_role(nodes, MailboxRole::Inbox)?;
    let unread = nodes.get(&id)?.unread_count;
    Some((id, unread))
}

/// `(N) Mailiner` when `unread > 0`, otherwise [`APP_TITLE`].
pub fn tab_title(unread: usize) -> String {
    if unread == 0 {
        APP_TITLE.to_string()
    } else {
        format!("({unread}) {APP_TITLE}")
    }
}

/// Body text for a desktop notification about `added` new Inbox messages.
pub fn notification_body(added: usize) -> String {
    if added == 1 {
        "1 new message in Inbox".to_string()
    } else {
        format!("{added} new messages in Inbox")
    }
}

/// Update `last` and return how many new Inbox messages to announce, if any.
///
/// The first sample for an account/Inbox pair only establishes a baseline
/// (existing unread is not “new”). A different account or Inbox is a new
/// baseline. Local events never notify. Remote increases notify only when
/// the count is also above the opened-folder watermark.
pub fn note_inbox_unread(
    last: &mut Option<InboxUnreadBaseline>,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    current: usize,
    acknowledged: usize,
    event: InboxCountEvent,
) -> Option<usize> {
    let prev = last.as_ref().and_then(|b| {
        (b.account_id == *account_id && b.mailbox_id == *mailbox_id).then_some(b.unread)
    });
    *last = Some(InboxUnreadBaseline {
        account_id: account_id.clone(),
        mailbox_id: mailbox_id.clone(),
        unread: current,
    });
    match (event, prev) {
        (_, None) => None,
        (InboxCountEvent::Local, _) => None,
        (InboxCountEvent::Remote, Some(prev)) if current > prev && current > acknowledged => {
            Some(current - prev)
        }
        _ => None,
    }
}

/// Clear the session baseline (mailbox tree tear-down).
pub fn reset_inbox_unread_baseline() {
    LAST_INBOX_UNREAD.with(|cell| *cell.borrow_mut() = None);
}

/// Observe Inbox unread against the session baseline.
pub fn observe_inbox_unread(
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    current: usize,
    acknowledged: usize,
    event: InboxCountEvent,
) -> Option<usize> {
    LAST_INBOX_UNREAD.with(|cell| {
        note_inbox_unread(
            &mut cell.borrow_mut(),
            account_id,
            mailbox_id,
            current,
            acknowledged,
            event,
        )
    })
}

/// Whether a desktop notification should be shown for new Inbox mail.
///
/// Pref off or missing permission → never. While the tab is visible and the
/// user already has Inbox open, the sidebar badge is enough.
pub fn should_show_desktop_notification(
    pref_enabled: bool,
    permission: NotifyPermission,
    document_hidden: bool,
    viewing_inbox: bool,
) -> bool {
    if !pref_enabled || permission != NotifyPermission::Granted {
        return false;
    }
    document_hidden || !viewing_inbox
}

/// Current Notification permission. Host unit tests report [`NotifyPermission::Unsupported`].
pub fn current_permission() -> NotifyPermission {
    #[cfg(target_arch = "wasm32")]
    {
        wasm::current_permission()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        NotifyPermission::Unsupported
    }
}

/// Prompt for Notification permission (must run from a user gesture).
pub async fn request_permission() -> NotifyPermission {
    #[cfg(target_arch = "wasm32")]
    {
        wasm::request_permission().await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        NotifyPermission::Unsupported
    }
}

pub fn is_document_hidden() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        wasm::is_document_hidden()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

/// Best-effort `new Notification(...)`. No-op without permission or the API.
#[cfg(target_arch = "wasm32")]
pub fn show_inbox_notification(added: usize) {
    if added == 0 {
        return;
    }
    wasm::show_inbox_notification(added);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn show_inbox_notification(_added: usize) {}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{NotifyPermission, notification_body};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    fn notification_api_available() -> bool {
        let Some(window) = web_sys::window() else {
            return false;
        };
        js_sys::Reflect::has(&window, &JsValue::from_str("Notification")).unwrap_or(false)
    }

    pub fn current_permission() -> NotifyPermission {
        if !notification_api_available() {
            return NotifyPermission::Unsupported;
        }
        match web_sys::Notification::permission() {
            web_sys::NotificationPermission::Granted => NotifyPermission::Granted,
            web_sys::NotificationPermission::Denied => NotifyPermission::Denied,
            web_sys::NotificationPermission::Default => NotifyPermission::Prompt,
            _ => NotifyPermission::Unsupported,
        }
    }

    pub async fn request_permission() -> NotifyPermission {
        if !notification_api_available() {
            return NotifyPermission::Unsupported;
        }
        match web_sys::Notification::request_permission() {
            Ok(promise) => {
                let _ = JsFuture::from(promise).await;
                current_permission()
            }
            Err(_) => NotifyPermission::Unsupported,
        }
    }

    pub fn is_document_hidden() -> bool {
        web_sys::window()
            .and_then(|w| w.document())
            .map(|d| d.hidden())
            .unwrap_or(false)
    }

    pub fn show_inbox_notification(added: usize) {
        if !notification_api_available() {
            return;
        }
        if current_permission() != NotifyPermission::Granted {
            return;
        }
        let opts = web_sys::NotificationOptions::new();
        opts.set_body(&notification_body(added));
        opts.set_tag("mailiner-inbox");
        opts.set_renotify(true);
        let _ = web_sys::Notification::new_with_options("Mailiner", &opts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::build_mailbox_tree;
    use mailiner_core::{AccountId, Folder, FolderId, MailboxRole};

    fn folder(id: &str, name: &str, role: MailboxRole) -> Folder {
        Folder {
            id: FolderId::new(id),
            account_id: AccountId::new("acc"),
            name: name.to_string(),
            parent_id: None,
            role,
            selectable: true,
            subscribed: true,
        }
    }

    fn acc(id: &str) -> AccountId {
        AccountId::new(id)
    }

    fn inbox() -> MailboxId {
        MailboxId::from("INBOX".to_string())
    }

    fn note(
        last: &mut Option<InboxUnreadBaseline>,
        account: &str,
        current: usize,
        ack: usize,
        event: InboxCountEvent,
    ) -> Option<usize> {
        note_inbox_unread(last, &acc(account), &inbox(), current, ack, event)
    }

    fn unread_of(last: &Option<InboxUnreadBaseline>) -> Option<usize> {
        last.as_ref().map(|b| b.unread)
    }

    #[test]
    fn tab_title_hides_zero() {
        assert_eq!(tab_title(0), "Mailiner");
        assert_eq!(tab_title(1), "(1) Mailiner");
        assert_eq!(tab_title(12), "(12) Mailiner");
    }

    #[test]
    fn notification_body_pluralizes() {
        assert_eq!(notification_body(1), "1 new message in Inbox");
        assert_eq!(notification_body(3), "3 new messages in Inbox");
    }

    #[test]
    fn first_sample_is_baseline() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 4, 0, InboxCountEvent::Remote), None);
        assert_eq!(unread_of(&last), Some(4));
    }

    #[test]
    fn remote_increase_above_ack_notifies() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 4, 4, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "a", 6, 4, InboxCountEvent::Remote), Some(2));
        assert_eq!(unread_of(&last), Some(6));
    }

    #[test]
    fn remote_increase_not_above_ack_is_silent() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 4, 4, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "a", 6, 6, InboxCountEvent::Remote), None);
        assert_eq!(unread_of(&last), Some(6));
    }

    #[test]
    fn remote_same_or_decrease_is_silent() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 5, 0, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "a", 5, 0, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "a", 2, 0, InboxCountEvent::Remote), None);
        assert_eq!(unread_of(&last), Some(2));
    }

    #[test]
    fn local_events_never_notify() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 1, 0, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "a", 8, 0, InboxCountEvent::Local), None);
        assert_eq!(unread_of(&last), Some(8));
    }

    #[test]
    fn decrease_then_increase_notifies_delta() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 5, 5, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "a", 2, 2, InboxCountEvent::Local), None);
        assert_eq!(note(&mut last, "a", 4, 2, InboxCountEvent::Remote), Some(2));
    }

    #[test]
    fn other_account_is_new_baseline() {
        let mut last = None;
        assert_eq!(note(&mut last, "a", 4, 0, InboxCountEvent::Remote), None);
        assert_eq!(note(&mut last, "b", 9, 0, InboxCountEvent::Remote), None);
        assert_eq!(
            note(&mut last, "b", 11, 0, InboxCountEvent::Remote),
            Some(2)
        );
    }

    #[test]
    fn desktop_notify_requires_pref_and_grant() {
        assert!(!should_show_desktop_notification(
            false,
            NotifyPermission::Granted,
            true,
            false,
        ));
        assert!(!should_show_desktop_notification(
            true,
            NotifyPermission::Prompt,
            true,
            false,
        ));
        assert!(!should_show_desktop_notification(
            true,
            NotifyPermission::Denied,
            true,
            false,
        ));
        assert!(should_show_desktop_notification(
            true,
            NotifyPermission::Granted,
            true,
            true,
        ));
    }

    #[test]
    fn desktop_notify_skips_visible_inbox() {
        assert!(!should_show_desktop_notification(
            true,
            NotifyPermission::Granted,
            false,
            true,
        ));
        assert!(should_show_desktop_notification(
            true,
            NotifyPermission::Granted,
            false,
            false,
        ));
    }

    #[test]
    fn inbox_unread_reads_special_use() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", MailboxRole::Inbox),
            folder("Sent", "Sent", MailboxRole::Sent),
        ]);
        let (id, n) = inbox_unread(&nodes).expect("inbox");
        assert_eq!(id.as_str(), "INBOX");
        assert_eq!(n, 0);
    }

    #[test]
    fn observe_and_reset_session_baseline() {
        reset_inbox_unread_baseline();
        let a = acc("a");
        let mb = inbox();
        assert_eq!(
            observe_inbox_unread(&a, &mb, 3, 0, InboxCountEvent::Remote),
            None
        );
        assert_eq!(
            observe_inbox_unread(&a, &mb, 5, 3, InboxCountEvent::Remote),
            Some(2)
        );
        reset_inbox_unread_baseline();
        assert_eq!(
            observe_inbox_unread(&a, &mb, 5, 3, InboxCountEvent::Remote),
            None
        );
        reset_inbox_unread_baseline();
    }
}
