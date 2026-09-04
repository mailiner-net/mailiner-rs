//! Local snooze overlay: hide a folder row until a preset time.

use chrono::{DateTime, Duration, Utc};

use crate::ui_prefs::SnoozedMessage;

/// Preset hide-until durations (no custom picker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozePreset {
    OneHour,
    ThreeHours,
    Tomorrow,
    NextWeek,
}

impl SnoozePreset {
    pub const ALL: [Self; 4] = [
        Self::OneHour,
        Self::ThreeHours,
        Self::Tomorrow,
        Self::NextWeek,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OneHour => "1 hour",
            Self::ThreeHours => "3 hours",
            Self::Tomorrow => "Tomorrow",
            Self::NextWeek => "Next week",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::OneHour => "snooze.one_hour",
            Self::ThreeHours => "snooze.three_hours",
            Self::Tomorrow => "snooze.tomorrow",
            Self::NextWeek => "snooze.next_week",
        }
    }

    pub fn until(self, now: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::OneHour => now + Duration::hours(1),
            Self::ThreeHours => now + Duration::hours(3),
            Self::Tomorrow => now + Duration::days(1),
            Self::NextWeek => now + Duration::weeks(1),
        }
    }
}

/// Short clock time for toasts (`03 Sep, 14:05` in English).
pub fn format_until(until: DateTime<Utc>) -> String {
    crate::i18n::format_datetime(&until, crate::i18n::DateStyle::Snooze)
}

/// UIDs whose snooze is still in the future.
pub fn active_uids(entries: &[SnoozedMessage], now: DateTime<Utc>) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.until > now)
        .map(|entry| entry.uid.clone())
        .collect()
}

/// Whether every listed UID is currently snoozed (empty → treat as off).
pub fn all_snoozed(
    uids: impl IntoIterator<Item = impl AsRef<str>>,
    entries: &[SnoozedMessage],
    now: DateTime<Utc>,
) -> bool {
    let active = active_uids(entries, now);
    let mut any = false;
    let mut all_on = true;
    for uid in uids {
        any = true;
        all_on &= active.iter().any(|s| s == uid.as_ref());
    }
    any && all_on
}

/// Pull expired rows out of `entries`. Returns the expired set.
pub fn take_expired(entries: &mut Vec<SnoozedMessage>, now: DateTime<Utc>) -> Vec<SnoozedMessage> {
    let mut expired = Vec::new();
    entries.retain(|entry| {
        if entry.until <= now {
            expired.push(entry.clone());
            false
        } else {
            true
        }
    });
    expired
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("ts")
    }

    fn entry(uid: &str, until: DateTime<Utc>) -> SnoozedMessage {
        SnoozedMessage {
            uid: uid.into(),
            until,
            subject: String::new(),
        }
    }

    #[test]
    fn presets_are_in_the_future() {
        let now = ts(1_700_000_000);
        assert_eq!(SnoozePreset::OneHour.until(now), now + Duration::hours(1));
        assert_eq!(
            SnoozePreset::ThreeHours.until(now),
            now + Duration::hours(3)
        );
        assert_eq!(SnoozePreset::Tomorrow.until(now), now + Duration::days(1));
        assert_eq!(SnoozePreset::NextWeek.until(now), now + Duration::weeks(1));
        assert_eq!(SnoozePreset::OneHour.label(), "1 hour");
        assert_eq!(SnoozePreset::ALL.len(), 4);
    }

    #[test]
    fn active_uids_skip_expired() {
        let now = ts(100);
        let entries = vec![entry("a", ts(99)), entry("b", ts(100)), entry("c", ts(101))];
        assert_eq!(active_uids(&entries, now), vec!["c"]);
    }

    #[test]
    fn all_snoozed_requires_every_uid() {
        let now = ts(100);
        let entries = vec![entry("1", ts(200)), entry("2", ts(200))];
        assert!(all_snoozed(["1", "2"], &entries, now));
        assert!(!all_snoozed(["1", "3"], &entries, now));
        assert!(!all_snoozed(std::iter::empty::<&str>(), &entries, now));
        assert!(!all_snoozed(["1"], &[entry("1", ts(50))], now));
    }

    #[test]
    fn take_expired_splits_the_vec() {
        let now = ts(100);
        let mut entries = vec![
            entry("old", ts(50)),
            entry("live", ts(150)),
            entry("edge", ts(100)),
        ];
        let expired = take_expired(&mut entries, now);
        assert_eq!(
            expired.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(),
            vec!["old", "edge"]
        );
        assert_eq!(
            entries.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(),
            vec!["live"]
        );
    }

    #[test]
    fn format_until_is_day_and_clock() {
        let until = ts(1_704_067_200);
        let text = format_until(until);
        assert!(text.contains(','), "{text}");
        assert!(text.contains(':'), "{text}");
    }
}
