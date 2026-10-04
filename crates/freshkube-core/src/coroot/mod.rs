//! Read-only Coroot access. Protocols and decoding belong to `coroot-rs`;
//! this boundary owns explicit selection, limits and safe failure categories.
mod connection;
mod limits;
mod subject;
#[cfg(test)]
mod tests;

pub use connection::{Association, Provider, ProviderId, Source};
pub use coroot_rs::{
    AppHealth, AppId, Application, Chart, ClientLink, Credentials, Dependency, Issue,
    LogPatternSummary, MapEdge, MapNode, ProjectInfo, Report, SeriesSummary, ServiceMap, Signal,
    Status, TimeRange,
};
pub use subject::ObjectSubject;

/// A source failure, never an application's health. Messages cannot include
/// credentials, URLs, server response bodies or library error strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("Authentication required or expired. Replace the Coroot credentials.")]
    Authentication,
    #[error("Coroot refused this read. Check this user's project permissions.")]
    Refused,
    #[error("This evidence is not supported by the server or authentication method.")]
    Unsupported,
    #[error("The selected project or application is no longer available.")]
    Missing,
    #[error("Coroot could not be reached. Check the address and try again.")]
    Unreachable,
    #[error("Coroot returned an unrecognized response.")]
    InvalidResponse,
    #[error("Coroot could not complete this read. Try again.")]
    Failed,
    #[error("The Coroot read exceeded the size limit. Narrow the project or time range.")]
    Limit,
    #[error("Enter an HTTP(S) base URL without credentials, query or fragment.")]
    InvalidUrl,
    #[error("Select a project and a time range between one minute and seven days.")]
    InvalidSelection,
    #[error("The Coroot read timed out. Try again.")]
    Timeout,
}

impl From<coroot_rs::Error> for ReadError {
    fn from(error: coroot_rs::Error) -> Self {
        use coroot_rs::ErrorKind;
        match error.kind() {
            ErrorKind::Auth => Self::Authentication,
            ErrorKind::Forbidden => Self::Refused,
            ErrorKind::Unsupported => Self::Unsupported,
            ErrorKind::NotFound => Self::Missing,
            ErrorKind::Network => Self::Unreachable,
            ErrorKind::Decode => Self::InvalidResponse,
            ErrorKind::InvalidInput => Self::InvalidSelection,
            ErrorKind::ResponseTooLarge => Self::Limit,
            _ => Self::Failed,
        }
    }
}

/// Availability is established by a read, never by the presence of an API key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Unchecked,
    Available,
    Unavailable(ReadError),
}

impl Capability {
    pub fn from_result<T>(result: &Result<T, ReadError>) -> Self {
        match result {
            Ok(_) => Self::Available,
            Err(error) => Self::Unavailable(*error),
        }
    }
}
