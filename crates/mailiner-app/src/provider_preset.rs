//! Provider presets that fill IMAP/SMTP host, port, and TLS on the account form.
//!
//! Credentials (email / password) are never written here. Username is copied
//! from the email only when the username field is empty.

use crate::account_config::{SmtpTlsMode, tls_mode_from_legacy};

/// Documented IMAP/SMTP defaults for a named provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetServers {
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    /// `true` + port 587 → STARTTLS; `true` + 465 → implicit TLS.
    pub smtp_use_tls: bool,
}

impl PresetServers {
    pub fn smtp_tls_mode(self) -> SmtpTlsMode {
        tls_mode_from_legacy(self.smtp_use_tls, self.smtp_port)
    }
}

/// Onboarding / account-form provider choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPreset {
    Gmail,
    Outlook,
    Fastmail,
    Custom,
}

impl ProviderPreset {
    pub const ALL: &[Self] = &[Self::Gmail, Self::Outlook, Self::Fastmail, Self::Custom];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Gmail => "gmail",
            Self::Outlook => "outlook",
            Self::Fastmail => "fastmail",
            Self::Custom => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gmail => "Gmail",
            Self::Outlook => "Outlook / Microsoft 365",
            Self::Fastmail => "Fastmail",
            Self::Custom => "Custom",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "gmail" => Self::Gmail,
            "outlook" => Self::Outlook,
            "fastmail" => Self::Fastmail,
            _ => Self::Custom,
        }
    }

    /// Named-provider server defaults. [`Self::Custom`] has none.
    ///
    /// Outlook SMTP host depends on `email`: consumer Outlook.com family
    /// uses `smtp-mail.outlook.com`; Microsoft 365 uses `smtp.office365.com`.
    pub fn servers(self, email: &str) -> Option<PresetServers> {
        match self {
            // IMAP 993 implicit TLS. SMTP 465 implicit TLS (also documents 587 STARTTLS).
            Self::Gmail => Some(PresetServers {
                imap_host: "imap.gmail.com",
                imap_port: 993,
                smtp_host: "smtp.gmail.com",
                smtp_port: 465,
                smtp_use_tls: true,
            }),
            // IMAP 993 implicit; SMTP 587 STARTTLS (Microsoft does not document 465).
            Self::Outlook => Some(PresetServers {
                imap_host: "outlook.office365.com",
                imap_port: 993,
                smtp_host: outlook_smtp_host(email),
                smtp_port: 587,
                smtp_use_tls: true,
            }),
            // Fastmail: IMAP 993 implicit; SMTP 465 implicit TLS.
            Self::Fastmail => Some(PresetServers {
                imap_host: "imap.fastmail.com",
                imap_port: 993,
                smtp_host: "smtp.fastmail.com",
                smtp_port: 465,
                smtp_use_tls: true,
            }),
            Self::Custom => None,
        }
    }
}

/// IMAP/SMTP form strings a preset may write. No email or password fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFormFields {
    pub imap_host: String,
    pub imap_port: String,
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    pub smtp_use_tls: bool,
}

impl PresetFormFields {
    pub fn empty() -> Self {
        Self {
            imap_host: String::new(),
            imap_port: String::new(),
            imap_username: String::new(),
            smtp_host: String::new(),
            smtp_port: String::new(),
            smtp_username: String::new(),
            smtp_use_tls: true,
        }
    }
}

/// Apply `preset` onto `fields`.
///
/// [`ProviderPreset::Custom`] is a no-op (keeps user tweaks). Named presets
/// overwrite host/port/TLS only. Usernames are filled from `email` when empty.
pub fn apply_preset(preset: ProviderPreset, email: &str, fields: &mut PresetFormFields) {
    let Some(servers) = preset.servers(email) else {
        return;
    };
    fields.imap_host = servers.imap_host.to_string();
    fields.imap_port = servers.imap_port.to_string();
    fields.smtp_host = servers.smtp_host.to_string();
    fields.smtp_port = servers.smtp_port.to_string();
    fields.smtp_use_tls = servers.smtp_use_tls;
    fill_usernames_from_email(email, "", fields);
}

/// After the email field changes: update auto-filled usernames, and if SMTP is
/// still a documented Microsoft host, switch consumer vs Microsoft 365.
///
/// IMAP username is filled when empty or still equal to `previous_email`.
/// SMTP username is only filled when an SMTP host is already set (optional
/// SMTP must stay `None` if the section is unused).
pub fn apply_email_change(previous_email: &str, email: &str, fields: &mut PresetFormFields) {
    fill_usernames_from_email(email, previous_email, fields);
    if is_microsoft_smtp_host(&fields.smtp_host) {
        fields.smtp_host = outlook_smtp_host(email).to_string();
    }
}

fn fill_usernames_from_email(email: &str, previous_email: &str, fields: &mut PresetFormFields) {
    let Some(user) = username_from_email(email) else {
        return;
    };
    if should_update_username(&fields.imap_username, previous_email) {
        fields.imap_username = user.clone();
    }
    if !fields.smtp_host.trim().is_empty()
        && should_update_username(&fields.smtp_username, previous_email)
    {
        fields.smtp_username = user;
    }
}

fn should_update_username(current: &str, previous_email: &str) -> bool {
    let current = current.trim();
    let previous_email = previous_email.trim();
    current.is_empty() || (!previous_email.is_empty() && current == previous_email)
}

/// Which named preset matches the current host/port/TLS, or Custom.
pub fn matching_preset(fields: &PresetFormFields) -> ProviderPreset {
    for preset in [
        ProviderPreset::Gmail,
        ProviderPreset::Outlook,
        ProviderPreset::Fastmail,
    ] {
        // Email only affects Outlook's SMTP host; matching accepts both.
        let Some(servers) = preset.servers("") else {
            continue;
        };
        let smtp_ok = if preset == ProviderPreset::Outlook {
            is_microsoft_smtp_host(&fields.smtp_host)
        } else {
            hosts_eq(&fields.smtp_host, servers.smtp_host)
        };
        if hosts_eq(&fields.imap_host, servers.imap_host)
            && parse_port(&fields.imap_port) == Some(servers.imap_port)
            && smtp_ok
            && parse_port(&fields.smtp_port) == Some(servers.smtp_port)
            && fields.smtp_use_tls == servers.smtp_use_tls
        {
            return preset;
        }
    }
    ProviderPreset::Custom
}

/// Consumer Outlook.com family → `smtp-mail.outlook.com`; else Microsoft 365.
fn outlook_smtp_host(email: &str) -> &'static str {
    if is_outlook_consumer_email(email) {
        "smtp-mail.outlook.com"
    } else {
        "smtp.office365.com"
    }
}

fn is_microsoft_smtp_host(host: &str) -> bool {
    hosts_eq(host, "smtp.office365.com") || hosts_eq(host, "smtp-mail.outlook.com")
}

fn is_outlook_consumer_email(email: &str) -> bool {
    let Some((_, domain)) = email.trim().rsplit_once('@') else {
        return false;
    };
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    matches!(
        consumer_sld(&domain),
        Some("outlook" | "hotmail" | "live" | "msn")
    )
}

/// Second-level label, or the label before `co.uk` / `com.br`-style public suffixes.
fn consumer_sld(domain: &str) -> Option<&str> {
    let mut labels = domain.split('.').rev();
    let tld = labels.next()?;
    let sld = labels.next()?;
    if tld.is_empty() || sld.is_empty() {
        return None;
    }
    if matches!(sld, "co" | "com" | "ac" | "org" | "ne" | "or") {
        return labels.next().filter(|s| !s.is_empty());
    }
    Some(sld)
}

fn username_from_email(email: &str) -> Option<String> {
    let email = email.trim();
    if email.is_empty() {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    Some(email.to_string())
}

fn hosts_eq(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b)
}

fn parse_port(s: &str) -> Option<u16> {
    let p: u16 = s.trim().parse().ok()?;
    (p != 0).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(preset: ProviderPreset, email: &str) -> PresetFormFields {
        let mut fields = PresetFormFields::empty();
        apply_preset(preset, email, &mut fields);
        fields
    }

    #[test]
    fn gmail_fills_imap_993_and_smtp_465_implicit() {
        let fields = apply(ProviderPreset::Gmail, "ada@gmail.com");
        assert_eq!(fields.imap_host, "imap.gmail.com");
        assert_eq!(fields.imap_port, "993");
        assert_eq!(fields.smtp_host, "smtp.gmail.com");
        assert_eq!(fields.smtp_port, "465");
        assert!(fields.smtp_use_tls);
        assert_eq!(fields.imap_username, "ada@gmail.com");
        assert_eq!(fields.smtp_username, "ada@gmail.com");
        assert_eq!(
            tls_mode_from_legacy(fields.smtp_use_tls, 465),
            SmtpTlsMode::Implicit
        );
        assert_eq!(matching_preset(&fields), ProviderPreset::Gmail);
        assert_eq!(
            ProviderPreset::Gmail
                .servers("ada@gmail.com")
                .unwrap()
                .smtp_tls_mode(),
            SmtpTlsMode::Implicit
        );
    }

    #[test]
    fn outlook_fills_imap_993_and_smtp_587_starttls() {
        let fields = apply(ProviderPreset::Outlook, "ada@contoso.com");
        assert_eq!(fields.imap_host, "outlook.office365.com");
        assert_eq!(fields.imap_port, "993");
        assert_eq!(fields.smtp_host, "smtp.office365.com");
        assert_eq!(fields.smtp_port, "587");
        assert!(fields.smtp_use_tls);
        assert_eq!(fields.imap_username, "ada@contoso.com");
        assert_eq!(fields.smtp_username, "ada@contoso.com");
        assert_eq!(
            tls_mode_from_legacy(fields.smtp_use_tls, 587),
            SmtpTlsMode::StartTls
        );
        assert_eq!(matching_preset(&fields), ProviderPreset::Outlook);
        assert_eq!(
            ProviderPreset::Outlook
                .servers("ada@contoso.com")
                .unwrap()
                .smtp_tls_mode(),
            SmtpTlsMode::StartTls
        );
    }

    #[test]
    fn outlook_consumer_uses_smtp_mail_outlook_com() {
        for email in [
            "ada@outlook.com",
            "ada@hotmail.com",
            "ada@live.com",
            "ada@msn.com",
            "ada@hotmail.co.uk",
            "Ada@Outlook.Com",
        ] {
            let fields = apply(ProviderPreset::Outlook, email);
            assert_eq!(fields.smtp_host, "smtp-mail.outlook.com", "email={email}");
            assert_eq!(fields.imap_host, "outlook.office365.com");
            assert_eq!(fields.smtp_port, "587");
            assert_eq!(matching_preset(&fields), ProviderPreset::Outlook);
        }
    }

    #[test]
    fn email_change_fills_empty_usernames_and_outlook_smtp() {
        let mut fields = apply(ProviderPreset::Outlook, "");
        assert!(fields.imap_username.is_empty());
        assert_eq!(fields.smtp_host, "smtp.office365.com");

        apply_email_change("", "ada@outlook.com", &mut fields);
        assert_eq!(fields.imap_username, "ada@outlook.com");
        assert_eq!(fields.smtp_username, "ada@outlook.com");
        assert_eq!(fields.smtp_host, "smtp-mail.outlook.com");

        apply_email_change("ada@outlook.com", "ada@contoso.com", &mut fields);
        assert_eq!(fields.imap_username, "ada@contoso.com");
        assert_eq!(fields.smtp_host, "smtp.office365.com");
    }

    #[test]
    fn email_change_updates_autofilled_partial_username() {
        let mut fields = apply(ProviderPreset::Gmail, "");
        apply_email_change("", "ada@g", &mut fields);
        assert_eq!(fields.imap_username, "ada@g");
        apply_email_change("ada@g", "ada@gmail.com", &mut fields);
        assert_eq!(fields.imap_username, "ada@gmail.com");
        assert_eq!(fields.smtp_username, "ada@gmail.com");

        fields.imap_username = "keep-me".into();
        apply_email_change("ada@gmail.com", "ada@other.com", &mut fields);
        assert_eq!(fields.imap_username, "keep-me");
    }

    #[test]
    fn email_change_does_not_fill_smtp_username_without_host() {
        let mut fields = PresetFormFields::empty();
        apply_email_change("", "ada@gmail.com", &mut fields);
        assert_eq!(fields.imap_username, "ada@gmail.com");
        assert!(fields.smtp_username.is_empty());
        assert!(fields.smtp_host.is_empty());
    }

    #[test]
    fn email_change_does_not_overwrite_custom_smtp_host() {
        let mut fields = apply(ProviderPreset::Gmail, "");
        fields.smtp_host = "smtp.example.com".into();
        apply_email_change("", "ada@gmail.com", &mut fields);
        assert_eq!(fields.smtp_host, "smtp.example.com");
        assert_eq!(fields.imap_username, "ada@gmail.com");
    }

    #[test]
    fn outlook_consumer_includes_cctld_aliases() {
        for email in [
            "ada@hotmail.es",
            "ada@outlook.es",
            "ada@live.nl",
            "ada@msn.de",
        ] {
            let fields = apply(ProviderPreset::Outlook, email);
            assert_eq!(fields.smtp_host, "smtp-mail.outlook.com", "email={email}");
        }
        let work = apply(ProviderPreset::Outlook, "ada@contoso.com");
        assert_eq!(work.smtp_host, "smtp.office365.com");
    }

    #[test]
    fn fastmail_fills_imap_993_and_smtp_465_implicit() {
        let fields = apply(ProviderPreset::Fastmail, "ada@fastmail.com");
        assert_eq!(fields.imap_host, "imap.fastmail.com");
        assert_eq!(fields.imap_port, "993");
        assert_eq!(fields.smtp_host, "smtp.fastmail.com");
        assert_eq!(fields.smtp_port, "465");
        assert!(fields.smtp_use_tls);
        assert_eq!(fields.imap_username, "ada@fastmail.com");
        assert_eq!(fields.smtp_username, "ada@fastmail.com");
        assert_eq!(
            tls_mode_from_legacy(fields.smtp_use_tls, 465),
            SmtpTlsMode::Implicit
        );
        assert_eq!(matching_preset(&fields), ProviderPreset::Fastmail);
    }

    #[test]
    fn custom_is_noop() {
        let mut fields = PresetFormFields {
            imap_host: "imap.example.com".into(),
            imap_port: "143".into(),
            imap_username: "keep-me".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: "25".into(),
            smtp_username: "also-keep".into(),
            smtp_use_tls: false,
        };
        let before = fields.clone();
        apply_preset(ProviderPreset::Custom, "ada@example.com", &mut fields);
        assert_eq!(fields, before);
        assert_eq!(matching_preset(&fields), ProviderPreset::Custom);
    }

    #[test]
    fn does_not_overwrite_usernames() {
        let mut fields = PresetFormFields {
            imap_username: "imap-user".into(),
            smtp_username: "smtp-user".into(),
            smtp_use_tls: true,
            ..PresetFormFields::empty()
        };
        apply_preset(ProviderPreset::Gmail, "ada@gmail.com", &mut fields);
        assert_eq!(fields.imap_username, "imap-user");
        assert_eq!(fields.smtp_username, "smtp-user");
        assert_eq!(fields.imap_host, "imap.gmail.com");
        assert_eq!(fields.smtp_host, "smtp.gmail.com");
    }

    #[test]
    fn prefills_username_from_email_only_when_empty() {
        let mut fields = PresetFormFields {
            imap_username: "  ".into(),
            smtp_username: String::new(),
            smtp_use_tls: true,
            ..PresetFormFields::empty()
        };
        apply_preset(ProviderPreset::Gmail, "  ada@gmail.com  ", &mut fields);
        assert_eq!(fields.imap_username, "ada@gmail.com");
        assert_eq!(fields.smtp_username, "ada@gmail.com");
    }

    #[test]
    fn empty_or_invalid_email_does_not_prefill_username() {
        let mut fields = PresetFormFields::empty();
        apply_preset(ProviderPreset::Gmail, "", &mut fields);
        assert!(fields.imap_username.is_empty());
        assert!(fields.smtp_username.is_empty());

        apply_preset(ProviderPreset::Gmail, "not-an-email", &mut fields);
        assert!(fields.imap_username.is_empty());

        apply_preset(ProviderPreset::Gmail, "@gmail.com", &mut fields);
        assert!(fields.imap_username.is_empty());
    }

    #[test]
    fn switching_named_presets_overwrites_hosts_keeps_usernames() {
        let mut fields = apply(ProviderPreset::Gmail, "ada@example.com");
        apply_preset(ProviderPreset::Outlook, "ada@example.com", &mut fields);
        assert_eq!(fields.imap_host, "outlook.office365.com");
        assert_eq!(fields.smtp_host, "smtp.office365.com");
        assert_eq!(fields.smtp_port, "587");
        assert_eq!(fields.imap_username, "ada@example.com");
        assert_eq!(fields.smtp_username, "ada@example.com");
    }

    #[test]
    fn matching_preset_is_case_insensitive_and_trims() {
        let fields = PresetFormFields {
            imap_host: "  IMAP.GMAIL.COM  ".into(),
            imap_port: "993".into(),
            imap_username: String::new(),
            smtp_host: "Smtp.Gmail.Com".into(),
            smtp_port: "0465".into(),
            smtp_username: String::new(),
            smtp_use_tls: true,
        };
        assert_eq!(matching_preset(&fields), ProviderPreset::Gmail);
    }

    #[test]
    fn matching_preset_custom_on_tls_or_port_mismatch() {
        let mut fields = apply(ProviderPreset::Outlook, "ada@contoso.com");
        fields.smtp_use_tls = false;
        assert_eq!(matching_preset(&fields), ProviderPreset::Custom);

        let mut fields = apply(ProviderPreset::Gmail, "ada@gmail.com");
        fields.smtp_port = "587".into();
        assert_eq!(matching_preset(&fields), ProviderPreset::Custom);
    }

    #[test]
    fn from_key_and_labels() {
        assert_eq!(ProviderPreset::from_key("gmail"), ProviderPreset::Gmail);
        assert_eq!(ProviderPreset::from_key("outlook"), ProviderPreset::Outlook);
        assert_eq!(
            ProviderPreset::from_key("fastmail"),
            ProviderPreset::Fastmail
        );
        assert_eq!(ProviderPreset::from_key("nope"), ProviderPreset::Custom);
        assert_eq!(ProviderPreset::Outlook.label(), "Outlook / Microsoft 365");
        assert_eq!(ProviderPreset::ALL.len(), 4);
    }
}
