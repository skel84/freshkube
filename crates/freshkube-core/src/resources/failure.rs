use std::fmt;

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
}
