//! Structure → parse → selective FETCH → TE decode pipeline.

use std::fmt::Debug;

use mailiner_core::connector::EmailConnector;
use mailiner_core::error::{MailinerError, Result};
use mailiner_core::ids::{FolderId, MessageId};
use mailiner_core::models::{LoadedMessage, MessageContent};
use mailiner_mime::{MessageParser, decode_part_content};
use tokio::io::{AsyncRead, AsyncWrite};

/// Load a message: BODYSTRUCTURE → parse parts → FETCH content sections → decode.
pub async fn load_message<S, C>(
    connector: &C,
    folder_id: &FolderId,
    message_id: &MessageId,
) -> Result<LoadedMessage>
where
    C: EmailConnector<S>,
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send + Sync,
{
    let structure = connector.get_body_structure(folder_id, message_id).await?;
    let parser = MessageParser::with_defaults();
    let mut parts = parser.parse(message_id, &structure);

    let mut sections: Vec<String> = parts
        .iter()
        .filter(|p| p.should_prefetch())
        .map(|p| p.section())
        .collect();
    sections.sort();
    sections.dedup();

    if sections.is_empty() {
        return Ok(LoadedMessage {
            envelope_id: message_id.clone(),
            folder_id: folder_id.clone(),
            parts,
        });
    }

    let raw = connector
        .fetch_raw_parts(folder_id, message_id, &sections)
        .await?;

    let mut missing = Vec::new();
    for part in &mut parts {
        if !part.should_prefetch() {
            continue;
        }
        let sec = part.section();
        match raw.get(&sec) {
            Some(bytes) => match decode_part_content(
                bytes,
                part.encoding,
                &part.content_type,
                part.charset.as_deref(),
            ) {
                Ok(content) => {
                    part.content = content;
                }
                Err(_) => {
                    missing.push(sec);
                }
            },
            None => {
                missing.push(sec);
            }
        }
    }

    let any_content = parts
        .iter()
        .any(|p| p.should_prefetch() && !matches!(p.content, MessageContent::Empty));
    if !any_content && !sections.is_empty() {
        return Err(MailinerError::Connector(format!(
            "failed to load content sections: {}",
            missing.join(", ")
        )));
    }

    Ok(LoadedMessage {
        envelope_id: message_id.clone(),
        folder_id: folder_id.clone(),
        parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::connector::MockConnector;
    use mailiner_core::models::PartKind;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    #[derive(Debug)]
    struct NullStream;

    impl AsyncRead for NullStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for NullStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn loads_multipart_prefers_html_content() {
        let connector = MockConnector::new();
        let folder = FolderId::new("inbox");
        let msg = MessageId::new("1");
        let loaded = block_on(load_message::<NullStream, _>(&connector, &folder, &msg)).unwrap();

        assert_eq!(loaded.envelope_id, msg);
        // Prefetched content: plain + html (not the pdf attachment).
        let html = loaded
            .parts
            .iter()
            .find(|p| p.kind == PartKind::TextHtml)
            .expect("html part");
        match &html.content {
            MessageContent::Text(t) => assert!(t.contains("HTML")),
            other => panic!("expected text, got {:?}", other),
        }
        let att = loaded
            .parts
            .iter()
            .find(|p| p.is_attachment && !p.is_hidden)
            .expect("attachment");
        assert!(matches!(att.content, MessageContent::Empty));
        assert!(loaded.attachments().count() >= 1);
    }

    #[test]
    fn content_parts_decoded_attachment_not_fetched() {
        let connector = MockConnector::new();
        let folder = FolderId::new("inbox");
        let msg = MessageId::new("42");
        let loaded = block_on(load_message::<NullStream, _>(&connector, &folder, &msg)).unwrap();

        for p in loaded.attachments() {
            assert!(matches!(p.content, MessageContent::Empty));
        }
        assert!(
            loaded
                .content_parts()
                .any(|p| !matches!(p.content, MessageContent::Empty))
        );
    }
}
