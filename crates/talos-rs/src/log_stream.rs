//! Pull-based log framing: dropping the stream drops the source immediately.

use futures::{Stream, StreamExt, stream};

use crate::error::TalosError;

const MAX_LINE_BYTES: usize = 64 * 1024;

pub(crate) fn decode_log_chunk(data: crate::proto::common::Data) -> Result<Vec<u8>, TalosError> {
    validate_metadata(data.metadata.as_ref())?;
    Ok(data.bytes)
}

/// Aggregated Talos replies can carry proxy failures inside an otherwise OK RPC.
pub(crate) fn validate_metadata(
    metadata: Option<&crate::proto::common::Metadata>,
) -> Result<(), TalosError> {
    if let Some(metadata) = metadata {
        let status = metadata.status.as_ref().filter(|status| status.code != 0);
        if !metadata.error.is_empty() || status.is_some() {
            let code = status.map_or(tonic::Code::Unknown, |status| {
                tonic::Code::from_i32(status.code)
            });
            let message = if metadata.error.is_empty() {
                status
                    .map(|status| status.message.clone())
                    .unwrap_or_default()
            } else {
                metadata.error.clone()
            };
            return Err(TalosError::Grpc(tonic::Status::new(code, message)));
        }
    }
    Ok(())
}

/// Keep at most one transport chunk and one bounded partial line. Unlike a
/// channel-backed worker, this consumes the transport only when callers poll.
pub(crate) fn frame_log_lines<S>(source: S) -> impl Stream<Item = Result<String, TalosError>> + Send
where
    S: Stream<Item = Result<Vec<u8>, TalosError>> + Send + 'static,
{
    let state = (
        Box::pin(source.fuse()),
        Vec::<u8>::new(),
        Vec::<u8>::new(),
        0,
    );
    stream::try_unfold(
        state,
        |(mut source, mut pending, mut chunk, mut cursor)| async move {
            loop {
                if cursor < chunk.len() {
                    let remaining = &chunk[cursor..];
                    let newline = remaining.iter().position(|byte| *byte == b'\n');
                    let count = newline.unwrap_or(remaining.len());
                    if pending.len().saturating_add(count) > MAX_LINE_BYTES {
                        return Err(TalosError::Grpc(tonic::Status::resource_exhausted(
                            "Talos log line exceeded the 64 KiB safety limit",
                        )));
                    }
                    pending.extend_from_slice(&remaining[..count]);
                    cursor += count + usize::from(newline.is_some());
                    if newline.is_some() {
                        let line = String::from_utf8_lossy(&pending)
                            .trim_end_matches('\r')
                            .to_owned();
                        pending.clear();
                        if !line.trim().is_empty() {
                            return Ok(Some((line, (source, pending, chunk, cursor))));
                        }
                    }
                    continue;
                }
                match source.next().await {
                    Some(Ok(bytes)) => {
                        chunk = bytes;
                        cursor = 0;
                    }
                    Some(Err(error)) => return Err(error),
                    None => {
                        let line = String::from_utf8_lossy(&pending)
                            .trim_end_matches('\r')
                            .to_owned();
                        if line.trim().is_empty() {
                            return Ok(None);
                        }
                        pending.clear();
                        return Ok(Some((line, (source, pending, Vec::new(), 0))));
                    }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[test]
    fn propagates_proxy_metadata_errors_instead_of_rendering_undefined_bytes() {
        let data = crate::proto::common::Data {
            metadata: Some(crate::proto::common::Metadata {
                hostname: "node".into(),
                error: "service unavailable".into(),
                status: Some(crate::proto::google::rpc::Status {
                    code: 14,
                    message: "unavailable".into(),
                    details: vec![],
                }),
            }),
            bytes: b"undefined".to_vec(),
        };
        let TalosError::Grpc(status) = decode_log_chunk(data).unwrap_err() else {
            panic!("expected status");
        };
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(status.message(), "service unavailable");
    }

    #[test]
    fn propagates_status_only_proxy_failure_and_accepts_successful_bytes() {
        let mut data = crate::proto::common::Data {
            metadata: Some(crate::proto::common::Metadata {
                hostname: "node".into(),
                error: String::new(),
                status: Some(crate::proto::google::rpc::Status {
                    code: 7,
                    message: "permission denied".into(),
                    details: vec![],
                }),
            }),
            bytes: b"valid".to_vec(),
        };
        let TalosError::Grpc(status) = decode_log_chunk(data.clone()).unwrap_err() else {
            panic!("expected status");
        };
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(status.message(), "permission denied");
        data.metadata
            .as_mut()
            .unwrap()
            .status
            .as_mut()
            .unwrap()
            .code = 0;
        assert_eq!(decode_log_chunk(data).unwrap(), b"valid");
    }

    #[tokio::test]
    async fn frames_split_utf8_crlf_and_final_partial_line() {
        let source = stream::iter(vec![
            Ok(b"first\r\n\nca\xc3".to_vec()),
            Ok(b"\xa9\nlast".to_vec()),
        ]);
        let lines: Vec<_> = frame_log_lines(source).collect().await;
        assert_eq!(
            lines.into_iter().map(Result::unwrap).collect::<Vec<_>>(),
            ["first", "caé", "last"]
        );
    }

    #[tokio::test]
    async fn reports_source_error_instead_of_clean_end() {
        let source = stream::iter(vec![Err(TalosError::Grpc(tonic::Status::unavailable(
            "offline",
        )))]);
        let lines: Vec<_> = frame_log_lines(source).collect().await;
        assert!(
            lines[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
    }

    #[tokio::test]
    async fn rejects_unbounded_partial_lines() {
        let source = stream::iter(vec![Ok(vec![b'x'; MAX_LINE_BYTES]), Ok(vec![b'x'])]);
        let lines: Vec<_> = frame_log_lines(source).collect().await;
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("64 KiB")
        );
    }

    #[tokio::test]
    async fn accepts_line_at_exact_limit_across_chunks() {
        let source = stream::iter(vec![
            Ok(vec![b'x'; MAX_LINE_BYTES]),
            Ok(b"\nnext\n".to_vec()),
        ]);
        let lines: Vec<_> = frame_log_lines(source).collect().await;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].as_ref().unwrap().len(), MAX_LINE_BYTES);
        assert_eq!(lines[1].as_ref().unwrap(), "next");
    }

    #[tokio::test]
    async fn backpressure_does_not_poll_next_chunk_until_needed() {
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = polls.clone();
        let source =
            stream::iter([Ok(b"a\nb\n".to_vec()), Ok(b"c\n".to_vec())]).inspect(move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
            });
        let lines = frame_log_lines(source);
        futures::pin_mut!(lines);
        assert_eq!(lines.next().await.unwrap().unwrap(), "a");
        assert_eq!(lines.next().await.unwrap().unwrap(), "b");
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }

    struct IdleSource(Arc<AtomicBool>);

    impl Stream for IdleSource {
        type Item = Result<Vec<u8>, TalosError>;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Pending
        }
    }

    impl Drop for IdleSource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_idle_stream_drops_transport_without_waiting_for_a_line() {
        let dropped = Arc::new(AtomicBool::new(false));
        let lines = frame_log_lines(IdleSource(dropped.clone()));
        drop(lines);
        assert!(dropped.load(Ordering::SeqCst));
    }
}
