use super::settings::AuthMethod;
use crate::imap_stream::ImapStream;
use imap_next::client::{Client as ImapClient, Error as ImapError, Event, Options};
use imap_types::command::{Command, CommandBody};
use imap_types::core::{Tag, TagGenerator, VecN};
use imap_types::response::{Capability, Data, Status, StatusKind};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("IMAP error: {0}")]
    Imap(#[from] ImapError),
    #[error("Login error: {0}")]
    LoginError(String),
    #[error("Unexpected response from server")]
    UnexpectedResponse(String),
}

pub struct ImapConnector<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    stream: ImapStream<T>,
    client: ImapClient,
    tag_generator: TagGenerator,
}

impl<T> ImapConnector<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn new(transport: T) -> Result<Self, Error> {
        let imap_stream = ImapStream::new(transport);
        let client = ImapClient::new(Options::default());

        let mut connector = Self {
            stream: imap_stream,
            client,
            tag_generator: TagGenerator::new(),
        };

        loop {
            match connector.stream.next(&mut connector.client).await.unwrap() {
                Event::GreetingReceived { .. } => {
                    break;
                }
                event => {
                    return Err(Error::UnexpectedResponse(format!(
                        "Unexpected response from server: {:?}",
                        event
                    )));
                }
            }
        }

        Ok(connector)
    }

    fn next_tag(&mut self) -> Tag<'static> {
        self.tag_generator.generate()
    }

    pub async fn authenticate(&mut self, auth: AuthMethod) -> Result<(), Error> {
        match auth {
            AuthMethod::Login { username, password } => {
                let cmd = Command::new(
                    self.next_tag(),
                    CommandBody::login(username.to_string(), password.to_string()).unwrap(),
                )
                .unwrap();
                self.client.enqueue_command(cmd);
            }
            _ => todo!(),
        };

        loop {
            match self.stream.next(&mut self.client).await.unwrap() {
                Event::CommandSent { .. } => {},
                Event::CommandRejected { .. } => {
                    return Err(Error::LoginError("Command rejected".to_string()))
                }
                Event::StatusReceived { .. } => {}, // TODO: collect capabilities
                Event::AuthenticateStatusReceived {
                    handle: _,
                    command_authenticate: _,
                    status,
                } => match status {
                    Status::Tagged(s) => match s.body.kind {
                        StatusKind::Ok => return Ok(()),
                        _ => {
                            return Err(Error::LoginError(format!("Login failed: {}", s.body.text)))
                        }
                    },
                    _ => return Err(Error::LoginError("Unexpected status".to_string())),
                },
                event => {
                    return Err(Error::LoginError(format!("Unexpected response from server: {:?}", event)));
                }
            }
        }
    }

    pub async fn capabilities(&mut self) -> Result<Vec<Capability>, Error> {
        let cmd = Command::new(self.next_tag(), CommandBody::Capability).unwrap();
        self.client.enqueue_command(cmd);

        let mut capabilities = Vec::<Capability>::new();
        loop {
            match self.stream.next(&mut self.client).await.unwrap() {
                Event::CommandSent { .. } => {},
                Event::DataReceived { data } => match data {
                    Data::Capability(capability) => {
                        capabilities.extend_from_slice(&capability.into_inner());
                    }
                    resp => {
                        return Err(Error::UnexpectedResponse(format!(
                            "Expected CAPABILITY data, received {:?}",
                            resp
                        )))
                    }
                },
                Event::StatusReceived { status } => match status {
                    Status::Tagged(s) => match s.body.kind {
                        StatusKind::Ok => return Ok(capabilities),
                        _ => return Err(Error::UnexpectedResponse(s.body.text.to_string())),
                    },
                    status => {
                        return Err(Error::UnexpectedResponse(format!(
                            "Unexpected status: {:?}",
                            status
                        )))
                    }
                },
                event => {
                    return Err(Error::UnexpectedResponse(format!(
                        "Unexpected event from server: {:?}",
                        event
                    )));
                }
            }
        }
    }
}

#[cfg(test)]
mod testing {

    use super::*;
    use imap_types::core::Atom;
    use imap_types::response::{Capability, CapabilityOther};
    use tokio_test::io::Builder;

    #[tokio::test]
    async fn test_authenticate() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"1.0 LOGIN testuser testpassword\r\n")
            .read(b"1.0 OK [CAPABILITY IMAP4rev1 ID IDLE MOVE LITERAL+ QUOTA X-REALLY-SPECIAL] Logged in\r\n")
            .build();

        let mut connector = ImapConnector::new(stream)
            .await
            .expect("Failed to create connector");
        connector
            .authenticate(AuthMethod::Login {
                username: "testuser".to_string(),
                password: "testpassword".to_string(),
            })
            .await
            .expect("Failed to authenticate");
    }

    #[tokio::test]
    async fn test_capabilities() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"0.0 CAPABILITY\r\n")
            .read(b"* CAPABILITY IMAP4rev1 ID IDLE MOVE LITERAL+ QUOTA X-REALLY-SPECIAL\r\n")
            .read(b"0.0 OK Capability completed\r\n")
            .build();
        let mut connector = ImapConnector::new(stream)
            .await
            .expect("Failed to create connector");

        let result = connector
            .capabilities()
            .await
            .expect("Failed to get capabilities");
        assert_eq!(
            result,
            vec![
                Capability::Imap4Rev1,
                Capability::try_from(Atom::try_from("ID").unwrap()).unwrap(),
                Capability::Idle,
                Capability::Move,
                Capability::LiteralPlus,
                Capability::Quota,
                Capability::try_from(Atom::try_from("X-REALLY-SPECIAL").unwrap()).unwrap()
            ]
        );
    }
}
