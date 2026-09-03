//! One in-flight SMTP operation **per account**.
//!
//! v1 used a single global slot. Concurrent sends across accounts are now
//! allowed: each account may have at most one SMTP op (a write-ahead send or
//! Test SMTP). A second send for a busy account stays `Queued`. Test SMTP for
//! a busy account is rejected. There is no extra global cap (the outbox is
//! already limited to [`crate::outbox_store::MAX_OUTBOX_ITEMS`] items).
//!
//! Cancel signals the spawned task but **keeps the slot** until
//! `SmtpFinished`. Drain must not start a second DATA for the same rfc822
//! while the first attempt may still succeed.

use std::collections::HashMap;

use futures_channel::oneshot;
use mailiner_core::MessageId;

use crate::account::AccountId;
use crate::outbox_store::OutboxId;

pub struct InFlightSmtp {
    pub account_id: AccountId,
    pub generation: u64,
    pub cancel_tx: Option<oneshot::Sender<()>>,
    pub outbox_id: Option<OutboxId>,
    pub is_test: bool,
    /// Source message to mark `\Answered` if the outbox row cannot be re-read.
    pub reply_source: Option<MessageId>,
}

/// Live SMTP tasks keyed by account. `core_loop` is the sole writer.
#[derive(Default)]
pub struct SmtpInflight {
    slots: HashMap<AccountId, InFlightSmtp>,
    next_generation: u64,
}

impl SmtpInflight {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_busy(&self, account_id: &AccountId) -> bool {
        self.slots.contains_key(account_id)
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn contains_generation(&self, generation: u64) -> bool {
        self.slots
            .values()
            .any(|flight| flight.generation == generation)
    }

    pub fn busy_account_ids(&self) -> impl Iterator<Item = &AccountId> {
        self.slots.keys()
    }

    pub fn alloc_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }

    /// Record a new slot. The account must not already be busy.
    pub fn insert(&mut self, flight: InFlightSmtp) {
        debug_assert!(
            !self.slots.contains_key(&flight.account_id),
            "account already has an in-flight SMTP op"
        );
        self.slots.insert(flight.account_id.clone(), flight);
    }

    /// Cancel and drop every slot (sign-out). Later `SmtpFinished` must not persist.
    pub fn take_all(&mut self) {
        for flight in self.slots.values_mut() {
            signal_cancel(flight);
        }
        self.slots.clear();
        self.next_generation = self.next_generation.wrapping_add(1);
    }

    pub fn take_by_generation(&mut self, generation: u64) -> Option<InFlightSmtp> {
        let account_id = self
            .slots
            .iter()
            .find(|(_, flight)| flight.generation == generation)
            .map(|(id, _)| id.clone())?;
        self.slots.remove(&account_id)
    }

    /// Fire cancel for this account. Keeps the slot until `SmtpFinished`.
    pub fn cancel_for_account(&mut self, account_id: &AccountId) -> bool {
        let Some(flight) = self.slots.get_mut(account_id) else {
            return false;
        };
        signal_cancel(flight);
        self.next_generation = self.next_generation.wrapping_add(1);
        true
    }

    /// Fire cancel if this outbox row is the in-flight send. Keeps the slot.
    pub fn cancel_by_outbox_id(&mut self, id: &OutboxId) -> bool {
        let Some(flight) = self
            .slots
            .values_mut()
            .find(|flight| flight.outbox_id.as_ref() == Some(id))
        else {
            return false;
        };
        signal_cancel(flight);
        self.next_generation = self.next_generation.wrapping_add(1);
        true
    }
}

fn signal_cancel(flight: &mut InFlightSmtp) {
    if let Some(tx) = flight.cancel_tx.take() {
        let _ = tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flight(
        account: &str,
        generation: u64,
        outbox: Option<&str>,
    ) -> (InFlightSmtp, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (
            InFlightSmtp {
                account_id: AccountId::new(account),
                generation,
                cancel_tx: Some(tx),
                outbox_id: outbox.map(|s| OutboxId(s.to_string())),
                is_test: false,
                reply_source: None,
            },
            rx,
        )
    }

    #[test]
    fn two_accounts_can_be_in_flight() {
        let mut set = SmtpInflight::new();
        let (a, _) = flight("a", set.alloc_generation(), Some("oa"));
        let (b, _) = flight("b", set.alloc_generation(), Some("ob"));
        set.insert(a);
        set.insert(b);
        assert!(set.is_busy(&AccountId::new("a")));
        assert!(set.is_busy(&AccountId::new("b")));
        assert!(!set.is_busy(&AccountId::new("c")));
        let mut ids: Vec<_> = set
            .busy_account_ids()
            .map(|id| id.as_str().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn cancel_keeps_slot_until_finished() {
        let mut set = SmtpInflight::new();
        let generation = set.alloc_generation();
        let (a, mut rx) = flight("a", generation, Some("oa"));
        set.insert(a);
        assert!(set.cancel_for_account(&AccountId::new("a")));
        assert!(set.is_busy(&AccountId::new("a")));
        assert_eq!(rx.try_recv(), Ok(Some(())));
        let taken = set.take_by_generation(generation).expect("slot");
        assert_eq!(taken.account_id.as_str(), "a");
        assert!(!set.is_busy(&AccountId::new("a")));
    }

    #[test]
    fn take_by_generation_leaves_other_accounts() {
        let mut set = SmtpInflight::new();
        let ga = set.alloc_generation();
        let gb = set.alloc_generation();
        let (a, _) = flight("a", ga, Some("oa"));
        let (b, _) = flight("b", gb, Some("ob"));
        set.insert(a);
        set.insert(b);
        assert!(set.take_by_generation(ga).is_some());
        assert!(!set.is_busy(&AccountId::new("a")));
        assert!(set.is_busy(&AccountId::new("b")));
        assert!(set.take_by_generation(ga).is_none());
        assert!(set.take_by_generation(gb).is_some());
    }

    #[test]
    fn cancel_by_outbox_id_does_not_touch_other_slots() {
        let mut set = SmtpInflight::new();
        let (a, mut rx_a) = flight("a", set.alloc_generation(), Some("oa"));
        let (b, mut rx_b) = flight("b", set.alloc_generation(), Some("ob"));
        set.insert(a);
        set.insert(b);
        assert!(set.cancel_by_outbox_id(&OutboxId("oa".into())));
        assert!(set.is_busy(&AccountId::new("a")));
        assert!(set.is_busy(&AccountId::new("b")));
        assert_eq!(rx_a.try_recv(), Ok(Some(())));
        assert_eq!(rx_b.try_recv(), Ok(None));
        assert!(!set.cancel_by_outbox_id(&OutboxId("missing".into())));
    }
}
