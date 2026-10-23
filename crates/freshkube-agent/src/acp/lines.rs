//! A bound on what the agent can make the client buffer: the ACP framing is
//! one JSON message per line, and a line longer than the limit fails the
//! connection instead of growing a buffer without end.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

/// The longest line the client will read. Text arrives in chunks of at most
/// 64 KiB; the rest is tool output and arguments.
pub(super) const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct LineLimit<R> {
    inner: R,
    run: usize,
    max: usize,
}

impl<R> LineLimit<R> {
    pub(super) fn new(inner: R, max: usize) -> Self {
        Self { inner, run: 0, max }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LineLimit<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let read = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &read {
            for index in before..buf.filled().len() {
                if buf.filled()[index] == b'\n' {
                    self.run = 0;
                } else {
                    self.run += 1;
                    if self.run > self.max {
                        // Nothing past the limit is handed on.
                        buf.set_filled(before);
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "the agent sent a line longer than the limit",
                        )));
                    }
                }
            }
        }
        read
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    #[tokio::test]
    async fn a_line_past_the_limit_fails_the_read() {
        let data = [vec![b'a'; 10], vec![b'\n'], vec![b'b'; 11]].concat();
        let mut reader = LineLimit::new(&data[..], 10);
        let mut out = Vec::new();
        let error = reader.read_to_end(&mut out).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        // Lines of exactly the limit, however many, pass.
        let ok = [vec![b'a'; 10], vec![b'\n'], vec![b'b'; 10], vec![b'\n']].concat();
        let mut out = Vec::new();
        LineLimit::new(&ok[..], 10)
            .read_to_end(&mut out)
            .await
            .unwrap();
        assert_eq!(out, ok);
    }
}
