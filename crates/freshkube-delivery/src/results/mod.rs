//! Tekton Results through an address or a caller-supplied loopback route.
//! No cluster client, discovery or forward is started here. The caller owns
//! the read's job, identity and Snapshot; replacing the client gives it a new
//! identity even if only the key or CA changed.
//!
//! The build read supplements a successful live listing. Archived log
//! content has a bounded on-demand API; its dock consumer is a later slice.

mod builds;
mod logs;
mod records;
#[cfg(test)]
mod tests;
mod transport;

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use freshkube_core::secrets::Secrets;
use freshkube_core::setup::{Forward, Results};
use freshkube_core::snapshot::Snapshot;
use tokio::time::Instant;

use crate::digest::is_full_sha;
use crate::read::{MAX_BODY_BYTES, MAX_PAGES, PAGE_LIMIT};
use crate::tekton::{MOST_TASK_READS, PipelineRun, SHA_LABEL, TaskRun};

pub(crate) use builds::{Archive, History, supplement, supplement_digests};
pub(crate) use logs::Log;

pub(crate) const MOST_RESULT_RUNS: usize = MOST_TASK_READS;
const MOST_RESULT_TASKS: usize = PAGE_LIMIT as usize * MAX_PAGES as usize;
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

/// Supplied by the forward's owner; this client never starts or replaces one.
#[derive(Clone, Copy)]
pub(crate) enum Route {
    Address,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "forward slice 2 supplies the route")
    )]
    Forward {
        local_port: NonZeroU16,
    },
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "forward slice 2 supplies the route")
    )]
    Ended(&'static str),
}

/// Deliberately has no Debug: neither an address nor an authorization header
/// should appear in logs. There is no constructor accepting cluster credentials.
pub(crate) struct Client {
    http: reqwest::Client,
    base: url::Url,
    token: Option<http::HeaderValue>,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "slice 9 consumes Results identity and freshness")
    )]
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
    /// Records returned by the server, including any beyond the parsed cap.
    /// Exact only when the final page was read; never estimated.
    pub total: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cap {
    Runs,
    Tasks,
    Logs,
    Pages,
    Bytes,
}

impl std::fmt::Display for Cap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runs => write!(f, "the {MOST_RESULT_RUNS}-run cap"),
            Self::Tasks => write!(f, "the {MOST_RESULT_TASKS}-task cap"),
            Self::Logs => write!(f, "the {MOST_RESULT_TASKS}-log cap"),
            Self::Pages => write!(f, "the {MAX_PAGES}-page cap"),
            Self::Bytes => write!(f, "the {} MiB response cap", MAX_BODY_BYTES / (1024 * 1024)),
        }
    }
}

/// Every state in the design, plus an unreadable response/configuration.
/// Stale is projected from the owner's Snapshot, never cached by the client.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "slice 9 consumes Results identity and freshness")
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Off,
    Skipped,
    Loading,
    NotForwarded,
    ForwardEnded,
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
    Skipped(Skip),
    NotForwarded,
    ForwardEnded(&'static str),
    NotInstalled,
    LogsNotServed,
    LogMissing,
    Unreachable(Cause),
    Refused { status: u16, token_set: bool },
    Http(u16),
    Unreadable(&'static str),
}

/// Results was not asked because the live read cannot establish absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Skip {
    LiveFailed,
    LiveCapped,
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LiveFailed => "the cluster's PipelineRuns were not read",
            Self::LiveCapped => {
                "the live build read stopped at the cap, so absence could not be established"
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    TimedOut,
    Certificate,
    Connect,
    Transport,
    Gateway(u16),
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => f.write_str("the request timed out"),
            Self::Certificate => f.write_str("certificate not trusted"),
            Self::Connect => f.write_str("the connection failed"),
            Self::Transport => f.write_str("the response could not be received"),
            Self::Gateway(status) => write!(f, "the gateway returned HTTP {status}"),
        }
    }
}

impl Error {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "slice 9 consumes Results identity and freshness")
    )]
    pub(crate) fn state(&self) -> State {
        match self {
            Self::Off => State::Off,
            Self::Skipped(_) => State::Skipped,
            Self::NotForwarded => State::NotForwarded,
            Self::ForwardEnded(_) => State::ForwardEnded,
            Self::NotInstalled | Self::LogsNotServed => State::NotInstalled,
            Self::LogMissing => State::Empty,
            Self::Unreachable(_) => State::Unreachable,
            Self::Refused { .. } => State::Refused,
            Self::Http(_) | Self::Unreadable(_) => State::Unreadable,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("Tekton Results isn't set up"),
            Self::Skipped(why) => write!(f, "Tekton Results wasn't read: {why}"),
            Self::NotForwarded => f.write_str("Tekton Results isn't forwarded yet"),
            Self::ForwardEnded(why) => write!(f, "the Tekton Results forward ended: {why}"),
            Self::NotInstalled => {
                f.write_str("the Tekton Results API isn't installed or served here")
            }
            Self::LogsNotServed => f.write_str("this Tekton Results doesn't serve logs"),
            Self::LogMissing => f.write_str("Tekton Results kept no log for this task"),
            Self::Unreachable(cause) => write!(f, "Tekton Results didn't answer: {cause}"),
            Self::Refused { status, token_set } => {
                write!(f, "Tekton Results refused the request ({status})")?;
                if !token_set {
                    f.write_str("; no token is set for this address")?;
                }
                Ok(())
            }
            Self::Http(status) => write!(f, "Tekton Results returned HTTP {status}"),
            Self::Unreadable(why) => write!(f, "Tekton Results could not be read: {why}"),
        }
    }
}

/// Use an identity containing the client revision and the entire query
/// (namespace, SHA or result), not just the address. Snapshot owns freshness
/// and request generation, exactly as it does for ordinary cluster reads.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "slice 9 consumes Results identity and freshness")
)]
pub(crate) fn state<T, I: Clone + Eq>(snapshot: &Snapshot<Listing<T>, I, Error>) -> State {
    if snapshot.is_loading() {
        State::Loading
    } else if snapshot.is_stale() {
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

/// Stable across local ports and restarts; only the configured triple matters.
pub(crate) fn forward_token_account(forward: &Forward) -> String {
    format!(
        "Tekton Results forward://{}/{}:{}",
        forward.namespace, forward.service, forward.port
    )
}

impl Client {
    /// Opens on an I/O worker, reading only the explicitly selected key store
    /// and CA file. Missing keys allow an anonymous request; store failures do
    /// not silently downgrade authentication. No I/O for Off or NotForwarded.
    pub(crate) async fn open(
        settings: Option<&Results>,
        secrets: Option<Secrets>,
    ) -> Result<Self, Error> {
        Self::open_routed(settings, secrets, Route::Address).await
    }

    /// The caller must match a running forward to the configured triple and
    /// replace the client when that forward's epoch changes. An ended route
    /// reads neither the key store nor the CA file and never falls back.
    pub(crate) async fn open_routed(
        settings: Option<&Results>,
        secrets: Option<Secrets>,
        route: Route,
    ) -> Result<Self, Error> {
        let settings = settings.ok_or(Error::Off)?;
        let (base, account, pin) = match (&settings.address, &settings.forward, route) {
            (Some(address), None, Route::Address) => {
                (transport::base_url(address)?, token_account(address)?, None)
            }
            (None, Some(_), Route::Address) => return Err(Error::NotForwarded),
            (None, Some(_), Route::Ended(why)) => return Err(Error::ForwardEnded(why)),
            (None, Some(forward), Route::Forward { local_port }) => {
                transport::segment(&forward.namespace)?;
                transport::segment(&forward.service)?;
                let host = format!("{}.{}.svc", forward.service, forward.namespace);
                let base = transport::base_url(&format!("https://{host}/"))?;
                let pin = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port.get()));
                (base, forward_token_account(forward), Some((host, pin)))
            }
            (Some(_), None, _) => {
                return Err(Error::Unreadable("a forward route needs forward settings"));
            }
            _ => return Err(Error::Unreadable("set exactly one address or forward")),
        };
        let use_token = settings.token.is_some();
        let ca_file = settings.ca_file.clone();
        let (token, ca) = tokio::task::spawn_blocking(move || {
            let token = if use_token {
                secrets
                    .ok_or(Error::Unreadable(
                        "the key store isn't available to this read",
                    ))?
                    .read(&account)
                    .map_err(|_| Error::Unreadable("the key store is unavailable"))?
            } else {
                None
            };
            let ca = ca_file.map(transport::read_ca).transpose()?;
            Ok::<_, Error>((token, ca))
        })
        .await
        .map_err(|_| Error::Unreadable("the Results settings could not be read"))??;
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
        if let Some((host, address)) = &pin {
            // No URL port: an explicit one would override resolve's local port.
            builder = builder.resolve(host, *address).no_proxy();
        }
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

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "slice 9 consumes Results identity and freshness")
    )]
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
            &format!("{parent}/records"),
            &filter,
            MOST_RESULT_RUNS,
            Cap::Runs,
            until,
            |record| records::pipeline_run(record, namespace, sha),
        )
        .await
    }

    /// The first [`MOST_RESULT_RUNS`] PipelineRun records the archive returns
    /// for the namespace, whatever commit they carry; no order is asked for.
    /// A result can't be selected on, so a digest is looked for in these in
    /// memory.
    pub(crate) async fn first_pipeline_runs(
        &self,
        namespace: &str,
        until: Instant,
    ) -> Result<Listing<PipelineRun>, Error> {
        transport::segment(namespace)?;
        let filter = "data_type == 'tekton.dev/v1.PipelineRun' || data_type == 'tekton.dev/v1beta1.PipelineRun'";
        self.list(
            &format!("{namespace}/results/-/records"),
            filter,
            MOST_RESULT_RUNS,
            Cap::Runs,
            until,
            |record| records::pipeline_run_of(record, namespace),
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
            &format!("{result}/records"),
            filter,
            MOST_RESULT_TASKS,
            Cap::Tasks,
            until,
            |record| records::task_run(record, result),
        )
        .await
    }
}
