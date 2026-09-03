//! RFC 2087 GETQUOTA / GETQUOTAROOT → [`MailboxQuota`].

use async_imap::types::{Quota, QuotaResourceName};
use mailiner_core::MailboxQuota;

/// STORAGE is reported in KiB (1024-octet units).
const KIB: u64 = 1024;

/// First finite STORAGE resource from GETQUOTA / GETQUOTAROOT untagged replies.
pub fn storage_quota(quotas: &[Quota]) -> Option<MailboxQuota> {
    quotas.iter().find_map(|q| {
        q.resources.iter().find_map(|r| match r.name {
            QuotaResourceName::Storage if r.limit > 0 => Some(MailboxQuota {
                used_bytes: r.usage.saturating_mul(KIB),
                limit_bytes: r.limit.saturating_mul(KIB),
            }),
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_imap::types::QuotaResource;

    fn storage(usage: u64, limit: u64) -> Quota {
        Quota {
            root_name: String::new(),
            resources: vec![QuotaResource {
                name: QuotaResourceName::Storage,
                usage,
                limit,
            }],
        }
    }

    fn message(usage: u64, limit: u64) -> Quota {
        Quota {
            root_name: "INBOX".into(),
            resources: vec![QuotaResource {
                name: QuotaResourceName::Message,
                usage,
                limit,
            }],
        }
    }

    #[test]
    fn picks_storage_and_converts_kib() {
        let quota = storage_quota(&[storage(10, 512)]).unwrap();
        assert_eq!(quota.used_bytes, 10 * 1024);
        assert_eq!(quota.limit_bytes, 512 * 1024);
        assert_eq!(quota.display(), "10 KB of 512 KB");
    }

    #[test]
    fn rfc_example_userquota() {
        let quota = storage_quota(&[Quota {
            root_name: "Userquota".into(),
            resources: vec![QuotaResource {
                name: QuotaResourceName::Storage,
                usage: 4855,
                limit: 48576,
            }],
        }])
        .unwrap();
        assert_eq!(quota.used_bytes, 4855 * 1024);
        assert_eq!(quota.limit_bytes, 48576 * 1024);
    }

    #[test]
    fn hides_zero_limit_and_message_only() {
        assert!(storage_quota(&[storage(0, 0)]).is_none());
        assert!(storage_quota(&[message(3, 100)]).is_none());
        assert!(storage_quota(&[]).is_none());
    }

    #[test]
    fn skips_message_then_takes_storage() {
        let quota = storage_quota(&[message(1, 10), storage(1200, 15 * 1024 * 1024)]).unwrap();
        assert_eq!(quota.display(), "1.2 MB of 15 GB");
    }

    #[test]
    fn mixed_resources_on_one_root() {
        let quota = storage_quota(&[Quota {
            root_name: String::new(),
            resources: vec![
                QuotaResource {
                    name: QuotaResourceName::Message,
                    usage: 12,
                    limit: 1000,
                },
                QuotaResource {
                    name: QuotaResourceName::Storage,
                    usage: 1024,
                    limit: 2048,
                },
            ],
        }])
        .unwrap();
        assert_eq!(quota.used_bytes, 1024 * 1024);
        assert_eq!(quota.limit_bytes, 2048 * 1024);
        assert_eq!(quota.display(), "1 MB of 2 MB");
    }
}
