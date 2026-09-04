//! Pre-auth mechanism selection: SASL PLAIN / LOGIN, or XOAUTH2 when requested.

use std::fmt::Debug;

use async_imap::types::UnsolicitedResponse;
use async_imap::{Authenticator, Client};
use imap_proto::{Capability, Response, ResponseCode};
use tokio::io::{AsyncRead, AsyncWrite};

/// How the connector should authenticate. Password is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImapAuthKind {
    #[default]
    Password,
    Xoauth2,
}

/// RFC 7628 / Google XOAUTH2 SASL blob: `user=…\x01auth=Bearer …\x01\x01`.
pub fn xoauth2_sasl_payload(username: &str, access_token: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(username.len() + access_token.len() + 20);
    buf.extend_from_slice(b"user=");
    buf.extend_from_slice(username.as_bytes());
    buf.push(0x01);
    buf.extend_from_slice(b"auth=Bearer ");
    buf.extend_from_slice(access_token.as_bytes());
    buf.push(0x01);
    buf.push(0x01);
    buf
}

/// RFC 4616 SASL PLAIN (`NUL authcid NUL passwd`).
pub(crate) struct SaslPlain<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

impl Authenticator for SaslPlain<'_> {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        let mut buf = Vec::with_capacity(self.username.len() + self.password.len() + 2);
        buf.push(0);
        buf.extend_from_slice(self.username.as_bytes());
        buf.push(0);
        buf.extend_from_slice(self.password.as_bytes());
        buf
    }
}

/// SASL XOAUTH2. `credentials` is the access token (never a password).
pub(crate) struct SaslXoauth2<'a> {
    pub username: &'a str,
    pub access_token: &'a str,
}

impl Authenticator for SaslXoauth2<'_> {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        xoauth2_sasl_payload(self.username, self.access_token)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreauthCaps {
    pub auth_plain: bool,
    pub auth_xoauth2: bool,
    pub login_disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthChoice {
    Plain,
    Login,
    Xoauth2,
    None,
}

impl PreauthCaps {
    /// Password login: PLAIN if advertised, else IMAP LOGIN unless disabled.
    pub(crate) fn choice(self) -> AuthChoice {
        if self.auth_plain {
            AuthChoice::Plain
        } else if !self.login_disabled {
            AuthChoice::Login
        } else {
            AuthChoice::None
        }
    }

    /// OAuth: XOAUTH2 only. Never fall back to LOGIN with a bearer token.
    pub(crate) fn choice_oauth(self) -> AuthChoice {
        if self.auth_xoauth2 {
            AuthChoice::Xoauth2
        } else {
            AuthChoice::None
        }
    }
}

pub(crate) fn apply_capability_list(list: &[Capability<'_>], caps: &mut PreauthCaps) {
    for c in list {
        match c {
            Capability::Auth(m) if m.eq_ignore_ascii_case("PLAIN") => caps.auth_plain = true,
            Capability::Auth(m) if m.eq_ignore_ascii_case("XOAUTH2") => caps.auth_xoauth2 = true,
            Capability::Atom(a) if a.eq_ignore_ascii_case("LOGINDISABLED") => {
                caps.login_disabled = true;
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_from_response(resp: &Response<'_>, caps: &mut PreauthCaps) {
    match resp {
        Response::Capabilities(list) => apply_capability_list(list, caps),
        Response::Data {
            code: Some(ResponseCode::Capabilities(list)),
            ..
        }
        | Response::Done {
            code: Some(ResponseCode::Capabilities(list)),
            ..
        } => apply_capability_list(list, caps),
        _ => {}
    }
}

/// Issue `CAPABILITY` on an unauthenticated client and parse AUTH=PLAIN / LOGINDISABLED.
pub(crate) async fn query_preauth_caps<T>(
    client: &mut Client<T>,
) -> Result<PreauthCaps, async_imap::error::Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let (tx, rx) = async_channel::unbounded();
    client
        .run_command_and_check_ok("CAPABILITY", Some(tx))
        .await?;
    let mut caps = PreauthCaps::default();
    while let Ok(msg) = rx.try_recv() {
        if let UnsolicitedResponse::Other(data) = msg {
            collect_from_response(data.parsed(), &mut caps);
        }
    }
    Ok(caps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps_from(atoms: &[&str]) -> PreauthCaps {
        let list: Vec<Capability<'static>> = atoms
            .iter()
            .map(|a| {
                if a.eq_ignore_ascii_case("IMAP4rev1") {
                    Capability::Imap4rev1
                } else if let Some(rest) =
                    a.strip_prefix("AUTH=").or_else(|| a.strip_prefix("auth="))
                {
                    Capability::Auth(rest.to_string().into())
                } else {
                    Capability::Atom(a.to_string().into())
                }
            })
            .collect();
        let mut caps = PreauthCaps::default();
        apply_capability_list(&list, &mut caps);
        caps
    }

    #[test]
    fn prefers_plain_when_advertised() {
        assert_eq!(
            caps_from(&["IMAP4rev1", "AUTH=PLAIN", "AUTH=LOGIN"]).choice(),
            AuthChoice::Plain
        );
        assert_eq!(
            caps_from(&["IMAP4rev1", "AUTH=plain", "LOGINDISABLED"]).choice(),
            AuthChoice::Plain
        );
    }

    #[test]
    fn login_when_plain_absent() {
        assert_eq!(
            caps_from(&["IMAP4rev1", "AUTH=XOAUTH2"]).choice(),
            AuthChoice::Login
        );
        assert_eq!(caps_from(&["IMAP4rev1"]).choice(), AuthChoice::Login);
    }

    #[test]
    fn none_when_login_disabled_and_no_plain() {
        assert_eq!(
            caps_from(&["IMAP4rev1", "LOGINDISABLED", "AUTH=XOAUTH2"]).choice(),
            AuthChoice::None
        );
    }

    #[test]
    fn sasl_plain_payload_is_nul_user_nul_pass() {
        let mut auth = SaslPlain {
            username: "user@example.com",
            password: "secret",
        };
        assert_eq!(auth.process(b""), b"\0user@example.com\0secret");
    }

    #[test]
    fn xoauth2_sasl_blob_is_user_soh_auth_bearer_soh_soh() {
        let blob = xoauth2_sasl_payload("ada@gmail.com", "ya29.token");
        assert_eq!(
            blob,
            b"user=ada@gmail.com\x01auth=Bearer ya29.token\x01\x01"
        );
        let mut auth = SaslXoauth2 {
            username: "ada@gmail.com",
            access_token: "ya29.token",
        };
        assert_eq!(auth.process(b""), blob);
    }

    #[test]
    fn oauth_choice_requires_xoauth2() {
        assert_eq!(
            caps_from(&["IMAP4rev1", "AUTH=XOAUTH2", "AUTH=PLAIN"]).choice_oauth(),
            AuthChoice::Xoauth2
        );
        assert_eq!(
            caps_from(&["IMAP4rev1", "AUTH=PLAIN", "LOGINDISABLED"]).choice_oauth(),
            AuthChoice::None
        );
    }

    #[test]
    fn password_choice_ignores_xoauth2() {
        assert_eq!(
            caps_from(&["IMAP4rev1", "AUTH=XOAUTH2"]).choice(),
            AuthChoice::Login
        );
    }
}
