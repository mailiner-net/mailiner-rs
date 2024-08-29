use super::settings::AuthMethod;
use imap_codec::{encode, decode};
use imap_next::imap_types::secret::Secret;
use imap_types::command::{Command, CommandBody};
use imap_types::auth::AuthMechanism;
use imap_types::core::Tag;
use imap_next::client::{Client as ImapClient, Options};
use futures::{AsyncRead, AsyncWrite};
use futures_util::stream::StreamExt;
use crate::transport_stream::TransportStream;


pub struct ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin
{
    stream: S,
    client: ImapClient,
}

impl ImapConnector<TransportStream>
{
    pub async fn new(proxy_url: &str, imap_server: &str, imap_port: u16) -> Self
    {
        let stream = TransportStream::connect_with_tls(proxy_url, imap_server, cert_store).await.unwrap();
        let client = ImapClient::new(Options::default());
        Self {
            stream,
            client
        }
    }

    pub async fn authenticate(&self, auth: AuthMethod) -> Result<(), ImapError>
    {
        match auth {
            AuthMethod::Login{ username, password } => {
                let cmd = Command::new(self.generate_tag(), CommandBody::login(username.into(), password.into())?)?;
                let resp = self.client.enqueue_command(cmd);
            },
            AuthMethod::Plain { username, password } => {
                let cmd = Command::new(self.generate_tag(), CommandBody::authenticate(AuthMechanism::Login))?;
                let resp = self.client.enqueue_command(cmd);
            }
            _ => todo!(),
        };

        Ok(())
    }

    pub async fn capabilities(&self) -> Result<imap_types::response::Capability, ImapError>
    {
        todo!();
        Ok(imap_types::response::Capability::default())
    }

    pub async fn list(&self) -> Result<Vec<imap_types::response::List>, ImapError>
    {
        todo!();
        Ok(vec![])
    }
}