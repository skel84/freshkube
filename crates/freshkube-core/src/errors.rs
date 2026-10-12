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
/// assert!(message.contains("refused"));
/// ```
pub fn format_talos_error(error: &TalosError) -> String {
    match error {
        TalosError::Connection(msg) => format_connection_error(msg),
        TalosError::Grpc(status) => format_grpc_error(status),
        TalosError::Transport(e) => format_transport_error(&e.to_string()),
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

/// Format a connection error message
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

/// Format a transport error message
fn format_transport_error(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("refused") {
        "Connection refused - is the node reachable?".to_string()
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "Connection timed out".to_string()
    } else if lower.contains("reset") {
        "Connection reset by peer".to_string()
    } else if lower.contains("broken pipe") {
        "Connection closed unexpectedly".to_string()
    } else {
        format!("Transport error: {}", msg)
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
