//! IMAP IDLE (RFC 2177) and NOOP polling for live mailbox updates.

use std::fmt::Debug;
use std::future::Future;

use async_imap::extensions::idle::Handle;
use async_imap::types::UnsolicitedResponse;
use async_imap::Session;
use futures::future::{self, Either};
use futures::StreamExt;
use imap_proto::{MailboxDatum, Response};

use crate::{ImapError, ImapIo};
use tokio::io::{AsyncRead, AsyncWrite};

/// What changed on the selected mailbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MailboxChange {
    pub exists: bool,
    pub expunge: bool,
    pub fetch: bool,
}

impl MailboxChange {
    pub fn exists() -> Self {
        Self {
            exists: true,
            ..Self::default()
        }
    }

    pub fn expunge() -> Self {
        Self {
            expunge: true,
            ..Self::default()
        }
    }

    pub fn fetch() -> Self {
        Self {
            fetch: true,
            ..Self::default()
        }
    }

    pub fn is_empty(self) -> bool {
        !self.exists && !self.expunge && !self.fetch
    }

    pub fn merge(&mut self, other: Self) {
        self.exists |= other.exists;
        self.expunge |= other.expunge;
        self.fetch |= other.fetch;
    }
}

/// Result of [`crate::ImapConnector::watch_mailbox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailboxWatchOutcome {
    /// EXISTS / EXPUNGE / FETCH (or VANISHED) arrived.
    Changed(MailboxChange),
    /// Caller cancelled so another IMAP command can run.
    Cancelled,
    /// IDLE keepalive window or NOOP interval elapsed with no mailbox change.
    TimedOut,
}

impl MailboxWatchOutcome {
    pub fn needs_refresh(self) -> bool {
        matches!(self, Self::Changed(c) if !c.is_empty())
    }
}

/// Classify an untagged IMAP response as a mailbox change.
pub fn mailbox_change_from_response(resp: &Response<'_>) -> Option<MailboxChange> {
    match resp {
        Response::MailboxData(MailboxDatum::Exists(_)) => Some(MailboxChange::exists()),
        Response::Expunge(_) | Response::Vanished { .. } => Some(MailboxChange::expunge()),
        Response::Fetch(_, _) => Some(MailboxChange::fetch()),
        _ => None,
    }
}

/// Classify a queued unsolicited response (after NOOP / SELECT).
pub fn mailbox_change_from_unsolicited(resp: &UnsolicitedResponse) -> MailboxChange {
    match resp {
        UnsolicitedResponse::Exists(_) => MailboxChange::exists(),
        UnsolicitedResponse::Expunge(_) => MailboxChange::expunge(),
        UnsolicitedResponse::Other(data) => {
            mailbox_change_from_response(data.parsed()).unwrap_or_default()
        }
        UnsolicitedResponse::Recent(_) | UnsolicitedResponse::Status { .. } => {
            MailboxChange::default()
        }
    }
}

pub(crate) fn drain_unsolicited<S>(session: &mut Session<ImapIo<S>>) -> MailboxChange
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let mut change = MailboxChange::default();
    while let Ok(resp) = session.unsolicited_responses.try_recv() {
        change.merge(mailbox_change_from_unsolicited(&resp));
    }
    change
}

pub(crate) enum WatchFinish<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    Ready {
        session: Session<ImapIo<S>>,
        outcome: MailboxWatchOutcome,
    },
    Lost(ImapError),
}

pub(crate) async fn run_watch<S, C, T>(
    mut session: Session<ImapIo<S>>,
    folder_id: &str,
    use_idle: bool,
    cancel: C,
    timeout: T,
) -> WatchFinish<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
    C: Future<Output = ()>,
    T: Future<Output = ()>,
{
    if let Err(e) = session.select(folder_id).await {
        return WatchFinish::Lost(ImapError::Imap(format!(
            "Failed to select folder for watch: {e}"
        )));
    }
    // SELECT always reports EXISTS; that is not a live change.
    let _ = drain_unsolicited(&mut session);

    if use_idle {
        watch_idle(session, cancel, timeout).await
    } else {
        watch_noop(session, cancel, timeout).await
    }
}

async fn watch_idle<S, C, T>(session: Session<ImapIo<S>>, cancel: C, timeout: T) -> WatchFinish<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
    C: Future<Output = ()>,
    T: Future<Output = ()>,
{
    let mut idle = session.idle();
    if let Err(e) = idle.init().await {
        return WatchFinish::Lost(ImapError::Imap(format!("IDLE failed: {e}")));
    }

    let outcome = idle_wait_outcome(&mut idle, cancel, timeout).await;
    match idle.done().await {
        Ok(session) => match outcome {
            Ok(outcome) => WatchFinish::Ready { session, outcome },
            Err(e) => WatchFinish::Lost(e),
        },
        Err(e) => WatchFinish::Lost(ImapError::Imap(format!("IDLE DONE failed: {e}"))),
    }
}

async fn idle_wait_outcome<S, C, T>(
    idle: &mut Handle<ImapIo<S>>,
    cancel: C,
    timeout: T,
) -> Result<MailboxWatchOutcome, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
    C: Future<Output = ()>,
    T: Future<Output = ()>,
{
    let wait_change = async {
        loop {
            match idle.next().await {
                Some(Ok(data)) => {
                    if let Some(change) = mailbox_change_from_response(data.parsed()) {
                        return Ok(MailboxWatchOutcome::Changed(change));
                    }
                }
                Some(Err(e)) => {
                    return Err(ImapError::Imap(format!("IDLE read failed: {e}")));
                }
                None => {
                    return Err(ImapError::Connection(
                        "IMAP connection closed during IDLE".into(),
                    ));
                }
            }
        }
    };

    futures::pin_mut!(wait_change);
    futures::pin_mut!(cancel);
    futures::pin_mut!(timeout);
    match future::select(wait_change, future::select(cancel, timeout)).await {
        Either::Left((result, _)) => result,
        Either::Right((Either::Left(((), _)), _)) => Ok(MailboxWatchOutcome::Cancelled),
        Either::Right((Either::Right(((), _)), _)) => Ok(MailboxWatchOutcome::TimedOut),
    }
}

async fn watch_noop<S, C, T>(
    mut session: Session<ImapIo<S>>,
    cancel: C,
    timeout: T,
) -> WatchFinish<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
    C: Future<Output = ()>,
    T: Future<Output = ()>,
{
    futures::pin_mut!(cancel);
    futures::pin_mut!(timeout);
    match future::select(cancel, timeout).await {
        Either::Left(((), _)) => WatchFinish::Ready {
            session,
            outcome: MailboxWatchOutcome::Cancelled,
        },
        Either::Right(((), _)) => match session.noop().await {
            Ok(()) => {
                let change = drain_unsolicited(&mut session);
                let outcome = if change.is_empty() {
                    MailboxWatchOutcome::TimedOut
                } else {
                    MailboxWatchOutcome::Changed(change)
                };
                WatchFinish::Ready { session, outcome }
            }
            Err(e) => WatchFinish::Lost(ImapError::Imap(format!("NOOP failed: {e}"))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imap_proto::Response;

    fn parse(line: &str) -> Response<'static> {
        let owned = line.to_string();
        let parsed = imap_proto::Response::from_bytes(owned.as_bytes())
            .unwrap_or_else(|e| panic!("parse {line:?}: {e:?}"))
            .1
            .into_owned();
        parsed
    }

    #[test]
    fn classifies_exists_expunge_fetch() {
        assert_eq!(
            mailbox_change_from_response(&parse("* 12 EXISTS\r\n")),
            Some(MailboxChange::exists())
        );
        assert_eq!(
            mailbox_change_from_response(&parse("* 3 EXPUNGE\r\n")),
            Some(MailboxChange::expunge())
        );
        assert_eq!(
            mailbox_change_from_response(&parse("* 1 FETCH (FLAGS (\\Seen))\r\n")),
            Some(MailboxChange::fetch())
        );
        assert_eq!(
            mailbox_change_from_response(&parse("* VANISHED 1:3\r\n")),
            Some(MailboxChange::expunge())
        );
    }

    #[test]
    fn ignores_keepalives_and_recent() {
        assert_eq!(
            mailbox_change_from_response(&parse("* OK Still here\r\n")),
            None
        );
        assert_eq!(mailbox_change_from_response(&parse("* 1 RECENT\r\n")), None);
        assert_eq!(mailbox_change_from_response(&parse("+ idling\r\n")), None);
    }

    #[test]
    fn unsolicited_exists_and_expunge() {
        assert_eq!(
            mailbox_change_from_unsolicited(&UnsolicitedResponse::Exists(4)),
            MailboxChange::exists()
        );
        assert_eq!(
            mailbox_change_from_unsolicited(&UnsolicitedResponse::Expunge(2)),
            MailboxChange::expunge()
        );
        assert_eq!(
            mailbox_change_from_unsolicited(&UnsolicitedResponse::Recent(1)),
            MailboxChange::default()
        );
    }

    #[test]
    fn change_merge_and_refresh() {
        let mut c = MailboxChange::exists();
        c.merge(MailboxChange::fetch());
        assert!(c.exists && c.fetch && !c.expunge);
        assert!(MailboxWatchOutcome::Changed(c).needs_refresh());
        assert!(!MailboxWatchOutcome::TimedOut.needs_refresh());
        assert!(!MailboxWatchOutcome::Cancelled.needs_refresh());
        assert!(!MailboxWatchOutcome::Changed(MailboxChange::default()).needs_refresh());
    }
}
