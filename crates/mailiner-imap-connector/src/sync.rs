//! RFC 7162 CONDSTORE / QRESYNC incremental folder index.
//!
//! After LOGIN the connector ENABLE's `QRESYNC` (implies CONDSTORE) or
//! `CONDSTORE` when advertised. The first SELECT still runs SEARCH/SORT; later
//! opens reuse the stored UID set + HIGHESTMODSEQ so flag changes, new UIDs,
//! and expunges can be applied without `UID SEARCH ALL`.

use std::collections::HashSet;
use std::fmt::Debug;

use async_imap::types::{Capabilities, Capability};
use async_imap::{types::Mailbox, Session};
use futures::StreamExt;
use imap_proto::{AttributeValue, MailboxDatum, Response, ResponseCode, Status};
use mailiner_core::{MessageListFilter, MessageSort};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{quote_mailbox, ImapError};

/// Advertised / enabled RFC 7162 extensions on this session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SyncCaps {
    /// Server listed `CONDSTORE` or `QRESYNC`.
    pub condstore: bool,
    /// `ENABLE QRESYNC` succeeded (required before `SELECT (QRESYNC …)`).
    pub qresync: bool,
}

/// Per-folder snapshot used to skip `SEARCH ALL` on the next SELECT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderSyncState {
    pub folder: String,
    pub uidvalidity: u32,
    pub highestmodseq: u64,
    pub sort: MessageSort,
    pub filter: MessageListFilter,
    pub search: String,
    pub exists: usize,
    pub uids: Vec<u32>,
    pub unread: Option<usize>,
    pub folder_unread: Option<usize>,
}

impl FolderSyncState {
    pub(crate) fn matches_view(
        &self,
        folder: &str,
        sort: MessageSort,
        filter: MessageListFilter,
        search: &str,
    ) -> bool {
        self.folder == folder && self.sort == sort && self.filter == filter && self.search == search
    }

    /// Incremental refresh only for an unfiltered view (list UIDs == mailbox).
    pub(crate) fn can_refresh(
        &self,
        folder: &str,
        sort: MessageSort,
        filter: MessageListFilter,
        search: &str,
    ) -> bool {
        self.matches_view(folder, sort, filter, search) && filter.is_empty() && search.is_empty()
    }
}

/// UID + flags from `CHANGEDSINCE` / QRESYNC `FETCH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FlagUpdate {
    pub uid: u32,
    pub is_read: bool,
    pub is_flagged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectOutcome {
    pub mailbox: Mailbox,
    pub vanished: Vec<u32>,
    pub flag_updates: Vec<FlagUpdate>,
    /// True when this SELECT used `QRESYNC` (CHANGEDSINCE is redundant).
    pub from_qresync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectMode<'a> {
    Plain,
    Condstore,
    Qresync {
        uidvalidity: u32,
        modseq: u64,
        known: &'a [u32],
    },
}

/// True when post-auth `CAPABILITY` lists `name` (case-insensitive atom).
pub(crate) fn advertises_atom(caps: &Capabilities, name: &str) -> bool {
    caps.has_str(name)
        || caps.iter().any(|c| match c {
            Capability::Atom(atom) => atom.eq_ignore_ascii_case(name),
            _ => false,
        })
}

pub(crate) fn sync_caps_from(caps: &Capabilities) -> (bool, bool) {
    let qresync = advertises_atom(caps, "QRESYNC");
    let condstore = qresync || advertises_atom(caps, "CONDSTORE");
    (condstore, qresync)
}

/// RFC 3501 sequence-set (`1:3,5,8:10`). Empty input → empty string.
pub(crate) fn compact_uid_set(uids: &[u32]) -> String {
    let mut sorted: Vec<u32> = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let mut start = sorted[0];
    let mut prev = sorted[0];
    for &uid in &sorted[1..] {
        if uid == prev.saturating_add(1) {
            prev = uid;
            continue;
        }
        push_uid_run(&mut out, start, prev);
        start = uid;
        prev = uid;
    }
    push_uid_run(&mut out, start, prev);
    out
}

fn push_uid_run(out: &mut String, start: u32, end: u32) {
    if !out.is_empty() {
        out.push(',');
    }
    if start == end {
        out.push_str(&start.to_string());
    } else {
        out.push_str(&start.to_string());
        out.push(':');
        out.push_str(&end.to_string());
    }
}

pub(crate) fn expand_uid_ranges(ranges: &[std::ops::RangeInclusive<u32>]) -> Vec<u32> {
    let mut out = Vec::new();
    for range in ranges {
        let start = *range.start();
        let end = *range.end();
        if end < start {
            continue;
        }
        // Guard against a degenerate 1:* vanish expanding to billions of UIDs.
        if end.saturating_sub(start) > 1_000_000 {
            continue;
        }
        out.extend(start..=end);
    }
    out
}

pub(crate) fn select_command(folder: &str, mode: &SelectMode<'_>) -> String {
    let name = quote_mailbox(folder);
    match mode {
        SelectMode::Plain => format!("SELECT {name}"),
        SelectMode::Condstore => format!("SELECT {name} (CONDSTORE)"),
        SelectMode::Qresync {
            uidvalidity,
            modseq,
            known,
        } => {
            let set = compact_uid_set(known);
            if set.is_empty() {
                format!("SELECT {name} (QRESYNC ({uidvalidity} {modseq}))")
            } else {
                format!("SELECT {name} (QRESYNC ({uidvalidity} {modseq} {set}))")
            }
        }
    }
}

/// ENABLE QRESYNC (preferred) or CONDSTORE. `NO`/`BAD` leaves the session usable.
pub(crate) async fn enable_sync_extensions<S>(
    session: &mut Session<S>,
    want_qresync: bool,
    want_condstore: bool,
) -> SyncCaps
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let command = if want_qresync {
        "ENABLE QRESYNC"
    } else if want_condstore {
        "ENABLE CONDSTORE"
    } else {
        return SyncCaps::default();
    };
    match session.run_command_and_check_ok(command).await {
        Ok(()) => {
            if want_qresync {
                tracing::info!("IMAP ENABLE QRESYNC");
                SyncCaps {
                    condstore: true,
                    qresync: true,
                }
            } else {
                tracing::info!("IMAP ENABLE CONDSTORE");
                SyncCaps {
                    condstore: true,
                    qresync: false,
                }
            }
        }
        Err(e) => {
            tracing::warn!("{command} failed ({e}); incremental sync will use SELECT (CONDSTORE) if advertised");
            SyncCaps {
                condstore: want_condstore || want_qresync,
                qresync: false,
            }
        }
    }
}

pub(crate) async fn select_sync<S>(
    session: &mut Session<S>,
    folder: &str,
    mode: SelectMode<'_>,
) -> Result<SelectOutcome, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    match mode {
        SelectMode::Plain => {
            let mailbox = session
                .select(folder)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;
            Ok(SelectOutcome {
                mailbox,
                vanished: Vec::new(),
                flag_updates: Vec::new(),
                from_qresync: false,
            })
        }
        SelectMode::Condstore => {
            let mailbox = session
                .select_condstore(folder)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;
            Ok(SelectOutcome {
                mailbox,
                vanished: Vec::new(),
                flag_updates: Vec::new(),
                from_qresync: false,
            })
        }
        SelectMode::Qresync { .. } => select_qresync(session, folder, &mode).await,
    }
}

async fn select_qresync<S>(
    session: &mut Session<S>,
    folder: &str,
    mode: &SelectMode<'_>,
) -> Result<SelectOutcome, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let command = select_command(folder, mode);
    let tag = session
        .run_command(&command)
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to SELECT: {e}")))?;
    let mut mailbox = Mailbox::default();
    let mut vanished = Vec::new();
    let mut flag_updates = Vec::new();
    loop {
        let resp = session
            .read_response()
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to read SELECT: {e}")))?
            .ok_or_else(|| ImapError::Imap("IMAP connection closed during SELECT".into()))?;
        match resp.parsed() {
            Response::Done {
                tag: done_tag,
                status,
                code,
                information,
            } if done_tag == &tag => {
                apply_select_code(&mut mailbox, code);
                return match status {
                    Status::Ok => Ok(SelectOutcome {
                        mailbox,
                        vanished,
                        flag_updates,
                        from_qresync: true,
                    }),
                    _ => Err(ImapError::Imap(format!(
                        "SELECT failed: {}",
                        information.as_deref().unwrap_or("error")
                    ))),
                };
            }
            Response::Data { status, code, .. } if *status == Status::Ok => {
                apply_select_code(&mut mailbox, code);
            }
            Response::MailboxData(MailboxDatum::Exists(n)) => mailbox.exists = *n,
            Response::MailboxData(MailboxDatum::Recent(n)) => mailbox.recent = *n,
            Response::Vanished { uids, .. } => {
                vanished.extend(expand_uid_ranges(uids));
            }
            Response::Fetch(_, attrs) => {
                if let Some(update) = flag_update_from_attrs(attrs) {
                    flag_updates.push(update);
                }
            }
            _ => {}
        }
    }
}

fn apply_select_code(mailbox: &mut Mailbox, code: &Option<ResponseCode<'_>>) {
    match code {
        Some(ResponseCode::UidValidity(uid)) => mailbox.uid_validity = Some(*uid),
        Some(ResponseCode::UidNext(next)) => mailbox.uid_next = Some(*next),
        Some(ResponseCode::HighestModSeq(ms)) => mailbox.highest_modseq = Some(*ms),
        Some(ResponseCode::Unseen(n)) => mailbox.unseen = Some(*n),
        _ => {}
    }
}

pub(crate) fn flag_update_from_attrs(attrs: &[AttributeValue<'_>]) -> Option<FlagUpdate> {
    let mut uid = None;
    let mut is_read = false;
    let mut is_flagged = false;
    let mut saw_flags = false;
    for attr in attrs {
        match attr {
            AttributeValue::Uid(id) => uid = Some(*id),
            AttributeValue::Flags(flags) => {
                saw_flags = true;
                for flag in flags {
                    if flag.eq_ignore_ascii_case("\\Seen") {
                        is_read = true;
                    } else if flag.eq_ignore_ascii_case("\\Flagged") {
                        is_flagged = true;
                    }
                }
            }
            _ => {}
        }
    }
    uid.filter(|_| saw_flags).map(|uid| FlagUpdate {
        uid,
        is_read,
        is_flagged,
    })
}

pub(crate) async fn fetch_changed_since<S>(
    session: &mut Session<S>,
    modseq: u64,
) -> Result<Vec<FlagUpdate>, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let query = format!("(FLAGS UID) (CHANGEDSINCE {modseq})");
    let mut fetch = session
        .uid_fetch("1:*", &query)
        .await
        .map_err(|e| ImapError::Imap(format!("UID FETCH CHANGEDSINCE: {e}")))?;
    let mut updates = Vec::new();
    while let Some(result) = fetch.next().await {
        let fetch = result.map_err(|e| ImapError::Imap(format!("UID FETCH CHANGEDSINCE: {e}")))?;
        let Some(uid) = fetch.uid else {
            continue;
        };
        let flags: Vec<_> = fetch.flags().collect();
        let parsed = crate::parse_flags(flags.into_iter());
        updates.push(FlagUpdate {
            uid,
            is_read: parsed.is_read,
            is_flagged: parsed.is_flagged,
        });
    }
    Ok(updates)
}

pub(crate) async fn search_uid_set<S>(
    session: &mut Session<S>,
    uids: &[u32],
) -> Result<HashSet<u32>, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let mut still = HashSet::new();
    for chunk in uid_search_chunks(uids) {
        let query = format!("UID {chunk}");
        let set = session
            .uid_search(&query)
            .await
            .map_err(|e| ImapError::Imap(format!("UID SEARCH {query}: {e}")))?;
        still.extend(set);
    }
    Ok(still)
}

pub(crate) async fn search_uids_from<S>(
    session: &mut Session<S>,
    from_uid: u32,
) -> Result<Vec<u32>, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let query = format!("UID {from_uid}:*");
    let set = session
        .uid_search(&query)
        .await
        .map_err(|e| ImapError::Imap(format!("UID SEARCH {query}: {e}")))?;
    Ok(crate::sort::arrival_uid_order(set))
}

/// Split a compact UID set so each `UID SEARCH` stays within typical line limits.
fn uid_search_chunks(uids: &[u32]) -> Vec<String> {
    const MAX_CHARS: usize = 800;
    let whole = compact_uid_set(uids);
    if whole.len() <= MAX_CHARS {
        return if whole.is_empty() {
            Vec::new()
        } else {
            vec![whole]
        };
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < uids.len() {
        let mut end = start + 1;
        while end <= uids.len() {
            let candidate = compact_uid_set(&uids[start..end]);
            if candidate.len() > MAX_CHARS && end > start + 1 {
                end -= 1;
                break;
            }
            if end == uids.len() {
                break;
            }
            end += 1;
        }
        let chunk = compact_uid_set(&uids[start..end]);
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        start = end;
    }
    chunks
}

/// Merge vanished UIDs, CHANGEDSINCE/QRESYNC flag updates, and newly discovered UIDs.
///
/// Returns `None` when the caller should rebuild with SEARCH/SORT (e.g. a
/// Date/Size/Sender list gained messages).
pub(crate) fn merge_uid_list(
    prior: &FolderSyncState,
    _exists: usize,
    vanished: &[u32],
    updates: &[FlagUpdate],
    extra_new: &[u32],
) -> Option<Vec<u32>> {
    let vanished: HashSet<u32> = vanished.iter().copied().collect();
    let mut uids: Vec<u32> = prior
        .uids
        .iter()
        .copied()
        .filter(|u| !vanished.contains(u))
        .collect();

    let prior_set: HashSet<u32> = prior.uids.iter().copied().collect();
    let mut new_uids: Vec<u32> = updates
        .iter()
        .map(|u| u.uid)
        .filter(|u| !prior_set.contains(u) && !vanished.contains(u))
        .collect();
    for uid in extra_new {
        if !prior_set.contains(uid) && !vanished.contains(uid) && !new_uids.contains(uid) {
            new_uids.push(*uid);
        }
    }

    if !new_uids.is_empty()
        && matches!(
            prior.sort,
            MessageSort::Date | MessageSort::Size | MessageSort::Sender
        )
    {
        return None;
    }

    apply_flag_filter(&mut uids, updates, prior.filter);

    match prior.sort {
        MessageSort::Arrival | MessageSort::Date | MessageSort::Size | MessageSort::Sender => {
            insert_uids_desc(&mut uids, &new_uids);
        }
        MessageSort::Unread => {
            let update_by_uid: std::collections::HashMap<u32, FlagUpdate> =
                updates.iter().map(|u| (u.uid, *u)).collect();
            let mut unread = prior.unread.unwrap_or(0).min(uids.len());
            for uid in &prior.uids {
                if vanished.contains(uid) {
                    continue;
                }
                if let Some(update) = update_by_uid.get(uid) {
                    crate::sort::move_uid_for_seen_flag(
                        &mut uids,
                        &mut unread,
                        *uid,
                        update.is_read,
                    );
                }
            }
            for uid in &new_uids {
                let is_read = update_by_uid.get(uid).is_some_and(|u| u.is_read);
                if !uids.contains(uid) {
                    crate::sort::move_uid_for_seen_flag(&mut uids, &mut unread, *uid, is_read);
                    if !uids.contains(uid) {
                        // `move_uid_for_seen_flag` no-ops when the UID is absent.
                        let dest = if is_read {
                            unread..uids.len()
                        } else {
                            0..unread
                        };
                        let pos = uids[dest.clone()]
                            .iter()
                            .position(|&u| u < *uid)
                            .map(|i| dest.start + i)
                            .unwrap_or(dest.end);
                        uids.insert(pos, *uid);
                    }
                }
            }
        }
    }

    Some(uids)
}

fn apply_flag_filter(uids: &mut Vec<u32>, updates: &[FlagUpdate], filter: MessageListFilter) {
    if filter.is_empty() {
        return;
    }
    for update in updates {
        let keep = filter.matches(update.is_read, update.is_flagged, false);
        if !keep {
            uids.retain(|u| *u != update.uid);
        }
    }
}

fn insert_uids_desc(uids: &mut Vec<u32>, new_uids: &[u32]) {
    for &uid in new_uids {
        if uids.contains(&uid) {
            continue;
        }
        let pos = uids.iter().position(|&u| u < uid).unwrap_or(uids.len());
        uids.insert(pos, uid);
    }
}

/// Decide whether EXISTS implies missing expunges / new UIDs after the delta.
pub(crate) fn exists_gap(
    prior_len: usize,
    vanished: usize,
    new: usize,
    exists: usize,
) -> ExistsGap {
    let expected = prior_len.saturating_sub(vanished).saturating_add(new);
    match exists.cmp(&expected) {
        std::cmp::Ordering::Equal => ExistsGap::Match,
        std::cmp::Ordering::Less => ExistsGap::MissingExpunge,
        std::cmp::Ordering::Greater => ExistsGap::MissingNew,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistsGap {
    Match,
    MissingExpunge,
    MissingNew,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_uid_set_collapses_runs() {
        assert_eq!(compact_uid_set(&[]), "");
        assert_eq!(compact_uid_set(&[10]), "10");
        assert_eq!(compact_uid_set(&[1, 2, 3, 5, 7, 8, 9]), "1:3,5,7:9");
        assert_eq!(compact_uid_set(&[9, 1, 3, 2, 1]), "1:3,9");
    }

    #[test]
    fn select_command_formats_modes() {
        assert_eq!(
            select_command("INBOX", &SelectMode::Plain),
            r#"SELECT "INBOX""#
        );
        assert_eq!(
            select_command("INBOX", &SelectMode::Condstore),
            r#"SELECT "INBOX" (CONDSTORE)"#
        );
        assert_eq!(
            select_command(
                "INBOX",
                &SelectMode::Qresync {
                    uidvalidity: 7,
                    modseq: 100,
                    known: &[1, 2, 3, 5],
                },
            ),
            r#"SELECT "INBOX" (QRESYNC (7 100 1:3,5))"#
        );
        assert_eq!(
            select_command(
                "INBOX",
                &SelectMode::Qresync {
                    uidvalidity: 7,
                    modseq: 100,
                    known: &[],
                },
            ),
            r#"SELECT "INBOX" (QRESYNC (7 100))"#
        );
    }

    #[test]
    fn expand_vanished_ranges() {
        assert_eq!(expand_uid_ranges(&[1..=3, 8..=8]), vec![1, 2, 3, 8]);
        assert!(expand_uid_ranges(&[1..=2_000_000]).is_empty());
    }

    fn prior(uids: &[u32], sort: MessageSort) -> FolderSyncState {
        FolderSyncState {
            folder: "INBOX".into(),
            uidvalidity: 1,
            highestmodseq: 10,
            sort,
            filter: MessageListFilter::default(),
            search: String::new(),
            exists: uids.len(),
            uids: uids.to_vec(),
            unread: None,
            folder_unread: None,
        }
    }

    #[test]
    fn merge_arrival_adds_and_vanishes() {
        let state = prior(&[10, 8, 5, 3], MessageSort::Arrival);
        let merged = merge_uid_list(
            &state,
            5,
            &[8],
            &[FlagUpdate {
                uid: 12,
                is_read: false,
                is_flagged: false,
            }],
            &[11],
        )
        .unwrap();
        assert_eq!(merged, vec![12, 11, 10, 5, 3]);
    }

    #[test]
    fn merge_date_with_new_uids_needs_rebuild() {
        let state = prior(&[10, 8, 5], MessageSort::Date);
        assert!(merge_uid_list(
            &state,
            4,
            &[],
            &[FlagUpdate {
                uid: 12,
                is_read: false,
                is_flagged: false,
            }],
            &[],
        )
        .is_none());
    }

    #[test]
    fn merge_date_expunge_only_keeps_order() {
        let state = prior(&[10, 8, 5], MessageSort::Date);
        let merged = merge_uid_list(&state, 2, &[8], &[], &[]).unwrap();
        assert_eq!(merged, vec![10, 5]);
    }

    #[test]
    fn exists_gap_detects_missing() {
        assert_eq!(exists_gap(3, 0, 1, 4), ExistsGap::Match);
        assert_eq!(exists_gap(3, 0, 0, 2), ExistsGap::MissingExpunge);
        assert_eq!(exists_gap(3, 0, 0, 4), ExistsGap::MissingNew);
    }

    #[test]
    fn can_refresh_requires_unfiltered_same_view() {
        let state = prior(&[1, 2], MessageSort::Arrival);
        assert!(state.can_refresh(
            "INBOX",
            MessageSort::Arrival,
            MessageListFilter::default(),
            ""
        ));
        assert!(!state.can_refresh(
            "Sent",
            MessageSort::Arrival,
            MessageListFilter::default(),
            ""
        ));
        assert!(!state.can_refresh(
            "INBOX",
            MessageSort::Arrival,
            MessageListFilter {
                unread: true,
                ..MessageListFilter::default()
            },
            "",
        ));
        assert!(!state.can_refresh(
            "INBOX",
            MessageSort::Arrival,
            MessageListFilter::default(),
            "ada"
        ));
        assert!(!state.can_refresh("INBOX", MessageSort::Date, MessageListFilter::default(), ""));
    }

    #[test]
    fn uid_search_chunks_split_long_sets() {
        let uids: Vec<u32> = (1..=2000).step_by(2).collect();
        let chunks = uid_search_chunks(&uids);
        assert!(
            chunks.len() > 1,
            "len={} first={:?}",
            chunks.len(),
            chunks.first()
        );
        assert!(chunks.iter().all(|c| c.len() <= 800));
        let rebuilt: usize = chunks.iter().map(|c| c.split(',').count()).sum();
        assert_eq!(rebuilt, uids.len());
    }
}
