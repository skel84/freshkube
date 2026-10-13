//! Error formatting utilities
//!
//! Provides user-friendly error message formatting for Talos API errors.

use talos_rs::TalosError;

/// Format a TalosError into a user-friendly message
///
/// Transforms technical error messages into actionable, human-readable text.
///
/// # Examples
///
/// ```
/// use freshkube_core::errors::format_talos_error;
/// use talos_rs::TalosError;
///
/// let error = TalosError::Connection("connection refused".to_string());
/// let message = format_talos_error(&error);
/// assert_eq!(message, "Connection refused - is the node reachable?");
/// ```
pub fn format_talos_error(error: &TalosError) -> String {
    match error {
        TalosError::Connection(msg) => format_connection_error(msg),
        TalosError::Grpc(status) => format_grpc_error(status),
        TalosError::Transport(e) => format_transport_error(e),
        TalosError::Tls(msg) => {
            format!("TLS error: {} - check talosconfig credentials", msg)
        }
        TalosError::ConfigNotFound(path) => {
            format!("Config not found: {}", path)
        }
        TalosError::ConfigInvalid(msg) => {
            format!("Invalid config: {}", msg)
        }
        TalosError::ContextNotFound(ctx) => {
            format!("Context '{}' not found in talosconfig", ctx)
        }
        _ => error.to_string(),
    }
}

/// Format a connection error message.
///
/// The one text check left in this file: `TalosError::Connection` carries
/// only text from talos-rs (talosctl's stderr, a node's own message), so there
/// is no type or source chain to classify it by. Everything with a type is
/// worded by its type.
fn format_connection_error(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl") {
        "TLS/certificate error - check talosconfig credentials".to_string()
    } else if lower.contains("refused") {
        "Connection refused - is the node reachable?".to_string()
    } else if lower.contains("timeout") {
        "Connection timed out - node may be slow or unreachable".to_string()
    } else if lower.contains("dns") || lower.contains("resolve") {
        "DNS resolution failed - check node hostname/IP".to_string()
    } else {
        format!("Connection failed: {}", msg)
    }
}

/// Format a gRPC status error, by its code and never its message. A known
/// code's words come first, followed by the server's own message when it
/// sent one.
fn format_grpc_error(status: &tonic::Status) -> String {
    let words = match status.code() {
        tonic::Code::Unavailable => "Service unavailable - node may be down",
        tonic::Code::PermissionDenied => "Permission denied - check RBAC/credentials",
        tonic::Code::Unauthenticated => "Authentication failed - check talosconfig",
        tonic::Code::DeadlineExceeded => "Request timed out",
        tonic::Code::NotFound => "Resource not found",
        tonic::Code::AlreadyExists => "Resource already exists",
        _ => return format!("gRPC error: {}", status.message()),
    };
    match status.message() {
        "" => words.to_string(),
        detail => format!("{words}: {detail}"),
    }
}

/// The kind of the first I/O error in an error's source chain, which is
/// where hyper and the OS report a refused, reset or timed-out connection.
fn io_kind(error: &(dyn std::error::Error + 'static)) -> Option<std::io::ErrorKind> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            return Some(io.kind());
        }
        current = error.source();
    }
    None
}

/// Format a transport error, by the kind of I/O error behind it and never its
/// text.
fn format_transport_error(error: &(dyn std::error::Error + 'static)) -> String {
    use std::io::ErrorKind;
    match io_kind(error) {
        Some(ErrorKind::ConnectionRefused) => "Connection refused - is the node reachable?".into(),
        Some(ErrorKind::TimedOut) => "Connection timed out".into(),
        Some(ErrorKind::ConnectionReset) => "Connection reset by peer".into(),
        Some(ErrorKind::BrokenPipe) => "Connection closed unexpectedly".into(),
        _ => format!("Transport error: {error}"),
    }
}

/// Format a timeout message
pub fn format_timeout_error(timeout_secs: u64) -> String {
    format!("Request timed out after {}s", timeout_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_connection_error() {
        assert!(format_connection_error("connection refused").contains("refused"));
        assert!(format_connection_error("TLS handshake failed").contains("TLS"));
        assert!(format_connection_error("timeout").contains("timed out"));
    }

    #[test]
    fn a_connection_error_is_worded_by_its_text() {
        let error = TalosError::Connection("talosctl failed: dial tcp: connection refused".into());
        assert_eq!(
            format_talos_error(&error),
            "Connection refused - is the node reachable?"
        );
        let other = TalosError::Connection("something else".into());
        assert_eq!(
            format_talos_error(&other),
            "Connection failed: something else"
        );
    }

    #[test]
    fn a_transport_error_is_worded_by_the_io_kind_in_its_chain() {
        use std::io::{Error, ErrorKind};
        // As hyper wraps what the OS reports.
        #[derive(Debug)]
        struct Connect(Error);
        impl std::fmt::Display for Connect {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "tcp connect error")
            }
        }
        impl std::error::Error for Connect {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        for (kind, words) in [
            (
                ErrorKind::ConnectionRefused,
                "Connection refused - is the node reachable?",
            ),
            (ErrorKind::TimedOut, "Connection timed out"),
            (ErrorKind::ConnectionReset, "Connection reset by peer"),
            (ErrorKind::BrokenPipe, "Connection closed unexpectedly"),
        ] {
            // The text says something else: the kind decides.
            let chained = Connect(Error::new(kind, "refused timed out reset"));
            assert_eq!(format_transport_error(&chained), words, "{kind:?}");
            assert_eq!(format_transport_error(&Error::from(kind)), words);
        }
        let other = Connect(Error::other("refused"));
        assert_eq!(
            format_transport_error(&other),
            "Transport error: tcp connect error"
        );
    }

    #[tokio::test]
    async fn a_refused_dial_is_worded_as_refused() {
        // A port nothing listens on: bind one, then let it go. Another
        // process could take it before the dial; the test would then fail.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = tonic::transport::Channel::from_shared(format!("http://127.0.0.1:{port}"))
            .unwrap()
            .connect()
            .await
            .unwrap_err();
        assert_eq!(
            format_talos_error(&TalosError::Transport(error)),
            "Connection refused - is the node reachable?"
        );
    }

    #[test]
    fn test_format_timeout_error() {
        assert_eq!(format_timeout_error(10), "Request timed out after 10s");
    }

    #[test]
    fn test_format_grpc_error_by_code() {
        use tonic::{Code, Status};
        for (code, words) in [
            (Code::Unavailable, "Service unavailable - node may be down"),
            (
                Code::PermissionDenied,
                "Permission denied - check RBAC/credentials",
            ),
            (
                Code::Unauthenticated,
                "Authentication failed - check talosconfig",
            ),
            (Code::DeadlineExceeded, "Request timed out"),
            (Code::NotFound, "Resource not found"),
            (Code::AlreadyExists, "Resource already exists"),
        ] {
            // The message says something else: the code decides, and the
            // server's message follows the words.
            assert_eq!(
                format_grpc_error(&Status::new(code, "boom")),
                format!("{words}: boom")
            );
            assert_eq!(format_grpc_error(&Status::new(code, "")), words);
        }
        assert_eq!(
            format_grpc_error(&Status::new(Code::Internal, "permission denied")),
            "gRPC error: permission denied"
        );
        assert_eq!(
            format_grpc_error(&Status::permission_denied(
                "validate configuration on cp-1: proxy denied"
            )),
            "Permission denied - check RBAC/credentials: validate configuration on cp-1: proxy denied"
        );
    }
}
