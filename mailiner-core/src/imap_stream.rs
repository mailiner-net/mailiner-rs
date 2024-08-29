use std::{borrow::BorrowMut, cell::RefCell, convert::Infallible, ops::Deref, pin::Pin};

use bytes::{Buf, BytesMut};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    select,
};

use imap_next::{Interrupt, Io, State};

pub struct ImapStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    stream: Pin<Box<S>>,
    read_buffer: BytesMut,
    write_buffer: BytesMut,
}

impl<S> ImapStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(transport: S) -> Self {
        Self {
            stream: Pin::new(Box::new(transport)),
            read_buffer: BytesMut::default(),
            write_buffer: BytesMut::default(),
        }
    }

    pub async fn flush(&mut self) -> Result<(), Error<Infallible>> {
        // Flush TCP
        write(&mut self.stream, &mut self.write_buffer).await?;
        self.stream.flush().await?;

        Ok(())
    }

    pub async fn next<F: State>(&mut self, mut state: F) -> Result<F::Event, Error<F::Error>> {
        let event = loop {
            // Provide input bytes to the client/server
            if !self.read_buffer.is_empty() {
                state.enqueue_input(&self.read_buffer);
                self.read_buffer.clear();
            }

            // Progress the client/server
            let result = state.next();

            // Return events immediately without doing IO
            let interrupt = match result {
                Err(interrupt) => interrupt,
                Ok(event) => break event,
            };

            // Return errors immediately without doing IO
            let io = match interrupt {
                Interrupt::Io(io) => io,
                Interrupt::Error(err) => return Err(Error::State(err)),
            };

            // Handle the output bytes from the client/server
            if let Io::Output(bytes) = io {
                self.write_buffer.extend(bytes);
            }

            // Progress the stream
            if self.write_buffer.is_empty() {
                read(&mut self.stream, &mut self.read_buffer).await?;
            } else {
                // We read and write the stream simultaneously because otherwise
                // a deadlock between client and server might occur if both sides
                // would only read or only write.
                let (mut read_stream, mut write_stream) = tokio::io::split(self.stream.borrow_mut());
                select! {
                    result = read(&mut read_stream, &mut self.read_buffer) => result,
                    result = write(&mut write_stream, &mut self.write_buffer) => result,
                }?;
            };
        };

        Ok(event)
    }
}

/// Error during reading into or writing from a stream.
#[derive(Debug, Error)]
pub enum Error<E> {
    /// Operation failed because stream is closed.
    ///
    /// We detect this by checking if the read or written byte count is 0. Whether the stream is
    /// closed indefinitely or temporarily depends on the actual stream implementation.
    #[error("Stream was closed")]
    Closed,
    /// An I/O error occurred in the underlying stream.
    #[error(transparent)]
    Io(#[from] tokio::io::Error),
    /// An error occurred while progressing the state.
    #[error(transparent)]
    State(E),
}

async fn read<S: AsyncRead + Unpin>(
    stream: &mut S,
    read_buffer: &mut BytesMut,
) -> Result<(), ReadWriteError> {
    let byte_count = stream.read_buf(read_buffer).await?;

    if byte_count == 0 {
        // The result is 0 if the stream reached "end of file" or the read buffer was
        // already full before calling `read_buf`. Because we use an unlimited buffer we
        // know that the first case occurred.
        return Err(ReadWriteError::Closed);
    }

    Ok(())
}

async fn write<S: AsyncWrite + Unpin>(
    stream: &mut S,
    write_buffer: &mut BytesMut,
) -> Result<(), ReadWriteError> {
    while !write_buffer.is_empty() {
        let byte_count = stream.write(write_buffer).await?;
        write_buffer.advance(byte_count);

        if byte_count == 0 {
            // The result is 0 if the stream doesn't accept bytes anymore or the write buffer
            // was already empty before calling `write_buf`. Because we checked the buffer
            // we know that the first case occurred.
            return Err(ReadWriteError::Closed);
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
enum ReadWriteError {
    #[error("Stream was closed")]
    Closed,
    #[error(transparent)]
    Io(#[from] tokio::io::Error),
}

impl<E> From<ReadWriteError> for Error<E> {
    fn from(value: ReadWriteError) -> Self {
        match value {
            ReadWriteError::Closed => Error::Closed,
            ReadWriteError::Io(err) => Error::Io(err),
        }
    }
}
