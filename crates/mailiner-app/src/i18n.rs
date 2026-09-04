//! WASM-safe UI translations.
//!
//! English (`en`) is the only complete catalog. Lookups fall back to English,
//! then to the key itself. Dates honor an injectable locale (tests), the
//! selected UI locale, or `navigator.language`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use mailiner_core::MailboxRole;
use serde_json::Value;

const EN_JSON: &str = include_str!("../i18n/en.json");

/// User-selected UI language. English is the only complete locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UiLocale {
    #[default]
    En,
}

impl UiLocale {
    pub const ALL: [Self; 1] = [Self::En];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::En => "en",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let primary = key.split(['-', '_']).next().unwrap_or(key);
        match primary {
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn bcp47(self) -> &'static str {
        self.as_key()
    }

    pub fn native_name(self) -> String {
        match self {
            Self::En => t("locale.en"),
        }
    }
}

thread_local! {
    static LOCALE: Cell<UiLocale> = const { Cell::new(UiLocale::En) };
    static DATE_LOCALE: RefCell<Option<String>> = const { RefCell::new(None) };
    static TEST_CATALOGS: RefCell<HashMap<String, HashMap<String, String>>> =
        RefCell::new(HashMap::new());
    static TEST_LOCALE_TAG: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn flatten_json(value: &Value, prefix: &str, out: &mut HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_json(v, &next, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        other => {
            out.insert(prefix.to_string(), other.to_string());
        }
    }
}

fn parse_catalog(json: &str) -> HashMap<String, String> {
    let value: Value = serde_json::from_str(json).unwrap_or(Value::Object(Default::default()));
    let mut out = HashMap::new();
    flatten_json(&value, "", &mut out);
    out
}

fn en_catalog() -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    EN.get_or_init(|| parse_catalog(EN_JSON))
}

fn catalog_for(locale: UiLocale) -> &'static HashMap<String, String> {
    match locale {
        UiLocale::En => en_catalog(),
    }
}

/// Activate a UI locale for subsequent [`t`] / [`t_args`] lookups.
pub fn set_locale(locale: UiLocale) {
    LOCALE.with(|c| c.set(locale));
    TEST_LOCALE_TAG.with(|c| *c.borrow_mut() = None);
    apply_document_lang(locale);
}

/// Currently selected UI locale (defaults to English).
pub fn current_locale() -> UiLocale {
    LOCALE.with(Cell::get)
}

/// Apply `lang` on `<html>` so the browser can pick spellcheck / voice.
pub fn apply_document_lang(locale: UiLocale) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        {
            let _ = el.set_attribute("lang", locale.bcp47());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = locale;
    }
}

fn active_tag() -> String {
    TEST_LOCALE_TAG
        .with(|c| c.borrow().clone())
        .unwrap_or_else(|| current_locale().as_key().to_string())
}

fn lookup_key(tag: &str, key: &str) -> String {
    if let Some(hit) =
        TEST_CATALOGS.with(|c| c.borrow().get(tag).and_then(|cat| cat.get(key).cloned()))
    {
        return hit;
    }
    if let Some(locale) = UiLocale::from_key(tag)
        && let Some(hit) = catalog_for(locale).get(key)
    {
        return hit.clone();
    }
    if let Some(hit) = en_catalog().get(key) {
        return hit.clone();
    }
    key.to_string()
}

/// Translate `key` in the active locale, falling back to English then the key.
pub fn t(key: &str) -> String {
    lookup_key(&active_tag(), key)
}

/// Translate `key` and replace `{name}` placeholders from `args`.
pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    interpolate(&t(key), args)
}

/// Lookup in an explicit locale tag (selected catalog → English → key).
pub fn lookup(tag: &str, key: &str) -> String {
    lookup_key(tag, key)
}

fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Localized message-list sort option.
pub fn message_sort_label(sort: mailiner_core::MessageSort) -> String {
    t(match sort {
        mailiner_core::MessageSort::Arrival => "sort.arrival",
        mailiner_core::MessageSort::Date => "sort.date",
        mailiner_core::MessageSort::Unread => "sort.unread",
        mailiner_core::MessageSort::Size => "sort.size",
        mailiner_core::MessageSort::Sender => "sort.sender",
    })
}

/// Localized special-use folder title. `None` keeps the server name.
pub fn folder_role_label(role: MailboxRole) -> Option<String> {
    let key = match role {
        MailboxRole::Inbox => "folder.inbox",
        MailboxRole::Archive => "folder.archive",
        MailboxRole::Drafts => "folder.drafts",
        MailboxRole::Sent => "folder.sent",
        MailboxRole::Outbox => "folder.outbox",
        MailboxRole::Trash => "folder.trash",
        MailboxRole::Junk => "folder.junk",
        MailboxRole::Other => return None,
    };
    Some(t(key))
}

/// Display formats used by the mail chrome (list, header, snooze toast).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// `14:05` (same calendar day).
    ListTime,
    /// `03 Sep` (same year).
    ListDay,
    /// `03 Sep 2026` (other year).
    ListDayYear,
    /// `03 Sep 2026, 14:05` (message header).
    Header,
    /// `03 Sep, 14:05` (snooze toast).
    Snooze,
}

/// Override the date locale for the current thread (tests). `None` clears it.
pub fn set_date_locale_override(locale: Option<String>) {
    DATE_LOCALE.with(|c| *c.borrow_mut() = locale);
}

/// Browser `navigator.language` when running on WASM.
pub fn browser_language() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().and_then(|w| w.navigator().language())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// Locale used for date display: test override, then browser, then UI locale.
pub fn effective_date_locale() -> String {
    if let Some(over) = DATE_LOCALE.with(|c| c.borrow().clone()) {
        return over;
    }
    if current_locale() == UiLocale::En
        && let Some(nav) = browser_language()
    {
        return nav;
    }
    current_locale().bcp47().to_string()
}

/// Format `dt` with the effective date locale.
pub fn format_datetime(dt: &DateTime<Utc>, style: DateStyle) -> String {
    format_datetime_in(dt, &effective_date_locale(), style)
}

/// Format `dt` with an explicit BCP-47 locale (deterministic in tests).
pub fn format_datetime_in(dt: &DateTime<Utc>, locale: &str, style: DateStyle) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(formatted) = intl_format(dt, locale, style) {
            return formatted;
        }
    }
    chrono_format(dt, locale, style)
}

fn primary_lang(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or(locale)
}

fn chrono_format(dt: &DateTime<Utc>, locale: &str, style: DateStyle) -> String {
    let lang = primary_lang(locale).to_ascii_lowercase();
    match lang.as_str() {
        "de" => match style {
            DateStyle::ListTime => dt.format("%H:%M").to_string(),
            DateStyle::ListDay => dt.format("%d.%m").to_string(),
            DateStyle::ListDayYear => dt.format("%d.%m.%Y").to_string(),
            DateStyle::Header => dt.format("%d.%m.%Y, %H:%M").to_string(),
            DateStyle::Snooze => dt.format("%d.%m, %H:%M").to_string(),
        },
        "fr" => match style {
            DateStyle::ListTime => dt.format("%H:%M").to_string(),
            DateStyle::ListDay => dt.format("%d/%m").to_string(),
            DateStyle::ListDayYear => dt.format("%d/%m/%Y").to_string(),
            DateStyle::Header => dt.format("%d/%m/%Y, %H:%M").to_string(),
            DateStyle::Snooze => dt.format("%d/%m, %H:%M").to_string(),
        },
        "ja" => match style {
            DateStyle::ListTime => dt.format("%H:%M").to_string(),
            DateStyle::ListDay => dt.format("%m/%d").to_string(),
            DateStyle::ListDayYear => dt.format("%Y/%m/%d").to_string(),
            DateStyle::Header => dt.format("%Y/%m/%d %H:%M").to_string(),
            DateStyle::Snooze => dt.format("%m/%d %H:%M").to_string(),
        },
        _ => match style {
            DateStyle::ListTime => dt.format("%H:%M").to_string(),
            DateStyle::ListDay => dt.format("%d %b").to_string(),
            DateStyle::ListDayYear => dt.format("%d %b %Y").to_string(),
            DateStyle::Header => dt.format("%d %b %Y, %H:%M").to_string(),
            DateStyle::Snooze => dt.format("%d %b, %H:%M").to_string(),
        },
    }
}

#[cfg(target_arch = "wasm32")]
fn intl_format(dt: &DateTime<Utc>, locale: &str, style: DateStyle) -> Option<String> {
    use js_sys::{Array, Intl, Object, Reflect};
    use wasm_bindgen::JsValue;

    let locales = Array::of1(&JsValue::from_str(locale));
    let options = Object::new();
    let set = |k: &str, v: &str| {
        Reflect::set(&options, &JsValue::from_str(k), &JsValue::from_str(v)).ok()
    };
    match style {
        DateStyle::ListTime => {
            set("hour", "2-digit")?;
            set("minute", "2-digit")?;
            set("hourCycle", "h23")?;
        }
        DateStyle::ListDay => {
            set("day", "2-digit")?;
            set("month", "short")?;
        }
        DateStyle::ListDayYear => {
            set("day", "2-digit")?;
            set("month", "short")?;
            set("year", "numeric")?;
        }
        DateStyle::Header => {
            set("day", "2-digit")?;
            set("month", "short")?;
            set("year", "numeric")?;
            set("hour", "2-digit")?;
            set("minute", "2-digit")?;
            set("hourCycle", "h23")?;
        }
        DateStyle::Snooze => {
            set("day", "2-digit")?;
            set("month", "short")?;
            set("hour", "2-digit")?;
            set("minute", "2-digit")?;
            set("hourCycle", "h23")?;
        }
    }
    let fmt = Intl::DateTimeFormat::new(&locales, &options);
    let js_date = js_sys::Date::new(&JsValue::from_f64(dt.timestamp_millis() as f64));
    fmt.format()
        .call1(&JsValue::NULL, &js_date)
        .ok()
        .and_then(|v| v.as_string())
}

/// Format a list-row date (today → time, this year → day, else day+year).
pub fn format_list_date(dt: &DateTime<Utc>, now: &DateTime<Utc>) -> String {
    if dt.date_naive() == now.date_naive() {
        format_datetime(dt, DateStyle::ListTime)
    } else if dt.format("%Y").to_string() == now.format("%Y").to_string() {
        format_datetime(dt, DateStyle::ListDay)
    } else {
        format_datetime(dt, DateStyle::ListDayYear)
    }
}

#[cfg(test)]
pub fn install_test_catalog(tag: &str, pairs: &[(&str, &str)]) {
    let mut map = HashMap::new();
    for (k, v) in pairs {
        map.insert((*k).to_string(), (*v).to_string());
    }
    TEST_CATALOGS.with(|c| {
        c.borrow_mut().insert(tag.to_string(), map);
    });
}

#[cfg(test)]
pub fn set_test_locale_tag(tag: Option<&str>) {
    TEST_LOCALE_TAG.with(|c| *c.borrow_mut() = tag.map(str::to_string));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_dt() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 3, 14, 5, 0).unwrap()
    }

    #[test]
    fn missing_key_falls_back_to_en_or_key() {
        assert_eq!(t("compose.send"), "Send");
        assert_eq!(lookup("en", "compose.send"), "Send");
        assert_eq!(lookup("en", "no.such.key"), "no.such.key");
        assert_eq!(t("no.such.key"), "no.such.key");
    }

    #[test]
    fn locale_switch_prefers_active_catalog_then_en() {
        install_test_catalog("xx", &[("compose.send", "Enviar")]);
        assert_eq!(lookup("xx", "compose.send"), "Enviar");
        assert_eq!(lookup("xx", "settings.title"), "Settings");
        assert_eq!(lookup("en", "compose.send"), "Send");

        set_test_locale_tag(Some("xx"));
        assert_eq!(t("compose.send"), "Enviar");
        assert_eq!(t("settings.title"), "Settings");
        set_test_locale_tag(None);
        set_locale(UiLocale::En);
        assert_eq!(t("compose.send"), "Send");
    }

    #[test]
    fn interpolation_replaces_placeholders() {
        assert_eq!(
            t_args("toast.moved", &[("folder", "Trash")]),
            "Moved to Trash"
        );
        assert_eq!(interpolate("Hello {name}", &[("name", "Ada")]), "Hello Ada");
    }

    #[test]
    fn date_format_uses_injected_locale() {
        let dt = sample_dt();
        assert_eq!(
            format_datetime_in(&dt, "en", DateStyle::Header),
            "03 Sep 2026, 14:05"
        );
        assert_eq!(
            format_datetime_in(&dt, "en-GB", DateStyle::Snooze),
            "03 Sep, 14:05"
        );
        assert_eq!(
            format_datetime_in(&dt, "de", DateStyle::Header),
            "03.09.2026, 14:05"
        );
        assert_eq!(
            format_datetime_in(&dt, "de-DE", DateStyle::ListDayYear),
            "03.09.2026"
        );
        assert_ne!(
            format_datetime_in(&dt, "de", DateStyle::Header),
            format_datetime_in(&dt, "en", DateStyle::Header)
        );

        set_date_locale_override(Some("de".into()));
        assert_eq!(format_datetime(&dt, DateStyle::Snooze), "03.09, 14:05");
        set_date_locale_override(Some("en".into()));
        assert_eq!(
            format_datetime(&dt, DateStyle::Header),
            "03 Sep 2026, 14:05"
        );
        set_date_locale_override(None);
    }

    #[test]
    fn ui_locale_keys_roundtrip() {
        for locale in UiLocale::ALL {
            assert_eq!(UiLocale::from_key(locale.as_key()), Some(locale));
        }
        assert_eq!(UiLocale::from_key("en-US"), Some(UiLocale::En));
        assert_eq!(UiLocale::from_key("de"), None);
        assert_eq!(UiLocale::default(), UiLocale::En);
    }
}
