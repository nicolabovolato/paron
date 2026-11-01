use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

pub struct Deadline<S> {
    stream: S,
    max_idle: Duration,
    timer: Pin<Box<tokio::time::Sleep>>,
}

impl<S> Deadline<S> {
    pub fn new(stream: S, max_idle: Duration) -> Self {
        Self {
            stream,
            max_idle,
            timer: Box::pin(tokio::time::sleep(max_idle)),
        }
    }

    fn postpone(&mut self) {
        self.timer.as_mut().reset(Instant::now() + self.max_idle);
    }

    fn or_expired<T>(&mut self, polled: Poll<Result<T>>, cx: &mut Context<'_>) -> Poll<Result<T>> {
        if self.timer.as_mut().poll(cx).is_pending() {
            return polled;
        }

        Poll::Ready(Err(Error::new(
            ErrorKind::TimedOut,
            "the connection was idle",
        )))
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Deadline<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let read = Pin::new(&mut this.stream).poll_read(cx, buf);

        if buf.filled().len() > before {
            this.postpone();
        }

        this.or_expired(read, cx)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Deadline<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = self.get_mut();
        let written = Pin::new(&mut this.stream).poll_write(cx, buf);

        if matches!(written, Poll::Ready(Ok(bytes)) if bytes > 0) {
            this.postpone();
        }

        this.or_expired(written, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize>> {
        let this = self.get_mut();
        let written = Pin::new(&mut this.stream).poll_write_vectored(cx, buffers);

        if matches!(written, Poll::Ready(Ok(bytes)) if bytes > 0) {
            this.postpone();
        }

        this.or_expired(written, cx)
    }

    // Without this hyper writes the head and the body separately.
    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();
        let flushed = Pin::new(&mut this.stream).poll_flush(cx);

        this.or_expired(flushed, cx)
    }

    /// The one thing a passed deadline does not stop, so that a connection being closed can finish
    /// saying so rather than leaving the peer to find out from a bare FIN.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();

        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    const MAX_IDLE: Duration = Duration::from_secs(60);

    #[tokio::test(start_paused = true)]
    async fn read_quiet_stream_times_out_after_max_idle() {
        let (_peer, stream) = tokio::io::duplex(64);
        let mut deadline = Deadline::new(stream, MAX_IDLE);
        let started = Instant::now();

        let error = deadline.read(&mut [0u8; 8]).await.unwrap_err();

        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert_eq!(started.elapsed(), MAX_IDLE);
    }

    #[tokio::test(start_paused = true)]
    async fn read_with_traffic_postpones_timeout() {
        let (mut peer, stream) = tokio::io::duplex(64);
        let mut deadline = Deadline::new(stream, MAX_IDLE);
        tokio::spawn(async move {
            for _ in 0..3 {
                tokio::time::sleep(MAX_IDLE * 3 / 4).await;
                peer.write_all(b"x").await.unwrap();
            }

            std::future::pending::<()>().await;
        });

        let mut received = [0u8; 3];

        let read = deadline.read_exact(&mut received).await;

        assert_eq!(read.unwrap(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn flush_does_not_postpone_timeout() {
        let (_peer, stream) = tokio::io::duplex(64);
        let mut deadline = Deadline::new(stream, MAX_IDLE);
        let started = Instant::now();
        tokio::time::sleep(MAX_IDLE / 2).await;
        deadline.flush().await.unwrap();

        let error = deadline.read(&mut [0u8; 8]).await.unwrap_err();

        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert_eq!(started.elapsed(), MAX_IDLE);
    }

    #[tokio::test(start_paused = true)]
    async fn read_closed_stream_returns_zero_bytes() {
        let (peer, stream) = tokio::io::duplex(64);
        let mut deadline = Deadline::new(stream, MAX_IDLE);
        drop(peer);

        let read = deadline.read(&mut [0u8; 8]).await.unwrap();

        assert_eq!(read, 0);
    }
}
