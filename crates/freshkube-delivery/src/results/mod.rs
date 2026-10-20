//! Tekton Results' address route. No cluster client, discovery or forward is
//! used here. The caller owns the read's job, identity and Snapshot; replacing
//! the client gives it a new identity even if only the key or CA changed.
//!
//! Slice 6 connects this client to the build read. Until then nothing in the
//! application calls it. Logs and the explicit forward belong to later slices.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "the build read uses Results in slice 6")
)]

mod records;
#[cfg(test)]
mod tests;
mod transport;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use freshkube_core::secrets::Secrets;
use freshkube_core::setup::Results;
use freshkube_core::snapshot::Snapshot;
use tokio::time::Instant;

use crate::digest::is_full_sha;
use crate::read::{MAX_PAGES, PAGE_LIMIT};
use crate::tekton::{MOST_TASK_READS, PipelineRun, SHA_LABEL, TaskRun};

pub(crate) const MOST_RESULT_RUNS: usize = MOST_TASK_READS;
const MOST_RESULT_TASKS: usize = PAGE_LIMIT as usize * MAX_PAGES as usize;
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

/// Deliberately has no Debug: neither an address nor an authorization header
/// should appear in logs. There is no constructor accepting cluster credentials.
pub(crate) struct Client {
    http: reqwest::Client,
    base: url::Url,
    token: Option<http::HeaderValue>,
    id: u64,
}

/// One archived object and its Results resource names, kept for a later task
/// or log read. Names are path components, never addresses to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record<T> {
    pub name: String,
    pub result: String,
    pub run: T,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listing<T> {
    pub records: Vec<Record<T>>,
    /// Why records were left unread. Results gives no total count; a cap
    /// never implies that a missing run does not exist.
    pub cap: Option<Cap>,
    /// Exact only when the final page was read; never estimated.
    pub total: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cap {
    Runs,
    Tasks,
    Pages,
    Bytes,
}

impl std::fmt::Display for Cap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runs => write!(f, "the {MOST_RESULT_RUNS}-run cap"),
            Self::Tasks => write!(f, "the {MOST_RESULT_TASKS}-task cap"),
            Self::Pages => write!(f, "the {MAX_PAGES}-page cap"),
            Self::Bytes => f.write_str("the 32 MiB response cap"),
        }
    }
}

/// Every state in the design, plus an unreadable response/configuration.
/// Stale is projected from the owner's Snapshot, never cached by the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Off,
    NotForwarded,
    NotInstalled,
    Unreachable,
    Refused,
    Capped(Cap),
    Stale,
    Empty,
    Read,
    Unreadable,
}

/// Safe to display: no library errors, response bodies, URLs, paths or tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Off,
    NotForwarded,
    NotInstalled,
    Unreachable(Cause),
    Refused { status: u16, token_set: bool },
    Http(u16),
    Capped(Cap),
    Unreadable(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    TimedOut,
    Certificate,
    Connect,
    Transport,
}

impl Error {
    pub(crate) fn state(&self) -> State {
        match self {
            Self::Off => State::Off,
            Self::NotForwarded => State::NotForwarded,
            Self::NotInstalled => State::NotInstalled,
            Self::Unreachable(_) => State::Unreachable,
            Self::Refused { .. } => State::Refused,
            Self::Http(_) | Self::Unreadable(_) => State::Unreadable,
            Self::Capped(cap) => State::Capped(*cap),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("Tekton Results isn't set up"),
            Self::NotForwarded => f.write_str("Tekton Results isn't forwarded yet"),
            Self::NotInstalled => {
                f.write_str("the Tekton Results API isn't installed or served here")
            }
            Self::Unreachable(cause) => write!(
                f,
                "Tekton Results didn't answer: {}",
                match cause {
                    Cause::TimedOut => "the request timed out",
                    Cause::Certificate => "certificate not trusted",
                    Cause::Connect => "the connection failed",
                    Cause::Transport => "the response could not be received",
                }
            ),
            Self::Refused { status, token_set } => {
                write!(f, "Tekton Results refused the request ({status})")?;
                if !token_set {
                    f.write_str("; no token is set for this address")?;
                }
                Ok(())
            }
            Self::Http(status) => write!(f, "Tekton Results returned HTTP {status}"),
            Self::Capped(cap) => write!(
                f,
                "Tekton Results stopped at {cap}; more records weren't read"
            ),
            Self::Unreadable(why) => write!(f, "Tekton Results could not be read: {why}"),
        }
    }
}

/// Use an identity containing the client revision and the entire query
/// (namespace, SHA or result), not just the address. Snapshot owns freshness
/// and request generation, exactly as it does for ordinary cluster reads.
pub(crate) fn state<T, I: Clone + Eq>(snapshot: &Snapshot<Listing<T>, I, Error>) -> State {
    if snapshot.is_stale() {
        State::Stale
    } else if let Some(error) = snapshot.error() {
        error.state()
    } else if let Some(listing) = snapshot.data() {
        if let Some(cap) = listing.cap {
            State::Capped(cap)
        } else if listing.records.is_empty() {
            State::Empty
        } else {
            State::Read
        }
    } else {
        State::Off
    }
}

/// The key-store namespace for a Results address, normalized exactly as the
/// client normalizes it. No Prometheus, Coroot or Kubernetes account is read.
pub(crate) fn token_account(address: &str) -> Result<String, Error> {
    transport::base_url(address).map(|base| format!("Tekton Results {base}"))
}

impl Client {
    /// Opens on an I/O worker, reading only the explicitly selected key store
    /// and CA file. Missing keys allow an anonymous request; store failures do
    /// not silently downgrade authentication. No I/O for Off or NotForwarded.
    pub(crate) async fn open(settings: Option<&Results>, secrets: Secrets) -> Result<Self, Error> {
        let settings = settings.ok_or(Error::Off)?;
        let address = match (&settings.address, &settings.forward) {
            (None, Some(_)) => return Err(Error::NotForwarded),
            (Some(address), None) => address,
            _ => return Err(Error::Unreadable("set exactly one address or forward")),
        };
        let base = transport::base_url(address)?;
        let account = token_account(address)?;
        let use_token = settings.token.is_some();
        let ca_file = settings.ca_file.clone();
        let (token, ca) = tokio::task::spawn_blocking(move || {
            let token = if use_token {
                secrets
                    .read(&account)
                    .map_err(|_| Error::Unreadable("the key store is unavailable"))?
            } else {
                None
            };
            let ca = ca_file.map(transport::read_ca).transpose()?;
            Ok::<_, Error>((token, ca))
        })
        .await
        .map_err(|_| Error::Unreadable("the address settings could not be read"))??;
        let token = token
            .map(|token| {
                if token.trim().is_empty() {
                    return Err(Error::Unreadable("the stored token is empty"));
                }
                if token.len() > 8192
                    || !token.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
                    })
                {
                    return Err(Error::Unreadable("the stored token is not a bearer token"));
                }
                let mut header = http::HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| Error::Unreadable("the stored token is not a bearer token"))?;
                header.set_sensitive(true);
                Ok(header)
            })
            .transpose()?;
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(REQUEST_DEADLINE)
            .timeout(REQUEST_DEADLINE);
        // Loopback is an explicit local route, never sent to an environment
        // proxy. HTTPS addresses retain reqwest's normal proxy handling.
        if transport::is_loopback(&base) {
            builder = builder.no_proxy();
        }
        if let Some(ca) = ca {
            let certificates = reqwest::Certificate::from_pem_bundle(&ca)
                .map_err(|_| Error::Unreadable("the CA file is not a PEM certificate bundle"))?;
            if certificates.is_empty() {
                return Err(Error::Unreadable("the CA file has no certificates"));
            }
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        let http = builder
            .build()
            .map_err(|_| Error::Unreadable("the HTTP client could not be configured"))?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            http,
            base,
            token,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        })
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) async fn pipeline_runs(
        &self,
        namespace: Option<&str>,
        sha: &str,
        until: Instant,
    ) -> Result<Listing<PipelineRun>, Error> {
        if !is_full_sha(sha) {
            return Err(Error::Unreadable("the commit is not a full SHA"));
        }
        let namespace = namespace.unwrap_or("-");
        transport::segment(namespace)?;
        let parent = format!("{namespace}/results/-");
        let filter = format!(
            "(data_type == 'tekton.dev/v1.PipelineRun' || data_type == 'tekton.dev/v1beta1.PipelineRun') && data.metadata.labels['{SHA_LABEL}'] == '{sha}'"
        );
        self.list(
            &parent,
            &filter,
            MOST_RESULT_RUNS,
            Cap::Runs,
            until,
            |record| records::pipeline_run(record, namespace, sha),
        )
        .await
    }

    pub(crate) async fn task_runs(
        &self,
        result: &str,
        until: Instant,
    ) -> Result<Listing<TaskRun>, Error> {
        records::result_parts(result)?;
        let filter =
            "data_type == 'tekton.dev/v1.TaskRun' || data_type == 'tekton.dev/v1beta1.TaskRun'";
        self.list(
            result,
            filter,
            MOST_RESULT_TASKS,
            Cap::Tasks,
            until,
            |record| records::task_run(record, result),
        )
        .await
    }
}
