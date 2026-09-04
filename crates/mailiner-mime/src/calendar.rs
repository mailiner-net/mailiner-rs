//! Minimal iCalendar (RFC 5545) reader for invite cards.
//!
//! Only the fields the viewer shows: title, time, organizer. No recurrence
//! expansion, no timezone database — named `TZID` values are displayed as-is.

use std::borrow::Cow;
use std::collections::HashSet;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use mailiner_core::models::{MessageContent, MessagePart};

const MAX_INVITES: usize = 8;

/// One `VEVENT` (or a filename fallback when the part is calendar but empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInvite {
    pub method: Option<String>,
    pub status: Option<String>,
    pub uid: Option<String>,
    pub summary: Option<String>,
    pub organizer: Option<String>,
    pub location: Option<String>,
    pub start: Option<CalendarDateTime>,
    pub end: Option<CalendarDateTime>,
}

/// DATE or DATE-TIME from `DTSTART` / `DTEND`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarDateTime {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
    /// `Z` suffix — UTC. Floating local times and `TZID` stay wall-clock.
    pub utc: bool,
    pub tzid: Option<String>,
}

impl CalendarInvite {
    pub fn title(&self) -> String {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Calendar invitation".into())
    }

    pub fn kind_label(&self) -> &'static str {
        let method = self.method.as_deref().unwrap_or("");
        let status = self.status.as_deref().unwrap_or("");
        if method.eq_ignore_ascii_case("CANCEL") || status.eq_ignore_ascii_case("CANCELLED") {
            "Cancelled"
        } else if method.eq_ignore_ascii_case("REPLY") {
            "Reply"
        } else if method.eq_ignore_ascii_case("COUNTER") {
            "Counter-proposal"
        } else if method.eq_ignore_ascii_case("PUBLISH") {
            "Event"
        } else if status.eq_ignore_ascii_case("TENTATIVE") {
            "Tentative invitation"
        } else {
            "Invitation"
        }
    }

    /// Human-readable time range, or `None` when no start is present.
    pub fn time_label(&self) -> Option<String> {
        format_time_range(self.start.as_ref()?, self.end.as_ref())
    }

    fn dedupe_key(&self) -> String {
        if let Some(uid) = self.uid.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return uid.to_string();
        }
        format!(
            "{}|{}",
            self.summary.as_deref().unwrap_or(""),
            self.start.as_ref().map(|s| s.raw_key()).unwrap_or_default()
        )
    }
}

impl CalendarDateTime {
    fn raw_key(&self) -> String {
        match self.time {
            Some(t) => format!("{}T{}{}", self.date, t, if self.utc { "Z" } else { "" }),
            None => self.date.to_string(),
        }
    }

    fn is_date_only(&self) -> bool {
        self.time.is_none()
    }
}

/// Parse every `VEVENT` in an iCalendar payload.
pub fn parse_calendar(ics: &str) -> Vec<CalendarInvite> {
    let unfolded = unfold(ics);
    let mut method: Option<String> = None;
    let mut invites = Vec::new();
    let mut current: Option<EventBuilder> = None;

    for line in unfolded.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let Some(prop) = Property::parse(line) else {
            continue;
        };
        if prop.name.eq_ignore_ascii_case("BEGIN") {
            if prop.value.eq_ignore_ascii_case("VEVENT") {
                current = Some(EventBuilder::default());
            }
            continue;
        }
        if prop.name.eq_ignore_ascii_case("END") {
            if prop.value.eq_ignore_ascii_case("VEVENT") {
                if let Some(builder) = current.take() {
                    invites.push(builder.finish(method.clone()));
                }
            }
            continue;
        }
        if current.is_none() && prop.name.eq_ignore_ascii_case("METHOD") {
            method = Some(prop.value.to_ascii_uppercase());
            continue;
        }
        if let Some(builder) = current.as_mut() {
            builder.apply(&prop);
        }
    }
    if let Some(builder) = current.take() {
        invites.push(builder.finish(method));
    }
    invites
}

/// Invites from calendar parts in `nested_in` (`None` = outer message).
pub fn invites_from_parts(parts: &[MessagePart], nested_in: Option<&str>) -> Vec<CalendarInvite> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for part in parts {
        if !part.in_scope(nested_in) || !part.is_calendar() {
            continue;
        }
        let Some(ics) = part_ics(part) else {
            continue;
        };
        let parsed = parse_calendar(&ics);
        if parsed.is_empty() {
            let fallback = CalendarInvite {
                method: None,
                status: None,
                uid: None,
                summary: part
                    .filename
                    .as_deref()
                    .map(strip_ics_filename)
                    .filter(|s| !s.is_empty()),
                organizer: None,
                location: None,
                start: None,
                end: None,
            };
            push_unique(&mut out, &mut seen, fallback);
        } else {
            for invite in parsed {
                push_unique(&mut out, &mut seen, invite);
            }
        }
        if out.len() >= MAX_INVITES {
            break;
        }
    }
    out.truncate(MAX_INVITES);
    out
}

fn part_ics(part: &MessagePart) -> Option<Cow<'_, str>> {
    match &part.content {
        MessageContent::Text(s) if !s.trim().is_empty() => Some(Cow::Borrowed(s.as_str())),
        MessageContent::Binary(b) => {
            let s = std::str::from_utf8(b).ok()?;
            if s.trim().is_empty() {
                None
            } else {
                Some(Cow::Borrowed(s))
            }
        }
        _ => None,
    }
}

fn strip_ics_filename(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(stem) = lower.strip_suffix(".ics") {
        trimmed[..stem.len()].to_string()
    } else {
        trimmed.to_string()
    }
}

fn push_unique(out: &mut Vec<CalendarInvite>, seen: &mut HashSet<String>, invite: CalendarInvite) {
    if out.len() >= MAX_INVITES {
        return;
    }
    let key = invite.dedupe_key();
    if seen.insert(key) {
        out.push(invite);
    }
}

/// RFC 5545 line unfolding (leading space/tab continues the previous line).
fn unfold(ics: &str) -> String {
    let mut out = String::with_capacity(ics.len());
    let mut first = true;
    for raw in ics.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if first {
            out.push_str(line);
            first = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            out.push_str(rest);
        } else {
            out.push('\n');
            out.push_str(line);
        }
    }
    out
}

struct Property<'a> {
    name: &'a str,
    params: Vec<(String, String)>,
    value: &'a str,
}

impl<'a> Property<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let colon = find_unquoted(line, ':')?;
        let (left, rest) = line.split_at(colon);
        let value = &rest[1..];
        let mut segs = left.split(';');
        let name = segs.next()?.trim();
        if name.is_empty() {
            return None;
        }
        let params = segs
            .filter_map(|p| {
                let (k, v) = p.split_once('=')?;
                Some((k.trim().to_ascii_uppercase(), unquote(v.trim())))
            })
            .collect();
        Some(Self {
            name,
            params,
            value,
        })
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn find_unquoted(s: &str, needle: char) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == needle && !quoted {
            return Some(i);
        }
    }
    None
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    t.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(t)
        .to_string()
}

fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Default)]
struct EventBuilder {
    uid: Option<String>,
    summary: Option<String>,
    organizer: Option<String>,
    location: Option<String>,
    status: Option<String>,
    start: Option<CalendarDateTime>,
    end: Option<CalendarDateTime>,
    duration: Option<String>,
}

impl EventBuilder {
    fn apply(&mut self, prop: &Property<'_>) {
        if prop.name.eq_ignore_ascii_case("UID") {
            self.uid = Some(prop.value.trim().to_string());
        } else if prop.name.eq_ignore_ascii_case("SUMMARY") {
            self.summary = Some(unescape_text(prop.value));
        } else if prop.name.eq_ignore_ascii_case("LOCATION") {
            self.location = Some(unescape_text(prop.value));
        } else if prop.name.eq_ignore_ascii_case("STATUS") {
            self.status = Some(prop.value.trim().to_ascii_uppercase());
        } else if prop.name.eq_ignore_ascii_case("ORGANIZER") {
            self.organizer = Some(format_organizer(prop));
        } else if prop.name.eq_ignore_ascii_case("DTSTART") {
            self.start = parse_ics_datetime(prop);
        } else if prop.name.eq_ignore_ascii_case("DTEND") {
            self.end = parse_ics_datetime(prop);
        } else if prop.name.eq_ignore_ascii_case("DURATION") {
            self.duration = Some(prop.value.trim().to_string());
        }
    }

    fn finish(self, method: Option<String>) -> CalendarInvite {
        let end = self.end.or_else(|| {
            self.start
                .as_ref()
                .zip(self.duration.as_deref())
                .and_then(|(start, dur)| add_duration(start, dur))
        });
        CalendarInvite {
            method,
            status: self.status,
            uid: self.uid,
            summary: self.summary,
            organizer: self.organizer,
            location: self.location,
            start: self.start,
            end,
        }
    }
}

fn format_organizer(prop: &Property<'_>) -> String {
    let cn = prop
        .param("CN")
        .map(unescape_text)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let email = cal_address_email(prop.value);
    match (cn, email) {
        (Some(name), Some(mail)) if !name.eq_ignore_ascii_case(&mail) => {
            format!("{name} <{mail}>")
        }
        (Some(name), _) => name,
        (None, Some(mail)) => mail,
        (None, None) => unescape_text(prop.value).trim().to_string(),
    }
}

fn cal_address_email(value: &str) -> Option<String> {
    let v = value.trim();
    let addr = v
        .strip_prefix("mailto:")
        .or_else(|| v.strip_prefix("MAILTO:"))
        .unwrap_or(v)
        .trim();
    if addr.is_empty() || !addr.contains('@') {
        None
    } else {
        Some(addr.to_string())
    }
}

fn parse_ics_datetime(prop: &Property<'_>) -> Option<CalendarDateTime> {
    let value_date = prop
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
    let tzid = prop
        .param("TZID")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    parse_ics_datetime_value(prop.value.trim(), value_date, tzid)
}

fn parse_ics_datetime_value(
    raw: &str,
    force_date: bool,
    tzid: Option<String>,
) -> Option<CalendarDateTime> {
    let compact: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if compact.len() == 8 || force_date && compact.len() >= 8 {
        let date = parse_ics_date(&compact[..8])?;
        return Some(CalendarDateTime {
            date,
            time: None,
            utc: false,
            tzid: None,
        });
    }
    // YYYYMMDDTHHMMSS or YYYYMMDDTHHMMSSZ, optional fractional seconds ignored.
    if compact.len() < 15 || compact.as_bytes().get(8) != Some(&b'T') {
        return None;
    }
    let date = parse_ics_date(&compact[..8])?;
    let time = parse_ics_time(&compact[9..])?;
    let utc = compact.ends_with('Z') || compact.ends_with('z');
    Some(CalendarDateTime {
        date,
        time: Some(time),
        utc,
        tzid: if utc { None } else { tzid },
    })
}

fn parse_ics_date(s: &str) -> Option<NaiveDate> {
    if s.len() < 8 {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[4..6].parse().ok()?;
    let d: u32 = s[6..8].parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d)
}

fn parse_ics_time(s: &str) -> Option<NaiveTime> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 6 {
        return None;
    }
    let h: u32 = digits[0..2].parse().ok()?;
    let min: u32 = digits[2..4].parse().ok()?;
    let sec: u32 = digits[4..6].parse().ok()?;
    NaiveTime::from_hms_opt(h, min, sec)
}

fn add_duration(start: &CalendarDateTime, raw: &str) -> Option<CalendarDateTime> {
    let dur = parse_duration(raw)?;
    if start.is_date_only() {
        let date = start.date.checked_add_signed(dur)?;
        return Some(CalendarDateTime {
            date,
            time: None,
            utc: false,
            tzid: None,
        });
    }
    let naive = NaiveDateTime::new(start.date, start.time?);
    let next = naive.checked_add_signed(dur)?;
    Some(CalendarDateTime {
        date: next.date(),
        time: Some(next.time()),
        utc: start.utc,
        tzid: start.tzid.clone(),
    })
}

/// RFC 5545 `dur-value`: `[sign]P[nW | [nD][T[nH][nM][nS]]]`.
fn parse_duration(raw: &str) -> Option<Duration> {
    let s = raw.trim();
    let (neg, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else {
        (false, s.strip_prefix('+').unwrap_or(s))
    };
    let rest = rest.strip_prefix(['P', 'p'])?;
    let mut weeks = 0i64;
    let mut days = 0i64;
    let mut hours = 0i64;
    let mut mins = 0i64;
    let mut secs = 0i64;
    if let Some(w) = rest.strip_suffix(['W', 'w']) {
        weeks = w.parse().ok()?;
    } else {
        let (date, time) = match rest.split_once(['T', 't']) {
            Some((d, t)) => (d, Some(t)),
            None => (rest, None),
        };
        let mut num = String::new();
        for c in date.chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else if c == 'D' || c == 'd' {
                days = num.parse().ok()?;
                num.clear();
            } else {
                return None;
            }
        }
        if !num.is_empty() {
            return None;
        }
        if let Some(time) = time {
            num.clear();
            for c in time.chars() {
                if c.is_ascii_digit() {
                    num.push(c);
                } else if matches!(c, 'H' | 'h') {
                    hours = num.parse().ok()?;
                    num.clear();
                } else if matches!(c, 'M' | 'm') {
                    mins = num.parse().ok()?;
                    num.clear();
                } else if matches!(c, 'S' | 's') {
                    secs = num.parse().ok()?;
                    num.clear();
                } else {
                    return None;
                }
            }
            if !num.is_empty() {
                return None;
            }
        }
    }
    let mut total = Duration::weeks(weeks)
        + Duration::days(days)
        + Duration::hours(hours)
        + Duration::minutes(mins)
        + Duration::seconds(secs);
    if neg {
        total = -total;
    }
    Some(total)
}

fn format_time_range(start: &CalendarDateTime, end: Option<&CalendarDateTime>) -> Option<String> {
    let start_s = format_datetime(start);
    let Some(end) = end else {
        return Some(if start.is_date_only() {
            format!("{start_s} (all day)")
        } else {
            start_s
        });
    };
    if start.is_date_only() && end.is_date_only() {
        let last = end.date.pred_opt().unwrap_or(end.date);
        if last <= start.date {
            return Some(format!("{start_s} (all day)"));
        }
        return Some(format_date_span(start.date, last));
    }
    if !start.is_date_only()
        && !end.is_date_only()
        && start.date == end.date
        && start.utc == end.utc
        && start.tzid == end.tzid
    {
        let tz = timezone_suffix(start);
        let t0 = start.time.map(|t| t.format("%H:%M").to_string())?;
        let t1 = end.time.map(|t| t.format("%H:%M").to_string())?;
        return Some(format!("{}, {t0}–{t1}{tz}", start.date.format("%d %b %Y")));
    }
    Some(format!("{start_s} – {}", format_datetime(end)))
}

fn format_datetime(dt: &CalendarDateTime) -> String {
    match dt.time {
        None => dt.date.format("%d %b %Y").to_string(),
        Some(t) => format!(
            "{}, {}{}",
            dt.date.format("%d %b %Y"),
            t.format("%H:%M"),
            timezone_suffix(dt)
        ),
    }
}

fn format_date_span(start: NaiveDate, end: NaiveDate) -> String {
    if start.year() == end.year() && start.month() == end.month() {
        return format!("{}–{}", start.format("%d"), end.format("%d %b %Y"));
    }
    if start.year() == end.year() {
        return format!("{} – {}", start.format("%d %b"), end.format("%d %b %Y"));
    }
    format!("{} – {}", start.format("%d %b %Y"), end.format("%d %b %Y"))
}

fn timezone_suffix(dt: &CalendarDateTime) -> String {
    if dt.utc {
        " UTC".into()
    } else if let Some(tz) = dt.tzid.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        format!(" ({tz})")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
    use mailiner_core::models::{PartKind, TransferEncoding};

    const SAMPLE: &str = "\
BEGIN:VCALENDAR\r\n\
METHOD:REQUEST\r\n\
BEGIN:VEVENT\r\n\
UID:meet-1@example.com\r\n\
SUMMARY:Design review\r\n\
DTSTART:20260415T100000Z\r\n\
DTEND:20260415T110000Z\r\n\
ORGANIZER;CN=Ada Lovelace:mailto:ada@example.com\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_request_title_time_organizer() {
        let invites = parse_calendar(SAMPLE);
        assert_eq!(invites.len(), 1);
        let ev = &invites[0];
        assert_eq!(ev.title(), "Design review");
        assert_eq!(ev.kind_label(), "Invitation");
        assert_eq!(
            ev.organizer.as_deref(),
            Some("Ada Lovelace <ada@example.com>")
        );
        assert_eq!(
            ev.time_label().as_deref(),
            Some("15 Apr 2026, 10:00–11:00 UTC")
        );
    }

    #[test]
    fn unfolds_and_unescapes_summary() {
        let ics = "\
BEGIN:VCALENDAR
BEGIN:VEVENT
SUMMARY:Team\\, offsite\\nDay 1
DTSTART;VALUE=DATE:20260415
END:VEVENT
END:VCALENDAR
";
        let ev = &parse_calendar(ics)[0];
        assert_eq!(ev.summary.as_deref(), Some("Team, offsite\nDay 1"));
        assert_eq!(ev.time_label().as_deref(), Some("15 Apr 2026 (all day)"));
    }

    #[test]
    fn folded_line_joins() {
        let ics = concat!(
            "BEGIN:VCALENDAR\r\n",
            "BEGIN:VEVENT\r\n",
            "SUMMARY:Very long\r\n",
            "  meeting title\r\n",
            "DTSTART:20260415T090000\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        assert_eq!(parse_calendar(ics)[0].title(), "Very long meeting title");
    }

    #[test]
    fn all_day_range_is_exclusive() {
        let ics = "\
BEGIN:VCALENDAR
BEGIN:VEVENT
SUMMARY:Retreat
DTSTART;VALUE=DATE:20260415
DTEND;VALUE=DATE:20260417
END:VEVENT
END:VCALENDAR
";
        assert_eq!(
            parse_calendar(ics)[0].time_label().as_deref(),
            Some("15–16 Apr 2026")
        );
    }

    #[test]
    fn duration_fills_end() {
        let ics = "\
BEGIN:VCALENDAR
BEGIN:VEVENT
SUMMARY:Standup
DTSTART:20260415T090000Z
DURATION:PT30M
END:VEVENT
END:VCALENDAR
";
        assert_eq!(
            parse_calendar(ics)[0].time_label().as_deref(),
            Some("15 Apr 2026, 09:00–09:30 UTC")
        );
    }

    #[test]
    fn tzid_is_shown_not_converted() {
        let ics = "\
BEGIN:VCALENDAR
BEGIN:VEVENT
SUMMARY:Local
DTSTART;TZID=Europe/Prague:20260415T120000
DTEND;TZID=Europe/Prague:20260415T130000
END:VEVENT
END:VCALENDAR
";
        assert_eq!(
            parse_calendar(ics)[0].time_label().as_deref(),
            Some("15 Apr 2026, 12:00–13:00 (Europe/Prague)")
        );
    }

    #[test]
    fn cancel_method_label() {
        let ics = "\
BEGIN:VCALENDAR
METHOD:CANCEL
BEGIN:VEVENT
SUMMARY:Old
DTSTART:20260415T100000Z
STATUS:CANCELLED
END:VEVENT
END:VCALENDAR
";
        assert_eq!(parse_calendar(ics)[0].kind_label(), "Cancelled");
    }

    #[test]
    fn organizer_email_only() {
        let ics = "\
BEGIN:VCALENDAR
BEGIN:VEVENT
ORGANIZER:mailto:ada@example.com
DTSTART:20260415T100000Z
END:VEVENT
END:VCALENDAR
";
        assert_eq!(
            parse_calendar(ics)[0].organizer.as_deref(),
            Some("ada@example.com")
        );
    }

    fn part(kind: PartKind, ct: &str, text: &str, nested_in: Option<&str>) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("cal"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["2".into()],
            kind,
            content_type: ct.into(),
            charset: Some("UTF-8".into()),
            content_id: None,
            description: None,
            filename: Some("invite.ics".into()),
            encoding: TransferEncoding::SevenBit,
            original_size: None,
            size: text.len() as u64,
            is_attachment: true,
            is_hidden: false,
            nested_in: nested_in.map(str::to_string),
            nested_headers: None,
            content: MessageContent::Text(text.into()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn invites_from_parts_dedupes_same_uid() {
        let a = part(PartKind::Calendar, "text/calendar", SAMPLE, None);
        let mut b = part(PartKind::Calendar, "application/ics", SAMPLE, None);
        b.id = MessagePartId::new("cal2");
        b.path = vec!["3".into()];
        let invites = invites_from_parts(&[a, b], None);
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].title(), "Design review");
    }

    #[test]
    fn invites_from_parts_respects_scope() {
        const INNER: &str = "\
BEGIN:VCALENDAR
BEGIN:VEVENT
UID:inner
SUMMARY:Inner
END:VEVENT
END:VCALENDAR
";
        let parts = [
            part(PartKind::Calendar, "text/calendar", SAMPLE, None),
            part(PartKind::Calendar, "text/calendar", INNER, Some("2")),
        ];
        let outer = invites_from_parts(&parts, None);
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].title(), "Design review");
        let inner = invites_from_parts(&parts, Some("2"));
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].title(), "Inner");
    }
}
