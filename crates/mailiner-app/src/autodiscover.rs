//! Autodiscover IMAP/SMTP settings from an email address.
//!
//! Browser-safe HTTPS only (no raw DNS / SRV). Lookup order:
//! 1. Mozilla ISPDB (`autoconfig.thunderbird.net`)
//! 2. Domain `.well-known` autoconfig XML
//! 3. Common host guesses (`imap.` / `smtp.` + 993 / 465)
//!
//! OAuth is ignored. Configs that list only `OAuth2` still prefill host/port
//! so the user can sign in with an app password.

use crate::provider_preset::PresetFormFields;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// Mozilla ISPDB base (domain appended as a path segment).
pub const ISPDB_BASE: &str = "https://autoconfig.thunderbird.net/v1.1/";

#[cfg(target_arch = "wasm32")]
const FETCH_TIMEOUT_MS: u32 = 8_000;

/// Path segment: unreserved + `.` so `gmail.com` stays readable.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Where a discovered config came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverSource {
    Ispdb,
    WellKnown,
    Guess,
}

impl DiscoverSource {
    pub fn filled_message(self, domain: &str) -> String {
        match self {
            Self::Ispdb => {
                "Filled IMAP and SMTP from Mozilla ISPDB. You can edit them.".to_string()
            }
            Self::WellKnown => {
                format!("Filled IMAP and SMTP from {domain} autoconfig. You can edit them.")
            }
            Self::Guess => {
                format!(
                    "No published config; guessed imap.{domain} / smtp.{domain}. You can edit them."
                )
            }
        }
    }
}

/// IMAP + SMTP values to write onto the account form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredConfig {
    pub source: DiscoverSource,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    /// `true` + port 587 → STARTTLS; `true` + 465 → implicit TLS.
    pub smtp_use_tls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Server {
    host: String,
    port: u16,
    username: String,
    socket: SocketType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SocketType {
    Plain = 0,
    StartTls = 1,
    Ssl = 2,
}

/// Domain used for ISPDB / `.well-known` / host guesses.
///
/// Requires `user@host.tld` with a plausible DNS name (not an IP).
pub fn domain_from_email(email: &str) -> Option<String> {
    let email = email.trim();
    let (local, domain) = email.rsplit_once('@')?;
    if local.is_empty() {
        return None;
    }
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    is_lookup_domain(&domain).then_some(domain)
}

fn is_lookup_domain(domain: &str) -> bool {
    if domain.len() < 3 || domain.len() > 253 || !domain.contains('.') {
        return false;
    }
    if domain.starts_with('.') || domain.contains("..") {
        return false;
    }
    if domain
        .split('.')
        .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    domain.split('.').all(is_dns_label)
}

fn is_dns_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }
    if label.starts_with('-') || label.ends_with('-') {
        return false;
    }
    label
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `https://autoconfig.thunderbird.net/v1.1/{domain}`
pub fn ispdb_url(domain: &str) -> String {
    format!("{ISPDB_BASE}{}", utf8_percent_encode(domain, PATH_SEGMENT))
}

/// `https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml`
pub fn well_known_url(domain: &str) -> String {
    format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml")
}

/// Guess `imap.{domain}:993` and `smtp.{domain}:465` (implicit TLS).
pub fn guess_config(email: &str) -> Option<DiscoveredConfig> {
    let domain = domain_from_email(email)?;
    let username = email.trim().to_string();
    Some(DiscoveredConfig {
        source: DiscoverSource::Guess,
        imap_host: format!("imap.{domain}"),
        imap_port: 993,
        imap_username: username.clone(),
        smtp_host: format!("smtp.{domain}"),
        smtp_port: 465,
        smtp_username: username,
        smtp_use_tls: true,
    })
}

/// Pick ISPDB, then `.well-known`, then a host guess.
///
/// `None` XML / unparsable XML is a miss (try the next source).
pub fn resolve_config(
    email: &str,
    ispdb_xml: Option<&str>,
    well_known_xml: Option<&str>,
) -> Option<DiscoveredConfig> {
    if let Some(xml) = ispdb_xml
        && let Some(cfg) = parse_client_config(xml, email)
    {
        return Some(with_source(cfg, DiscoverSource::Ispdb));
    }
    if let Some(xml) = well_known_xml
        && let Some(cfg) = parse_client_config(xml, email)
    {
        return Some(with_source(cfg, DiscoverSource::WellKnown));
    }
    guess_config(email)
}

fn with_source(mut cfg: DiscoveredConfig, source: DiscoverSource) -> DiscoveredConfig {
    cfg.source = source;
    cfg
}

/// Parse Thunderbird `clientConfig` XML. Prefers IMAP over POP3 and SSL over STARTTLS.
///
/// Missing SMTP falls back to a host guess so the form still gets both sides.
pub fn parse_client_config(xml: &str, email: &str) -> Option<DiscoveredConfig> {
    let xml = strip_xml_comments(xml);
    if !xml.contains("<clientConfig") && !xml.contains("<incomingServer") {
        return None;
    }
    let imap = pick_server(&xml, "incomingServer", "imap", server_ok_for_imap)?;
    let smtp = pick_server(&xml, "outgoingServer", "smtp", server_ok_for_smtp)
        .or_else(|| guess_config(email).map(smtp_from_discovered))?;
    let mut cfg = config_from_servers(imap, smtp, DiscoverSource::Ispdb);
    finalize_usernames(&mut cfg, email);
    Some(cfg)
}

fn smtp_from_discovered(cfg: DiscoveredConfig) -> Server {
    Server {
        host: cfg.smtp_host,
        port: cfg.smtp_port,
        username: cfg.smtp_username,
        socket: if cfg.smtp_use_tls {
            if cfg.smtp_port == 587 {
                SocketType::StartTls
            } else {
                SocketType::Ssl
            }
        } else {
            SocketType::Plain
        },
    }
}

fn config_from_servers(imap: Server, smtp: Server, source: DiscoverSource) -> DiscoveredConfig {
    let smtp_use_tls = smtp_use_tls_for_form(&smtp).unwrap_or(true);
    DiscoveredConfig {
        source,
        imap_host: imap.host,
        imap_port: imap.port,
        imap_username: imap.username,
        smtp_host: smtp.host,
        smtp_port: smtp.port,
        smtp_username: smtp.username,
        smtp_use_tls,
    }
}

/// IMAP form is implicit TLS only (`use_tls: true`). Skip STARTTLS/plain.
fn server_ok_for_imap(server: &Server) -> bool {
    server.socket == SocketType::Ssl
}

/// Form TLS is `use_tls` + port (`true`+587 → STARTTLS, `true`+other → implicit).
/// Reject combinations that would flip the advertised socket type.
fn server_ok_for_smtp(server: &Server) -> bool {
    smtp_use_tls_for_form(server).is_some()
}

fn smtp_use_tls_for_form(server: &Server) -> Option<bool> {
    match server.socket {
        SocketType::Ssl if server.port != 587 => Some(true),
        SocketType::StartTls if server.port == 587 => Some(true),
        SocketType::Plain => Some(false),
        _ => None,
    }
}

fn pick_server(
    xml: &str,
    tag: &str,
    type_attr: &str,
    ok: impl Fn(&Server) -> bool,
) -> Option<Server> {
    let mut best: Option<Server> = None;
    for (attrs, inner) in elements(xml, tag) {
        if !attr_eq_ignore_ascii_case(&attrs, "type", type_attr) {
            continue;
        }
        let Some(server) = parse_server_block(inner) else {
            continue;
        };
        if !ok(&server) {
            continue;
        }
        match &best {
            None => best = Some(server),
            Some(current) if server.socket > current.socket => best = Some(server),
            Some(_) => {}
        }
    }
    best
}

fn parse_server_block(inner: &str) -> Option<Server> {
    let host = child_text(inner, "hostname")?;
    if !is_server_host(&host) {
        return None;
    }
    let port: u16 = child_text(inner, "port")?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let socket = parse_socket_type(child_text(inner, "socketType").as_deref().unwrap_or("SSL"));
    let username = child_text(inner, "username").unwrap_or_else(|| "%EMAILADDRESS%".into());
    Some(Server {
        host,
        port,
        username,
        socket,
    })
}

fn parse_socket_type(raw: &str) -> SocketType {
    match raw.trim().to_ascii_uppercase().as_str() {
        "SSL" | "SSL/TLS" | "TLS" => SocketType::Ssl,
        "STARTTLS" => SocketType::StartTls,
        _ => SocketType::Plain,
    }
}

fn is_server_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    !host.contains('/') && !host.contains(' ') && !host.contains(':') && !host.contains('@')
}

/// Substitute Thunderbird username placeholders.
pub fn substitute_username(template: &str, email: &str) -> String {
    let email = email.trim();
    let (local, domain) = email.split_once('@').unwrap_or((email, ""));
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

fn finalize_usernames(cfg: &mut DiscoveredConfig, email: &str) {
    cfg.imap_username = substitute_username(&cfg.imap_username, email);
    cfg.smtp_username = substitute_username(&cfg.smtp_username, email);
    if cfg.imap_username.trim().is_empty() {
        cfg.imap_username = email.trim().to_string();
    }
    if cfg.smtp_username.trim().is_empty() {
        cfg.smtp_username = email.trim().to_string();
    }
}

/// HTTPS lookup. WASM uses `fetch`; native tests should call [`resolve_config`].
pub async fn lookup_servers(email: &str) -> Option<DiscoveredConfig> {
    let domain = domain_from_email(email)?;
    let ispdb = fetch_https_text(&ispdb_url(&domain)).await.ok();
    if let Some(cfg) = resolve_config(email, ispdb.as_deref(), None)
        && cfg.source == DiscoverSource::Ispdb
    {
        return Some(cfg);
    }
    let well_known = fetch_https_text(&well_known_url(&domain)).await.ok();
    resolve_config(email, None, well_known.as_deref())
}

/// Whether lookup may write at least one host side (empty or last fill).
pub fn should_autofill_hosts(fields: &PresetFormFields, last: Option<&DiscoveredConfig>) -> bool {
    imap_replaceable(fields, last) || smtp_replaceable(fields, last)
}

fn imap_replaceable(fields: &PresetFormFields, last: Option<&DiscoveredConfig>) -> bool {
    fields.imap_host.trim().is_empty() || last.is_some_and(|cfg| imap_matches(fields, cfg))
}

fn smtp_replaceable(fields: &PresetFormFields, last: Option<&DiscoveredConfig>) -> bool {
    fields.smtp_host.trim().is_empty() || last.is_some_and(|cfg| smtp_matches(fields, cfg))
}

fn imap_matches(fields: &PresetFormFields, cfg: &DiscoveredConfig) -> bool {
    eq_host(&fields.imap_host, &cfg.imap_host)
        && parse_port(&fields.imap_port) == Some(cfg.imap_port)
}

fn smtp_matches(fields: &PresetFormFields, cfg: &DiscoveredConfig) -> bool {
    eq_host(&fields.smtp_host, &cfg.smtp_host)
        && parse_port(&fields.smtp_port) == Some(cfg.smtp_port)
        && fields.smtp_use_tls == cfg.smtp_use_tls
}

/// Write replaceable hosts/ports/TLS. Usernames when empty, still the email, or last fill.
pub fn apply_discovered(
    cfg: &DiscoveredConfig,
    email: &str,
    fields: &mut PresetFormFields,
    last: Option<&DiscoveredConfig>,
) {
    if imap_replaceable(fields, last) {
        fields.imap_host = cfg.imap_host.clone();
        fields.imap_port = cfg.imap_port.to_string();
    }
    if smtp_replaceable(fields, last) {
        fields.smtp_host = cfg.smtp_host.clone();
        fields.smtp_port = cfg.smtp_port.to_string();
        fields.smtp_use_tls = cfg.smtp_use_tls;
    }
    if username_is_replaceable(
        &fields.imap_username,
        email,
        last.map(|l| l.imap_username.as_str()),
    ) {
        fields.imap_username = cfg.imap_username.clone();
    }
    if username_is_replaceable(
        &fields.smtp_username,
        email,
        last.map(|l| l.smtp_username.as_str()),
    ) {
        fields.smtp_username = cfg.smtp_username.clone();
    }
}

fn username_is_replaceable(current: &str, email: &str, last_username: Option<&str>) -> bool {
    let current = current.trim();
    let email = email.trim();
    current.is_empty()
        || (!email.is_empty() && current == email)
        || last_username.is_some_and(|u| current == u)
}

fn eq_host(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b)
}

fn parse_port(s: &str) -> Option<u16> {
    let p: u16 = s.trim().parse().ok()?;
    (p != 0).then_some(p)
}

fn strip_xml_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn elements<'a>(haystack: &'a str, tag: &str) -> Vec<(String, &'a str)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = haystack[pos..].find(&open) {
        let abs = pos + rel;
        let after_name = abs + open.len();
        let rest = &haystack[after_name..];
        let next = rest.as_bytes().first().copied();
        if !matches!(next, Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/')) {
            pos = after_name;
            continue;
        }
        let Some(gt) = rest.find('>') else {
            break;
        };
        let attrs = rest[..gt].trim().trim_end_matches('/').trim().to_string();
        if rest[..gt].trim_end().ends_with('/') {
            pos = after_name + gt + 1;
            continue;
        }
        let inner_start = after_name + gt + 1;
        let Some(end) = haystack[inner_start..].find(&close) else {
            break;
        };
        out.push((attrs, &haystack[inner_start..inner_start + end]));
        pos = inner_start + end + close.len();
    }
    out
}

fn child_text(inner: &str, tag: &str) -> Option<String> {
    let (_, body) = elements(inner, tag).into_iter().next()?;
    let text = decode_xml_text(body.trim());
    (!text.is_empty()).then_some(text)
}

fn attr_eq_ignore_ascii_case(attrs: &str, name: &str, expected: &str) -> bool {
    attr_value(attrs, name).is_some_and(|v| v.eq_ignore_ascii_case(expected))
}

fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let bytes = attrs.as_bytes();
    let name_bytes = name.as_bytes();
    let mut i = 0;
    while i + name_bytes.len() <= bytes.len() {
        let window = &bytes[i..i + name_bytes.len()];
        let boundary_before = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
        if boundary_before && window.eq_ignore_ascii_case(name_bytes) {
            let mut j = i + name_bytes.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] != b'=' {
                i += 1;
                continue;
            }
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() {
                return None;
            }
            let quote = bytes[j];
            if quote == b'"' || quote == b'\'' {
                j += 1;
                let start = j;
                while j < bytes.len() && bytes[j] != quote {
                    j += 1;
                }
                return Some(decode_xml_text(std::str::from_utf8(&bytes[start..j]).ok()?));
            }
            let start = j;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            return Some(decode_xml_text(std::str::from_utf8(&bytes[start..j]).ok()?));
        }
        i += 1;
    }
    None
}

fn decode_xml_text(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

async fn fetch_https_text(url: &str) -> Result<String, String> {
    if !url.starts_with("https://") {
        return Err("only HTTPS is allowed".into());
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fetch_https_text(url).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = url;
        Err("HTTPS lookup requires a browser".into())
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_fetch_https_text(url: &str) -> Result<String, String> {
    use futures_util::future::{Either, select};
    use std::pin::pin;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::Response;

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let fetch = pin!(JsFuture::from(window.fetch_with_str(url)));
    let timeout = pin!(gloo_timers::future::TimeoutFuture::new(FETCH_TIMEOUT_MS));
    let resp_val = match select(fetch, timeout).await {
        Either::Left((result, _)) => result.map_err(|_| "network error".to_string())?,
        Either::Right(_) => return Err("timed out".into()),
    };
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| "invalid response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().map_err(|_| "read failed".to_string())?;
    let text = JsFuture::from(text)
        .await
        .map_err(|_| "read failed".to_string())?;
    text.as_string()
        .ok_or_else(|| "non-text response".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GMAIL_XML: &str = r#"
<clientConfig version="1.1">
  <emailProvider id="googlemail.com">
    <incomingServer type="imap">
      <hostname>imap.gmail.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>OAuth2</authentication>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <incomingServer type="pop3">
      <hostname>pop.gmail.com</hostname>
      <port>995</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.gmail.com</hostname>
      <port>465</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>OAuth2</authentication>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
    <oAuth2>
      <issuer>accounts.google.com</issuer>
    </oAuth2>
  </emailProvider>
</clientConfig>
"#;

    const ICLOUD_XML: &str = r#"
<clientConfig version="1.1">
  <emailProvider id="me.com">
    <incomingServer type="imap">
      <hostname>imap.mail.me.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.mail.me.com</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>
"#;

    const OUTLOOK_OAUTH_ONLY_XML: &str = r#"
<clientConfig version="1.1">
  <emailProvider id="hotmail.com">
    <incomingServer type="imap">
      <hostname>outlook.office365.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <authentication>OAuth2</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp-mail.outlook.com</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <authentication>OAuth2</authentication>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>
"#;

    fn finalize(mut cfg: DiscoveredConfig, email: &str) -> DiscoveredConfig {
        finalize_usernames(&mut cfg, email);
        cfg
    }

    #[test]
    fn domain_from_valid_email() {
        assert_eq!(
            domain_from_email("  Ada@Gmail.COM.  ").as_deref(),
            Some("gmail.com")
        );
        assert_eq!(
            domain_from_email("user@sub.example.co.uk").as_deref(),
            Some("sub.example.co.uk")
        );
    }

    #[test]
    fn domain_rejects_incomplete_or_unsafe() {
        assert_eq!(domain_from_email(""), None);
        assert_eq!(domain_from_email("not-an-email"), None);
        assert_eq!(domain_from_email("@gmail.com"), None);
        assert_eq!(domain_from_email("ada@g"), None);
        assert_eq!(domain_from_email("ada@localhost"), None);
        assert_eq!(domain_from_email("ada@127.0.0.1"), None);
        assert_eq!(domain_from_email("ada@example.com/evil"), None);
        assert_eq!(domain_from_email("ada@exam ple.com"), None);
    }

    #[test]
    fn ispdb_and_well_known_urls() {
        assert_eq!(
            ispdb_url("gmail.com"),
            "https://autoconfig.thunderbird.net/v1.1/gmail.com"
        );
        assert_eq!(
            well_known_url("example.com"),
            "https://example.com/.well-known/autoconfig/mail/config-v1.1.xml"
        );
    }

    #[test]
    fn parse_gmail_prefers_imap_over_pop3_and_ignores_oauth_block() {
        let cfg = finalize(
            parse_client_config(GMAIL_XML, "ada@gmail.com").unwrap(),
            "ada@gmail.com",
        );
        assert_eq!(cfg.imap_host, "imap.gmail.com");
        assert_eq!(cfg.imap_port, 993);
        assert_eq!(cfg.smtp_host, "smtp.gmail.com");
        assert_eq!(cfg.smtp_port, 465);
        assert!(cfg.smtp_use_tls);
        assert_eq!(cfg.imap_username, "ada@gmail.com");
        assert_eq!(cfg.smtp_username, "ada@gmail.com");
    }

    #[test]
    fn parse_icloud_local_part_and_starttls_smtp() {
        let cfg = finalize(
            parse_client_config(ICLOUD_XML, "ada@icloud.com").unwrap(),
            "ada@icloud.com",
        );
        assert_eq!(cfg.imap_host, "imap.mail.me.com");
        assert_eq!(cfg.imap_username, "ada");
        assert_eq!(cfg.smtp_host, "smtp.mail.me.com");
        assert_eq!(cfg.smtp_port, 587);
        assert!(cfg.smtp_use_tls);
        assert_eq!(cfg.smtp_username, "ada@icloud.com");
    }

    #[test]
    fn parse_oauth_only_still_prefills_hosts() {
        let cfg = finalize(
            parse_client_config(OUTLOOK_OAUTH_ONLY_XML, "ada@outlook.com").unwrap(),
            "ada@outlook.com",
        );
        assert_eq!(cfg.imap_host, "outlook.office365.com");
        assert_eq!(cfg.smtp_host, "smtp-mail.outlook.com");
        assert_eq!(cfg.smtp_port, 587);
        assert!(cfg.smtp_use_tls);
    }

    #[test]
    fn parse_prefers_ssl_when_starttls_listed_first() {
        let xml = r#"
<clientConfig version="1.1">
  <incomingServer type="imap">
    <hostname>imap-plain.example.com</hostname>
    <port>143</port>
    <socketType>STARTTLS</socketType>
    <username>%EMAILADDRESS%</username>
  </incomingServer>
  <incomingServer type="imap">
    <hostname>imap.example.com</hostname>
    <port>993</port>
    <socketType>SSL</socketType>
    <username>%EMAILADDRESS%</username>
  </incomingServer>
  <outgoingServer type="smtp">
    <hostname>smtp.example.com</hostname>
    <port>465</port>
    <socketType>SSL</socketType>
    <username>%EMAILADDRESS%</username>
  </outgoingServer>
</clientConfig>
"#;
        let cfg = parse_client_config(xml, "ada@example.com").unwrap();
        assert_eq!(cfg.imap_host, "imap.example.com");
        assert_eq!(cfg.imap_port, 993);
    }

    #[test]
    fn parse_skips_comments_and_imap_only_guesses_smtp() {
        let xml = r#"
<clientConfig version="1.1">
  <!-- <incomingServer type="imap"><hostname>evil.example</hostname></incomingServer> -->
  <incomingServer type="imap">
    <hostname>imap.example.com</hostname>
    <port>993</port>
    <socketType>SSL</socketType>
    <username>%EMAILDOMAIN%\%EMAILLOCALPART%</username>
  </incomingServer>
</clientConfig>
"#;
        let cfg = finalize(
            parse_client_config(xml, "ada@example.com").unwrap(),
            "ada@example.com",
        );
        assert_eq!(cfg.imap_host, "imap.example.com");
        assert_eq!(cfg.imap_username, "example.com\\ada");
        assert_eq!(cfg.smtp_host, "smtp.example.com");
        assert_eq!(cfg.smtp_port, 465);
    }

    #[test]
    fn parse_rejects_html_and_pop3_only() {
        assert!(parse_client_config("<html>404</html>", "ada@example.com").is_none());
        let pop3 = r#"
<clientConfig version="1.1">
  <incomingServer type="pop3">
    <hostname>pop.example.com</hostname>
    <port>995</port>
    <socketType>SSL</socketType>
    <username>%EMAILADDRESS%</username>
  </incomingServer>
</clientConfig>
"#;
        assert!(parse_client_config(pop3, "ada@example.com").is_none());
    }

    #[test]
    fn guess_imap_smtp_993_465() {
        let cfg = guess_config("ada@Example.COM").unwrap();
        assert_eq!(cfg.source, DiscoverSource::Guess);
        assert_eq!(cfg.imap_host, "imap.example.com");
        assert_eq!(cfg.imap_port, 993);
        assert_eq!(cfg.smtp_host, "smtp.example.com");
        assert_eq!(cfg.smtp_port, 465);
        assert!(cfg.smtp_use_tls);
        assert_eq!(cfg.imap_username, "ada@Example.COM");
    }

    #[test]
    fn resolve_prefers_ispdb_then_well_known_then_guess() {
        let ispdb = resolve_config("ada@gmail.com", Some(GMAIL_XML), Some(ICLOUD_XML)).unwrap();
        assert_eq!(ispdb.source, DiscoverSource::Ispdb);
        assert_eq!(ispdb.imap_host, "imap.gmail.com");

        let well =
            resolve_config("ada@icloud.com", Some("<html>404</html>"), Some(ICLOUD_XML)).unwrap();
        assert_eq!(well.source, DiscoverSource::WellKnown);
        assert_eq!(well.imap_host, "imap.mail.me.com");

        let guess = resolve_config("ada@example.com", None, None).unwrap();
        assert_eq!(guess.source, DiscoverSource::Guess);
        assert_eq!(guess.imap_host, "imap.example.com");
    }

    #[test]
    fn apply_fills_empty_and_skips_custom_username() {
        let cfg = finalize(
            parse_client_config(GMAIL_XML, "ada@gmail.com").unwrap(),
            "ada@gmail.com",
        );
        let mut fields = PresetFormFields::empty();
        apply_discovered(&cfg, "ada@gmail.com", &mut fields, None);
        assert_eq!(fields.imap_host, "imap.gmail.com");
        assert_eq!(fields.imap_port, "993");
        assert_eq!(fields.smtp_host, "smtp.gmail.com");
        assert_eq!(fields.imap_username, "ada@gmail.com");

        fields.imap_username = "keep-me".into();
        let mut next = fields.clone();
        apply_discovered(&cfg, "ada@gmail.com", &mut next, Some(&cfg));
        assert_eq!(next.imap_username, "keep-me");
        assert_eq!(next.imap_host, "imap.gmail.com");
    }

    #[test]
    fn apply_replaces_last_local_part_username() {
        let first = finalize(
            parse_client_config(ICLOUD_XML, "ada@icloud.com").unwrap(),
            "ada@icloud.com",
        );
        let mut fields = PresetFormFields::empty();
        apply_discovered(&first, "ada@icloud.com", &mut fields, None);
        assert_eq!(fields.imap_username, "ada");

        let next_cfg = finalize(
            parse_client_config(ICLOUD_XML, "bob@icloud.com").unwrap(),
            "bob@icloud.com",
        );
        apply_discovered(&next_cfg, "bob@icloud.com", &mut fields, Some(&first));
        assert_eq!(fields.imap_username, "bob");
        assert_eq!(fields.smtp_username, "bob@icloud.com");
    }

    #[test]
    fn apply_does_not_overwrite_custom_smtp_when_imap_empty() {
        let cfg = guess_config("ada@example.com").unwrap();
        let mut fields = PresetFormFields {
            smtp_host: "smtp.custom.com".into(),
            smtp_port: "587".into(),
            smtp_use_tls: true,
            ..PresetFormFields::empty()
        };
        assert!(should_autofill_hosts(&fields, None));
        apply_discovered(&cfg, "ada@example.com", &mut fields, None);
        assert_eq!(fields.imap_host, "imap.example.com");
        assert_eq!(fields.smtp_host, "smtp.custom.com");
        assert_eq!(fields.smtp_port, "587");
    }

    #[test]
    fn autofill_only_when_a_side_is_empty_or_last_discovery() {
        let cfg = guess_config("ada@example.com").unwrap();
        let empty = PresetFormFields::empty();
        assert!(should_autofill_hosts(&empty, None));

        let mut filled = PresetFormFields::empty();
        apply_discovered(&cfg, "ada@example.com", &mut filled, None);
        assert!(should_autofill_hosts(&filled, Some(&cfg)));

        filled.imap_host = "imap.custom.com".into();
        assert!(should_autofill_hosts(&filled, Some(&cfg)));
        apply_discovered(&cfg, "ada@example.com", &mut filled, Some(&cfg));
        assert_eq!(filled.imap_host, "imap.custom.com");
        assert_eq!(filled.smtp_host, "smtp.example.com");

        filled.smtp_host = "smtp.custom.com".into();
        assert!(!should_autofill_hosts(&filled, Some(&cfg)));
        assert!(!should_autofill_hosts(&filled, None));
    }

    #[test]
    fn parse_rejects_starttls_imap_and_incompatible_smtp() {
        let starttls_imap = r#"
<clientConfig version="1.1">
  <incomingServer type="imap">
    <hostname>imap.example.com</hostname>
    <port>143</port>
    <socketType>STARTTLS</socketType>
    <username>%EMAILADDRESS%</username>
  </incomingServer>
  <outgoingServer type="smtp">
    <hostname>smtp.example.com</hostname>
    <port>465</port>
    <socketType>SSL</socketType>
    <username>%EMAILADDRESS%</username>
  </outgoingServer>
</clientConfig>
"#;
        assert!(parse_client_config(starttls_imap, "ada@example.com").is_none());

        let starttls_smtp_465 = r#"
<clientConfig version="1.1">
  <incomingServer type="imap">
    <hostname>imap.example.com</hostname>
    <port>993</port>
    <socketType>SSL</socketType>
    <username>%EMAILADDRESS%</username>
  </incomingServer>
  <outgoingServer type="smtp">
    <hostname>smtp.example.com</hostname>
    <port>465</port>
    <socketType>STARTTLS</socketType>
    <username>%EMAILADDRESS%</username>
  </outgoingServer>
</clientConfig>
"#;
        let cfg = parse_client_config(starttls_smtp_465, "ada@example.com").unwrap();
        assert_eq!(cfg.imap_host, "imap.example.com");
        assert_eq!(cfg.smtp_host, "smtp.example.com");
        assert_eq!(cfg.smtp_port, 465);
        assert!(cfg.smtp_use_tls);
    }

    #[test]
    fn source_messages_do_not_include_the_address() {
        let msg = DiscoverSource::Guess.filled_message("example.com");
        assert!(msg.contains("imap.example.com"));
        assert!(!msg.contains('@'));
    }
}
