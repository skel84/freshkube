use std::fmt;

use talos_rs::TalosError;

/// The reason kube gives an error whose body it couldn't read as a Status.
const UNPARSED: &str = "Failed to parse error data";

/// What kind of failure a read hit, so a frontend can say something more
/// useful than a raw error string and never mistake a refusal for an empty
/// result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The kubeconfig could not be read or did not describe the context.
    Config,
    /// The API server could not be reached.
    Unreachable,
    /// The request took too long.
    Timeout,
    /// Credentials were rejected (401) or could not be obtained.
    Unauthorized,
    /// The identity may not perform this read (403).
    Forbidden,
    /// The resource or API does not exist on this server (404).
    NotFound,
    /// Anything else the server or client reported.
    Other,
}

impl FailureKind {
    /// Retrying won't help until something outside the app changes: RBAC, or
    /// the API being installed.
    pub fn is_permanent(self) -> bool {
        matches!(self, FailureKind::Forbidden | FailureKind::NotFound)
    }
}

/// A failed Kubernetes read, already classified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Classifies a Talos error by its gRPC code, never its message. Errors
    /// with no code (transport, connection, TLS) are classified by the
    /// variant alone.
    pub fn from_talos(error: &TalosError) -> Self {
        let kind = match error {
            TalosError::Grpc(status) => match status.code() {
                tonic::Code::PermissionDenied => FailureKind::Forbidden,
                tonic::Code::Unauthenticated => FailureKind::Unauthorized,
                tonic::Code::Unavailable => FailureKind::Unreachable,
                tonic::Code::DeadlineExceeded => FailureKind::Timeout,
                tonic::Code::NotFound => FailureKind::NotFound,
                _ => FailureKind::Other,
            },
            TalosError::Connection(_) | TalosError::Transport(_) => FailureKind::Unreachable,
            TalosError::ConfigNotFound(_)
            | TalosError::ConfigParse(_)
            | TalosError::ConfigInvalid(_)
            | TalosError::ContextNotFound(_)
            | TalosError::NoEndpoints(_) => FailureKind::Config,
            _ => FailureKind::Other,
        };
        Self::new(kind, crate::errors::format_talos_error(error))
    }

    pub fn from_kube(error: kube::Error) -> Self {
        match error {
            kube::Error::Api(status) => {
                let kind = match status.code {
                    401 => FailureKind::Unauthorized,
                    403 => FailureKind::Forbidden,
                    404 => FailureKind::NotFound,
                    408 | 504 => FailureKind::Timeout,
                    _ => FailureKind::Other,
                };
                // A body that isn't a Status (a proxy's or the mux's "404
                // page not found") comes quoted, as Rust prints a string.
                let message = if status.reason == UNPARSED {
                    serde_json::from_str::<String>(&status.message)
                        .unwrap_or(status.message)
                        .trim()
                        .to_owned()
                } else {
                    status.message
                };
                let message = if message.is_empty() {
                    format!("API error {}", status.code)
                } else {
                    message
                };
                Self::new(kind, message)
            }
            kube::Error::Auth(error) => Self::new(FailureKind::Unauthorized, error.to_string()),
            kube::Error::HyperError(error) => {
                Self::new(FailureKind::Unreachable, error.to_string())
            }
            kube::Error::Service(error) => {
                let message = with_causes(error.as_ref());
                let kind = if message.to_lowercase().contains("timed out") {
                    FailureKind::Timeout
                } else {
                    FailureKind::Unreachable
                };
                Self::new(kind, message)
            }
            other => Self::new(FailureKind::Other, other.to_string()),
        }
    }

    pub fn timeout(what: &str) -> Self {
        Self::new(FailureKind::Timeout, format!("{what} timed out"))
    }
}

impl From<kube::Error> for Failure {
    fn from(error: kube::Error) -> Self {
        Self::from_kube(error)
    }
}

impl From<&TalosError> for Failure {
    fn from(error: &TalosError) -> Self {
        Self::from_talos(error)
    }
}

impl From<TalosError> for Failure {
    fn from(error: TalosError) -> Self {
        Self::from_talos(&error)
    }
}

/// An error and its causes, as "client error (Connect): unsuccessful tunnel
/// (HTTP/1.1 407 Pro)": hyper's own message names only the stage that failed.
fn with_causes(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_message = cause.to_string();
        if !message.contains(&cause_message) {
            message.push_str(": ");
            message.push_str(&cause_message);
        }
        source = cause.source();
    }
    message
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            FailureKind::Config => "Configuration",
            FailureKind::Unreachable => "Unreachable",
            FailureKind::Timeout => "Timed out",
            FailureKind::Unauthorized => "Unauthorized",
            FailureKind::Forbidden => "Forbidden",
            FailureKind::NotFound => "Not found",
            FailureKind::Other => "Error",
        };
        write!(f, "{kind} · {}", self.message)
    }
}

impl std::error::Error for Failure {}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::core::ErrorResponse;

    fn api(code: u16, message: &str) -> kube::Error {
        kube::Error::Api(ErrorResponse {
            status: "Failure".into(),
            message: message.into(),
            reason: String::new(),
            code,
        })
    }

    #[test]
    fn api_status_codes_are_classified() {
        let forbidden = Failure::from_kube(api(403, "pods is forbidden"));
        assert_eq!(forbidden.kind, FailureKind::Forbidden);
        assert_eq!(forbidden.to_string(), "Forbidden · pods is forbidden");
        assert!(forbidden.kind.is_permanent());
        assert_eq!(
            Failure::from_kube(api(404, "")).message,
            "API error 404",
            "an empty message names the code"
        );
        assert_eq!(
            Failure::from_kube(api(401, "x")).kind,
            FailureKind::Unauthorized
        );
        assert_eq!(Failure::from_kube(api(504, "x")).kind, FailureKind::Timeout);
        assert_eq!(Failure::from_kube(api(500, "x")).kind, FailureKind::Other);
        assert!(!FailureKind::Unreachable.is_permanent());
    }

    #[test]
    fn a_body_that_is_not_a_status_reads_as_its_text() {
        let unparsed = |message: &str| {
            Failure::from_kube(kube::Error::Api(ErrorResponse {
                status: "404 Not Found".into(),
                // As kube-client 0.98 builds it from the body.
                message: format!("{message:?}"),
                reason: UNPARSED.into(),
                code: 404,
            }))
        };
        let missing = unparsed("404 page not found\n");
        assert_eq!(missing.kind, FailureKind::NotFound);
        assert_eq!(missing.to_string(), "Not found · 404 page not found");
        assert_eq!(unparsed("\n").message, "API error 404");
        // A Status message is kept as the server wrote it.
        assert_eq!(
            Failure::from_kube(api(404, "\"x\" not found")).message,
            "\"x\" not found"
        );
    }

    /// A fake API server answering every request with `status` and `body`,
    /// or, with `None`, with a connection that is cut.
    fn server(reply: Option<(u16, &'static str)>) -> kube::Client {
        use kube::client::Body;
        let service = tower::service_fn(move |_: http::Request<Body>| async move {
            match reply {
                Some((status, body)) => Ok(http::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(Body::from(body.as_bytes().to_vec()))
                    .unwrap()),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "connection reset by peer",
                )),
            }
        });
        kube::Client::new(service, "default")
    }

    async fn read(client: kube::Client) -> Failure {
        let pods = kube::Api::<k8s_openapi::api::core::v1::Pod>::default_namespaced(client);
        let error = pods.list(&Default::default()).await.unwrap_err();
        Failure::from(error)
    }

    fn status_body(code: u16, message: &str) -> &'static str {
        Box::leak(
            serde_json::json!({
                "kind": "Status", "apiVersion": "v1", "status": "Failure",
                "message": message, "code": code,
            })
            .to_string()
            .into_boxed_str(),
        )
    }

    #[tokio::test]
    async fn a_fake_servers_refusals_and_cut_connection_are_classified() {
        let unauthorized = read(server(Some((401, status_body(401, "Unauthorized"))))).await;
        assert_eq!(unauthorized.kind, FailureKind::Unauthorized);

        let forbidden = read(server(Some((403, status_body(403, "pods is forbidden"))))).await;
        assert_eq!(forbidden.kind, FailureKind::Forbidden);
        assert_eq!(forbidden.message, "pods is forbidden");

        let missing = read(server(Some((404, status_body(404, "not found"))))).await;
        assert_eq!(missing.kind, FailureKind::NotFound);

        let cut = read(server(None)).await;
        assert_eq!(cut.kind, FailureKind::Unreachable);
        assert!(cut.message.contains("connection reset"), "{}", cut.message);
    }

    #[test]
    fn talos_grpc_codes_map_to_kinds_never_by_message() {
        use tonic::Code;
        for (code, kind) in [
            (Code::PermissionDenied, FailureKind::Forbidden),
            (Code::Unauthenticated, FailureKind::Unauthorized),
            (Code::Unavailable, FailureKind::Unreachable),
            (Code::DeadlineExceeded, FailureKind::Timeout),
            (Code::NotFound, FailureKind::NotFound),
            (Code::Internal, FailureKind::Other),
        ] {
            // A message that names another kind must not matter.
            let error = TalosError::Grpc(tonic::Status::new(code, "something unrelated"));
            assert_eq!(Failure::from(&error).kind, kind, "{code:?}");
            assert_eq!(Failure::from(error).kind, kind, "{code:?}");
        }
        let misleading = TalosError::Grpc(tonic::Status::new(Code::Internal, "permission denied"));
        assert_eq!(Failure::from(&misleading).kind, FailureKind::Other);
        let refused = TalosError::Connection("connection refused".into());
        assert_eq!(Failure::from(&refused).kind, FailureKind::Unreachable);
    }
}
