//! Pre-auth mechanism selection: SASL PLAIN when advertised, else IMAP LOGIN.

use std::fmt::Debug;

use async_imap::types::UnsolicitedResponse;
use async_imap::{Authenticator, Client};
use imap_proto::{Capability, Response, ResponseCode};
use tokio::io::{AsyncRead, AsyncWrite};

/// RFC 4616 SASL PLAIN (`NUL authcid NUL passwd`). No XOAUTH2.
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreauthCaps {
    pub auth_plain: bool,
    pub login_disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthChoice {
    Plain,
    Login,
    None,
}

impl PreauthCaps {
    pub(crate) fn choice(self) -> AuthChoice {
        if self.auth_plain {
            AuthChoice::Plain
        } else if !self.login_disabled {
            AuthChoice::Login
        } else {
            AuthChoice::None
        }
    }
}

pub(crate) fn apply_capability_list(list: &[Capability<'_>], caps: &mut PreauthCaps) {
    for c in list {
        match c {
            Capability::Auth(m) if m.eq_ignore_ascii_case("PLAIN") => caps.auth_plain = true,
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
}
