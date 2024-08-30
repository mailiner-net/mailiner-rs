use std::num::NonZeroU32;

use super::settings::AuthMethod;
use crate::imap_stream::ImapStream;
use imap_next::client::{Client as ImapClient, Error as ImapError, Event, Options};
use imap_types::command::{Command, CommandBody};
use imap_types::core::{Tag, TagGenerator as ImapTagGenerator};
use imap_types::flag::FlagPerm;
use imap_types::mailbox::{ListMailbox, Mailbox as ImapMailbox};
use imap_types::response::{Capability, Code, Data, Status, StatusKind};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("IMAP error: {0}")]
    Imap(#[from] ImapError),
    #[error("Login error: {0}")]
    BadCommandError(String),
    #[error("Command failed: {0}")]
    CommandFailed(String),
    #[error("Unexpected response: {0}")]
    UnexpectedResponse(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mailbox {
    pub flags: Vec<String>,
    pub delimiter: Option<char>,
    pub name: ImapMailbox<'static>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct MailboxStats {
    pub exists: Option<u32>,
    pub recent: Option<u32>,
    pub flags: Option<Vec<String>>,
    pub permanent_flags: Option<Vec<FlagPerm<'static>>>,
    pub read_only: bool,
    pub uid_next: Option<NonZeroU32>,
    pub uid_validity: Option<NonZeroU32>,
}

pub trait TagGenerator {
    fn generate(&mut self) -> Tag<'static>;
}

pub struct SecureTagGenerator {
    inner: ImapTagGenerator,
}

impl SecureTagGenerator {
    fn new() -> Self {
        Self {
            inner: ImapTagGenerator::new(),
        }
    }
}

impl TagGenerator for SecureTagGenerator {
    fn generate(&mut self) -> Tag<'static> {
        self.inner.generate()
    }
}

pub struct ImapConnector<T, G = SecureTagGenerator>
where
    T: AsyncRead + AsyncWrite + Unpin,
    G: TagGenerator,
{
    stream: ImapStream<T>,
    client: ImapClient,
    tag_generator: G,
}

impl<T> ImapConnector<T, SecureTagGenerator>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn new(transport: T) -> Result<Self, Error> {
        Self::new_with_tag_generator(transport, SecureTagGenerator::new()).await
    }
}

/// Macro used to handle responses from the server without having to write a lot of boilerplate
/// code. The macro takes a stream, a client and an optional list of data handlers.
///
/// The macro will loop until the response is fully processed and the sever responds with
/// a tagged /// response (OK/NO/BAD/...). If the response is OK, loop is exited and the code
/// that follows is executed. If the response is NO or BAD, an error is returned immediatelly
/// and the whole function that invoked the macro is exited.
///
/// The data handler takes form of a pattern matching block that is used to match the data
/// received:
///
/// ```ignore
/// Data::Capability(capability) => { /* user code */ }
// }
/// ```
macro_rules! handle_response {
    (
        $stream:expr,
        $client:expr,
        $($data:tt)*
    )  => {
        loop {
            match $stream.next(&mut $client).await.unwrap() {
                Event::CommandSent { .. } => {},
                Event::StatusReceived { status } => match status {
                    Status::Tagged(s) => match s.body.kind {
                        StatusKind::Ok => {
                            #[allow(unused_variables)]
                            if let Some(code) = s.body.code {
                                handle_response!(@process_code code, $($data)*)
                            }
                            break
                        }
                        StatusKind::No => {
                            return Err(Error::CommandFailed(s.body.text.to_string()));
                        },
                        StatusKind::Bad => {
                            return Err(Error::BadCommandError(s.body.text.to_string()));
                        }
                    },
                    Status::Untagged(s) => match s.code {
                        #[allow(unused_variables)]
                        Some(code) => {
                            handle_response!(@process_code code, $($data)*)
                        },
                        None => {},
                    },
                    status => return Err(Error::UnexpectedResponse(format!("status {:?}", status))),
                },
                Event::DataReceived{ data } => {
                    handle_response!(@process_data data, $($data)*)
                },
                event => {
                    return Err(Error::UnexpectedResponse(format!("{:?}", event)));
                }
            }
        }
    };
    (
        $stream:expr,
        $client:expr
    ) => {
        handle_response!(
            $stream,
            $client,
        )
    };

    (@process_data $data:expr, Data::$data_pat:ident ($($data_var:ident),*) => $data_handler:expr, $($tail:tt)*) => {
        if let Data::$data_pat($($data_var),*) = $data {
            $data_handler
        } else {
            handle_response!(@process_data $data, $($tail)*)
        }
    };
    (@process_data $data:expr, Code::$data_pat:ident ($($data_var:ident),*) => $data_handler:expr, $($tail:tt)*) => {
        handle_response!(@process_data $data, $($tail)*)
    };
    (@process_data $data:expr, Code::$data_pat:ident => $data_handler:expr, $($tail:tt)*) => {
        handle_response!(@process_data $data, $($tail)*)
    };
    (@process_data $data:expr, Data::$data_pat:ident { $($data_var:ident),* } => $data_handler:expr, $($tail:tt)*) => {
        if let Data::$data_pat{ $($data_var),* } = $data {
            $data_handler
        } else {
            handle_response!(@process_data $data, $($tail)*)
        }
    };
    (@process_data $data:expr, Code::$data_pat:ident { $($data_var:ident),* } => $data_handler:expr, $($tail:tt)*) => {
        handle_response!(@process_data $data, $($tail)*)
    };
    (@process_data $data:expr,) => {
        return Err(Error::UnexpectedResponse(format!("data {:?}", $data)))
    };

    (@process_code $code:expr, Code::$code_pat:ident => $code_handler:expr, $($tail:tt)*) => {
        if let Code::$code_pat = $code {
            $code_handler
        } else {
            handle_response!(@process_code $code, $($tail)*)
        }
    };
    (@process_code $code:expr, Code::$code_pat:ident ($($code_var:ident),*) => $code_handler:expr, $($tail:tt)*) => {
        if let Code::$code_pat($($code_var),*) = $code {
            $code_handler
        } else {
            handle_response!(@process_code $code, $($tail)*)
        }
    };
    (@process_code $code:expr, Data::$code_pat:ident ($($code_var:ident),*) => $code_handler:expr, $($tail:tt)*) => {
        handle_response!(@process_code $code, $($tail)*)
    };
    (@process_code $code:expr, Code::$code_pat:ident {$($code_var:ident),*} => $code_handler:expr, $($tail:tt)*) => {
        if let Code::$code_pat{ $($code_var),* } = $code {
            $code_handler
        } else {
            handle_response!(@process_code $code, $($tail)*)
        }
    };
    (@process_code $code:expr, Data::$code_pat:ident {$($code_var:ident),*} => $code_handler:expr, $($tail:tt)*) => {
        handle_response!(@process_code $code, $($tail)*)
    };
    (@process_code $code:expr,) => {
        // no-op
        {}
    };
}

impl<T, G> ImapConnector<T, G>
where
    T: AsyncRead + AsyncWrite + Unpin,
    G: TagGenerator,
{
    pub async fn new_with_tag_generator(transport: T, tag_generator: G) -> Result<Self, Error> {
        let imap_stream = ImapStream::new(transport);
        let client = ImapClient::new(Options::default());

        let mut connector = Self {
            stream: imap_stream,
            client,
            tag_generator,
        };

        loop {
            match connector.stream.next(&mut connector.client).await.unwrap() {
                Event::GreetingReceived { .. } => {
                    // TODO: Extract initial capabilities to find out what authentication methods
                    // are supported.
                    break;
                }
                event => {
                    return Err(Error::UnexpectedResponse(format!("{:?}", event)));
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
                    CommandBody::login(username.to_string(), password.to_string())
                        .expect("Failed to construct LOGIN command body"),
                )
                .expect("Failed to construct LOGIN command");
                self.client.enqueue_command(cmd);
            }
            _ => todo!(), // TODO: Support additional authentication methods
        };

        handle_response!(self.stream, self.client);
        Ok(())
    }

    pub async fn capabilities(&mut self) -> Result<Vec<Capability>, Error> {
        let cmd = Command::new(self.next_tag(), CommandBody::Capability).unwrap();
        self.client.enqueue_command(cmd);

        let mut capabilities = Vec::<Capability>::new();

        handle_response!(
            self.stream, self.client,
            Data::Capability(capability) => capabilities.extend_from_slice(&capability.into_inner()),
        );

        Ok(capabilities)
    }

    pub async fn list_mailboxes(&mut self) -> Result<Vec<Mailbox>, Error> {
        let cmd = Command::new(
            self.next_tag(),
            CommandBody::list(
                ImapMailbox::try_from("".to_string())
                    .expect("Failed to construct Mailbox for \"*\""),
                ListMailbox::try_from("*".to_string())
                    .expect("Failed to construct ListMailbox for \"*\""),
            )
            .expect("Failed to construct LIST command body"),
        )
        .expect("Failed to construct LIST body");
        self.client.enqueue_command(cmd);

        let mut mailboxes = Vec::<Mailbox>::new();

        handle_response!(
            self.stream, self.client,
            Data::List { items, delimiter, mailbox } => {
                mailboxes.push(Mailbox {
                    flags: items.into_iter().map(|item| item.to_string()).collect(),
                    delimiter: delimiter.map(|c| c.inner()),
                    name: mailbox
                });
            },
        );

        Ok(mailboxes)
    }

    pub async fn select_mailbox(&mut self, mailbox: String) -> Result<MailboxStats, Error> {
        let cmd = Command::new(
            self.next_tag(),
            CommandBody::select(mailbox).expect("Failed to construct SELECT command body"),
        ).expect("Failed to construct SELECT command");
        self.client.enqueue_command(cmd);

        let mut mailbox_stats = MailboxStats::default();

        handle_response!(
            self.stream, self.client,
            Data::Exists(count) => mailbox_stats.exists = Some(count),
            Data::Flags(flags) => mailbox_stats.flags = Some(flags.into_iter().map(|flag| flag.to_string()).collect()),
            Data::Recent(count) => mailbox_stats.recent = Some(count),
            Code::PermanentFlags(flags) => mailbox_stats.permanent_flags = Some(flags),
            Code::ReadOnly => mailbox_stats.read_only = true,
            Code::UidNext(uid) => mailbox_stats.uid_next = Some(uid),
            Code::UidValidity(uid) => mailbox_stats.uid_validity = Some(uid),
        );

        Ok(mailbox_stats)
    }
}

#[cfg(test)]
mod testing {

    use super::*;
    use imap_types::core::Atom;
    use imap_types::flag::Flag;
    use imap_types::response::Capability;
    use tokio_test::io::Builder;

    #[derive(Default)]
    struct SequentialTagGenerator {
        counter: u32,
    }

    impl TagGenerator for SequentialTagGenerator {
        fn generate(&mut self) -> Tag<'static> {
            self.counter += 1;
            Tag::try_from(self.counter.to_string()).expect("Failed to create Tag")
        }
    }

    #[tokio::test]
    async fn test_login_success() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"1 LOGIN testuser testpassword\r\n")
            .read(b"1 OK [CAPABILITY IMAP4rev1 ID IDLE MOVE LITERAL+ QUOTA X-REALLY-SPECIAL] Logged in\r\n")
            .build();

        let mut connector =
            ImapConnector::new_with_tag_generator(stream, SequentialTagGenerator::default())
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
    async fn test_login_failed() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"1 LOGIN testuser testpassword\r\n")
            .read(b"1 NO [AUTH] Authentication failed\r\n")
            .build();

        let mut connector =
            ImapConnector::new_with_tag_generator(stream, SequentialTagGenerator::default())
                .await
                .expect("Failed to create connector");
        let result = connector
            .authenticate(AuthMethod::Login {
                username: "testuser".to_string(),
                password: "testpassword".to_string(),
            })
            .await;
        match result {
            Ok(_) => assert!(result.is_err()),
            Err(Error::CommandFailed(msg)) => assert_eq!(msg, "Authentication failed"),
            Err(_) => assert!(false),
        }
    }

    #[tokio::test]
    async fn test_capabilities() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"1 CAPABILITY\r\n")
            .read(b"* CAPABILITY IMAP4rev1 ID IDLE MOVE LITERAL+ QUOTA X-REALLY-SPECIAL\r\n")
            .read(b"1 OK Capability completed\r\n")
            .build();
        let mut connector =
            ImapConnector::new_with_tag_generator(stream, SequentialTagGenerator::default())
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

    #[tokio::test]
    async fn test_list() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"1 LIST \"\" *\r\n")
            .read(b"* LIST (\\HasChildren) \".\" INBOX\r\n")
            .read(b"* LIST (\\HasNoChildren) \".\" INBOX.Sent\r\n")
            .read(b"1 OK List completed\r\n")
            .build();
        let mut connector =
            ImapConnector::new_with_tag_generator(stream, SequentialTagGenerator::default())
                .await
                .expect("Failed to create connector");

        let result = connector
            .list_mailboxes()
            .await
            .expect("Failed to list mailboxes");
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            Mailbox {
                flags: vec!["\\HasChildren".to_string()],
                delimiter: Some('.'),
                name: ImapMailbox::try_from("INBOX".to_string()).unwrap()
            }
        );
        assert_eq!(
            result[1],
            Mailbox {
                flags: vec!["\\HasNoChildren".to_string()],
                delimiter: Some('.'),
                name: ImapMailbox::try_from("INBOX.Sent".to_string()).unwrap()
            }
        )
    }

    #[tokio::test]
    async fn test_select() {
        let stream = Builder::new()
            .read(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] Greetings!!\r\n")
            .write(b"1 SELECT INBOX\r\n")
            .read(b"* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n")
            .read(b"* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.\r\n")
            .read(b"* 23 EXISTS\r\n")
            .read(b"* 0 RECENT\r\n")
            .read(b"* OK [UIDVALIDITY 3857529045] UIDs valid\r\n")
            .read(b"* OK [UIDNEXT 23] Predicted next UID\r\n")
            .read(b"1 OK [READ-ONLY] Select completed\r\n")
            .build();
        let mut connector =
            ImapConnector::new_with_tag_generator(stream, SequentialTagGenerator::default())
                .await
                .expect("Failed to create connector");

        let result = connector
            .select_mailbox("INBOX".to_string())
            .await
            .expect("Failed to select mailbox");
        assert_eq!(result.exists, Some(23));
        assert_eq!(result.recent, Some(0));
        assert_eq!(
            result.flags,
            Some(vec![
                "\\Answered".to_string(),
                "\\Flagged".to_string(),
                "\\Deleted".to_string(),
                "\\Seen".to_string(),
                "\\Draft".to_string()
            ])
        );
        assert_eq!(
            result.permanent_flags,
            Some(vec![
                FlagPerm::Flag(Flag::Answered),
                FlagPerm::Flag(Flag::Flagged),
                FlagPerm::Flag(Flag::Deleted),
                FlagPerm::Flag(Flag::Seen),
                FlagPerm::Flag(Flag::Draft),
                FlagPerm::Asterisk,
            ])
        );
        assert_eq!(result.read_only, true);
        assert_eq!(result.uid_next, Some(NonZeroU32::new(23).unwrap()));
        assert_eq!(result.uid_validity, Some(NonZeroU32::new(3857529045).unwrap()));
    }
}
