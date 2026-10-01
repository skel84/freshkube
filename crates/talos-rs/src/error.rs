//! Error types for talos-rs

use thiserror::Error;

/// Errors that can occur when interacting with Talos API
#[derive(Error, Debug)]
pub enum TalosError {
    /// Configuration file not found
    #[error("Config file not found: {0}")]
    ConfigNotFound(String),

    /// Failed to parse configuration
    #[error("Failed to parse config: {0}")]
    ConfigParse(#[from] serde_yaml::Error),

    /// Invalid configuration
    #[error("Invalid config: {0}")]
    ConfigInvalid(String),

    /// Context not found in config
    #[error("Context not found: {0}")]
    ContextNotFound(String),

    /// No endpoints configured for context
    #[error("No endpoints configured for context: {0}")]
    NoEndpoints(String),

    /// Base64 decoding error
    #[error("Base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// TLS error
    #[error("TLS error: {0}")]
    Tls(String),

    /// gRPC transport error
    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// gRPC status error
    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    /// Connection failed
    #[error("Connection failed: {0}")]
    Connection(String),

    /// No home directory found
    #[error("Could not determine home directory")]
    NoHomeDirectory,
}

impl TalosError {
    /// Whether the failure means the underlying connection is unusable, so a
    /// cached client should be dropped and rebuilt rather than retried.
    ///
    /// Application-level statuses (permission denied, not found, ...) are not
    /// transport failures.
    pub fn is_transport_failure(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Connection(_) | Self::Tls(_) | Self::Io(_) => true,
            Self::Grpc(status) => matches!(
                status.code(),
                tonic::Code::Unavailable
                    | tonic::Code::DeadlineExceeded
                    | tonic::Code::Cancelled
                    | tonic::Code::Aborted
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn classifies_transport_failures() {
        for code in [
            tonic::Code::Unavailable,
            tonic::Code::DeadlineExceeded,
            tonic::Code::Cancelled,
        ] {
            assert!(TalosError::Grpc(tonic::Status::new(code, "x")).is_transport_failure());
        }
        assert!(TalosError::Connection("refused".into()).is_transport_failure());
        assert!(TalosError::Tls("handshake".into()).is_transport_failure());
        for code in [tonic::Code::PermissionDenied, tonic::Code::NotFound] {
            assert!(!TalosError::Grpc(tonic::Status::new(code, "x")).is_transport_failure());
        }
        assert!(!TalosError::ContextNotFound("c".into()).is_transport_failure());
    }
}
