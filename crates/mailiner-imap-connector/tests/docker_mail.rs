//! Live checks against the docker-mail container.
//!
//! Skipped unless `MAILINER_IT=1`. Start the stack first:
//! `docker compose up --build --wait` from the repo root.
//! IMAP host on the certificate is `mail`; the published port is 127.0.0.1:993.
//!
//! The suite shares the `dev@mailiner.test` mailbox and does not re-seed it.
//! Tests that append or submit mail expunge those messages when they finish.

use std::process::{Command, Stdio};
use std::time::Duration;

use mailiner_core::{
    AccountId, BodyPart, EmailConnector, Envelope, Folder, MailboxRole, MailinerError,
    MessageListFilter, MessageSort, SubmitRequest,
};
use mailiner_imap_connector::{ImapConnector, MailboxWatchOutcome};
use mailiner_smtp_connector::SmtpConnector;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

const USER: &str = "dev@mailiner.test";
const PASSWORD: &str = "dev";

fn docker_enabled() -> bool {
    std::env::var("MAILINER_IT").ok().as_deref() == Some("1")
}

async fn connect_imap(password: &str) -> ImapConnector<TcpStream> {
    let imap = ImapConnector::new(AccountId::new("it"), "mail".into(), 993, USER.into());
    let tcp = TcpStream::connect(("127.0.0.1", 993))
        .await
        .expect("docker-mail IMAP on 127.0.0.1:993");
    imap.connect(tcp).await.expect("IMAP TLS connect");
    imap.authenticate(password)
        .await
        .expect("IMAP authenticate");
    imap
}

async fn folder(imap: &ImapConnector<TcpStream>, role: MailboxRole) -> Folder {
    let folders = imap
        .list_folders(&AccountId::new("it"))
        .await
        .expect("LIST");
    folders
        .into_iter()
        .find(|folder| folder.role == role)
        .unwrap_or_else(|| panic!("missing folder {role:?}"))
}

async fn envelopes(
    imap: &ImapConnector<TcpStream>,
    folder: &Folder,
    search: &str,
) -> Vec<Envelope> {
    let state = imap
        .prepare_folder_list(
            &folder.id,
            MessageSort::Arrival,
            MessageListFilter::default(),
            search,
        )
        .await
        .expect("prepare folder");
    if state.total == 0 {
        return Vec::new();
    }
    let end = state.total.min(30);
    imap.list_envelopes_range(&folder.id, 0..end)
        .await
        .expect("envelopes")
}

fn collect_descriptors(part: &BodyPart, out: &mut Vec<String>) {
    out.push(part.content_type());
    if let Some(name) = part.filename() {
        out.push(name.to_string());
    }
    if let Some(id) = &part.id {
        out.push(id.clone());
    }
    if let Some(encoding) = &part.encoding {
        out.push(encoding.to_ascii_uppercase());
    }
    for child in &part.subparts {
        collect_descriptors(child, out);
    }
}

fn mail_container() -> String {
    let output = Command::new("docker")
        .args(["ps", "-q", "--filter", "publish=993"])
        .output()
        .expect("docker ps");
    let id = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    assert!(
        output.status.success() && !id.is_empty(),
        "no mail container publishing 993: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    id
}

fn unique_subject(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{nanos}-{}", std::process::id())
}

/// Expunge messages this test added. `subject` must not be a substring of a
/// seed subject in `mailbox` (doveadm SUBJECT is a substring match).
fn expunge_subject(mailbox: &str, subject: &str) {
    let output = Command::new("docker")
        .args(["ps", "-q", "--filter", "publish=993"])
        .output();
    let Ok(output) = output else {
        eprintln!("cleanup: docker ps failed; left {mailbox} subject {subject}");
        return;
    };
    let id = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if !output.status.success() || id.is_empty() {
        eprintln!("cleanup: no mail container; left {mailbox} subject {subject}");
        return;
    }
    match Command::new("docker")
        .args([
            "exec", &id, "doveadm", "expunge", "-u", USER, "mailbox", mailbox, "subject", subject,
        ])
        .output()
    {
        Ok(result) if result.status.success() => {}
        Ok(result) => {
            let err = String::from_utf8_lossy(&result.stderr);
            if !err.contains("doesn't exist") {
                eprintln!("cleanup expunge {mailbox} {subject}: {err}");
            }
        }
        Err(err) => eprintln!("cleanup expunge {mailbox} {subject}: {err}"),
    }
}

struct SubjectCleanup {
    mailboxes: &'static [&'static str],
    subject: String,
}

impl Drop for SubjectCleanup {
    fn drop(&mut self) {
        for mailbox in self.mailboxes {
            expunge_subject(mailbox, &self.subject);
        }
    }
}

fn deliver(mailbox: &str, raw: &str) {
    let crlf = if raw.contains("\r\n") {
        raw.to_string()
    } else {
        raw.replace('\n', "\r\n")
    };
    let mut child = Command::new("docker")
        .args([
            "exec",
            "-i",
            &mail_container(),
            "/usr/libexec/dovecot/dovecot-lda",
            "-d",
            USER,
            "-m",
            mailbox,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("docker compose exec");
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(crlf.as_bytes())
        .expect("write message");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait lda");
    assert!(
        output.status.success(),
        "dovecot-lda failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seed_messages_have_the_expected_structure() {
    if !docker_enabled() {
        eprintln!("skip seed_messages_have_the_expected_structure: set MAILINER_IT=1");
        return;
    }
    let imap = connect_imap(PASSWORD).await;
    let inbox = folder(&imap, MailboxRole::Inbox).await;

    let welcome = envelopes(&imap, &inbox, "subject:\"Welcome to Mailiner\"")
        .await
        .into_iter()
        .next()
        .expect("welcome");
    assert!(welcome.is_read);
    let raw = imap
        .fetch_raw_message(&inbox.id, &welcome.id)
        .await
        .expect("raw welcome");
    let raw = String::from_utf8_lossy(&raw);
    assert!(raw.contains("This is a plain-text fixture used by the local mail container."));

    let notes = envelopes(
        &imap,
        &inbox,
        "subject:\"Please review the attached notes\"",
    )
    .await
    .into_iter()
    .next()
    .expect("notes");
    assert!(notes.is_flagged);
    assert!(notes.has_attachments);
    let tree = imap
        .get_body_structure(&inbox.id, &notes.id)
        .await
        .expect("notes structure");
    let mut desc = Vec::new();
    collect_descriptors(&tree, &mut desc);
    assert!(desc.iter().any(|d| d == "notes.txt"), "{desc:?}");
    assert!(desc.iter().any(|d| d == "multipart/mixed"), "{desc:?}");

    let two = envelopes(&imap, &inbox, "subject:\"Two attachments (txt + csv)\"")
        .await
        .into_iter()
        .next()
        .expect("two attachments");
    let tree = imap
        .get_body_structure(&inbox.id, &two.id)
        .await
        .expect("two structure");
    let mut desc = Vec::new();
    collect_descriptors(&tree, &mut desc);
    assert!(desc.iter().any(|d| d == "readme.txt"), "{desc:?}");
    assert!(desc.iter().any(|d| d == "invoice.csv"), "{desc:?}");

    let inline = envelopes(&imap, &inbox, "subject:\"Logo proof (inline image)\"")
        .await
        .into_iter()
        .next()
        .expect("inline");
    let tree = imap
        .get_body_structure(&inbox.id, &inline.id)
        .await
        .expect("inline structure");
    let mut desc = Vec::new();
    collect_descriptors(&tree, &mut desc);
    assert!(
        desc.iter().any(|d| d.contains("pixel@mailiner.test")),
        "{desc:?}"
    );

    let cafe = envelopes(&imap, &inbox, "subject:Café")
        .await
        .into_iter()
        .next()
        .expect("cafe");
    assert!(
        cafe.subject.as_deref().unwrap_or("").contains("Café"),
        "{:?}",
        cafe.subject
    );

    let b64 = envelopes(&imap, &inbox, "subject:\"Base64-encoded plain text\"")
        .await
        .into_iter()
        .next()
        .expect("base64");
    let tree = imap
        .get_body_structure(&inbox.id, &b64.id)
        .await
        .expect("base64 structure");
    let mut desc = Vec::new();
    collect_descriptors(&tree, &mut desc);
    assert!(desc.iter().any(|d| d == "text/plain"), "{desc:?}");
    assert!(desc.iter().any(|d| d == "BASE64"), "{desc:?}");

    let nested = envelopes(
        &imap,
        &inbox,
        "subject:\"Nested newsletter (mixed + related + alternative)\"",
    )
    .await
    .into_iter()
    .next()
    .expect("newsletter");
    let tree = imap
        .get_body_structure(&inbox.id, &nested.id)
        .await
        .expect("newsletter structure");
    let mut desc = Vec::new();
    collect_descriptors(&tree, &mut desc);
    assert!(desc.iter().any(|d| d == "multipart/mixed"), "{desc:?}");
    assert!(desc.iter().any(|d| d == "multipart/related"), "{desc:?}");
    assert!(
        desc.iter().any(|d| d == "multipart/alternative"),
        "{desc:?}"
    );
    assert!(desc.iter().any(|d| d == "standup.ics"), "{desc:?}");
    assert!(
        desc.iter().any(|d| d.contains("badge@mailiner.test")),
        "{desc:?}"
    );

    let quiet = envelopes(&imap, &inbox, "from:quiet@example.com")
        .await
        .into_iter()
        .next()
        .expect("no subject");
    assert!(quiet.subject.as_deref().unwrap_or("").trim().is_empty());

    let drafts = folder(&imap, MailboxRole::Drafts).await;
    let draft = envelopes(&imap, &drafts, "subject:\"half-written reply\"")
        .await
        .into_iter()
        .next()
        .expect("draft");
    assert!(draft.is_draft || draft.subject.as_deref().unwrap_or("").contains("draft"));

    let sent = folder(&imap, MailboxRole::Sent).await;
    assert!(
        !envelopes(&imap, &sent, "subject:\"Sent copy: lunch confirmed\"")
            .await
            .is_empty()
    );
    let trash = folder(&imap, MailboxRole::Trash).await;
    assert!(!envelopes(&imap, &trash, "subject:\"Already in Trash\"")
        .await
        .is_empty());
}

#[tokio::test]
async fn wrong_password_fails_at_auth() {
    if !docker_enabled() {
        eprintln!("skip wrong_password_fails_at_auth: set MAILINER_IT=1");
        return;
    }
    let imap = ImapConnector::new(AccountId::new("it"), "mail".into(), 993, USER.into());
    let tcp = TcpStream::connect(("127.0.0.1", 993))
        .await
        .expect("connect");
    imap.connect(tcp).await.expect("tls");
    let err = imap.authenticate("wrong").await.expect_err("auth");
    assert!(
        matches!(err, MailinerError::Auth(_)),
        "expected auth failure, got {err}"
    );
}

#[tokio::test]
async fn public_roots_reject_the_test_certificate() {
    if !docker_enabled() {
        eprintln!("skip public_roots_reject_the_test_certificate: set MAILINER_IT=1");
        return;
    }
    let store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    let connector = TlsConnector::from(std::sync::Arc::new(config));
    let tcp = TcpStream::connect(("127.0.0.1", 993))
        .await
        .expect("connect");
    let name = ServerName::try_from("mail").expect("sni");
    let err = connector.connect(name, tcp).await.expect_err("handshake");
    let message = err.to_string().to_ascii_lowercase();
    assert!(
        message.contains("certificate") || message.contains("unknown") || message.contains("ca"),
        "{err}"
    );
}

#[tokio::test]
async fn smtp_submit_is_readable_over_imap() {
    if !docker_enabled() {
        eprintln!("skip smtp_submit_is_readable_over_imap: set MAILINER_IT=1");
        return;
    }
    let cleanup = SubjectCleanup {
        mailboxes: &["INBOX", "Sent"],
        subject: unique_subject("it-smtp"),
    };
    let subject = cleanup.subject.as_str();
    let raw = format!(
        "From: Dev User <{USER}>\r\n\
         To: Dev User <{USER}>\r\n\
         Subject: {subject}\r\n\
         Message-ID: <{subject}@it.mailiner.test>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\
         \r\n\
         --b\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         hello integration\r\n\
         --b\r\n\
         Content-Type: text/plain; name=\"note.txt\"\r\n\
         Content-Disposition: attachment; filename=\"note.txt\"\r\n\
         \r\n\
         attachment-bytes-it\r\n\
         --b--\r\n"
    );
    let smtp = SmtpConnector::new(
        AccountId::new("it"),
        "mail".into(),
        465,
        USER.into(),
        "mailiner.test".into(),
    );
    let tcp = TcpStream::connect(("127.0.0.1", 465))
        .await
        .expect("smtp port");
    let tls = smtp.wrap_tls(tcp).await.expect("smtp tls");
    smtp.submit(
        tls,
        PASSWORD,
        &SubmitRequest {
            mail_from: USER.into(),
            rcpt_to: vec![USER.into()],
            rfc822: raw.into_bytes(),
            message_id: format!("<{subject}@it.mailiner.test>"),
            dsn: None,
        },
    )
    .await
    .expect("smtp submit");

    let imap = connect_imap(PASSWORD).await;
    let inbox = folder(&imap, MailboxRole::Inbox).await;
    let found = envelopes(&imap, &inbox, &format!("subject:\"{subject}\""))
        .await
        .into_iter()
        .next()
        .expect("submitted message");
    let bytes = imap
        .fetch_raw_message(&inbox.id, &found.id)
        .await
        .expect("fetch submitted");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("hello integration"));
    assert!(text.contains("attachment-bytes-it"));
    let tree = imap
        .get_body_structure(&inbox.id, &found.id)
        .await
        .expect("structure");
    let mut desc = Vec::new();
    collect_descriptors(&tree, &mut desc);
    assert!(desc.iter().any(|d| d == "note.txt"), "{desc:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_reports_a_message_appended_during_the_wait() {
    if !docker_enabled() {
        eprintln!("skip idle_reports_a_message_appended_during_the_wait: set MAILINER_IT=1");
        return;
    }
    let imap = connect_imap(PASSWORD).await;
    assert!(imap.supports_idle(), "Dovecot should advertise IDLE");
    let inbox = folder(&imap, MailboxRole::Inbox).await;
    imap.prepare_folder_list(
        &inbox.id,
        MessageSort::Arrival,
        MessageListFilter::default(),
        "",
    )
    .await
    .expect("select inbox");

    let cleanup = SubjectCleanup {
        mailboxes: &["INBOX"],
        subject: unique_subject("it-idle"),
    };
    let subject = cleanup.subject.as_str();
    let raw = format!(
        "From: Alice Example <alice@example.com>\nTo: Dev User <{USER}>\nSubject: {subject}\n\nidle\n"
    );
    let (outcome, _) = tokio::join!(
        imap.watch_mailbox(
            &inbox.id,
            std::future::pending::<()>(),
            tokio::time::sleep(Duration::from_secs(20)),
        ),
        async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            tokio::task::spawn_blocking(move || deliver("INBOX", &raw))
                .await
                .expect("spawn deliver");
        }
    );
    let outcome = outcome.expect("watch");
    assert!(
        matches!(outcome, MailboxWatchOutcome::Changed(change) if change.exists),
        "{outcome:?}"
    );
}
