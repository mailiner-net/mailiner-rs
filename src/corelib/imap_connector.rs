use super::settings::AuthMethod;
use imap_types::command::CommandBody;
use dioxus::prelude::*;
use futures_util::stream::StreamExt;

enum ImapCommand {
    AuthenticateCommand {
        auth: AuthMethod,
    },
    AuthenticateResponse,
}


pub struct ImapConnector
{
}

impl ImapConnector
{
    pub fn new() -> Self
    {
        let coro = use_coroutine(|mut rx: UnboundedReceiver<ImapCommand>| async move {
            while let Some(action) = rx.next().await {
                match action {
                    ImapCommand::AuthenticateCommand { auth } => {
                        // Authenticate
                    },
                    _ => todo!()
                }

            }
        });

        Self
        {

        }
    }

    pub async fn connect(&self) -> Result<(), ImapError>
    {
        todo!();
        Ok(())
    }

    pub async fn authenticate(&self, auth: AuthMethod) -> Result<(), ImapError>
    {
        todo!();
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