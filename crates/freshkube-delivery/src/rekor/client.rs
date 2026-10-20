//! The one request the transparency log gets: a GET over https. It is its own
//! small client and shares nothing with the Results client, so nothing that
//! authenticates to Results can reach a log: no header is ever added, no CA
//! from the Results setting is trusted, and a redirect is an answer, not a
//! hop to follow.

use std::time::Duration;

use freshkube_core::resources::{Failure, FailureKind};
use url::Url;

use super::{Fetch, Fetched, MOST_ENTRY_BYTES};

/// The most a single request may take, connecting and reading.
const DEADLINE: Duration = Duration::from_secs(5);

pub(crate) struct Client {
    http: reqwest::Client,
}

impl Client {
    /// A client that trusts reqwest's rustls roots and nothing else.
    pub(crate) fn new() -> Result<Self, Failure> {
        Self::from(reqwest::Client::builder())
    }

    /// A client that also trusts `pem`, for a fake log in a test.
    #[cfg(test)]
    pub(super) fn trusting(pem: &[u8]) -> Result<Self, Failure> {
        let root = reqwest::Certificate::from_pem(pem).map_err(|_| unusable())?;
        Self::from(reqwest::Client::builder().add_root_certificate(root))
    }

    fn from(builder: reqwest::ClientBuilder) -> Result<Self, Failure> {
        let http = builder
            .use_rustls_tls()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(DEADLINE)
            .timeout(DEADLINE)
            .build()
            .map_err(|_| unusable())?;
        Ok(Self { http })
    }
}

fn unusable() -> Failure {
    Failure::new(
        FailureKind::Config,
        "the transparency log client could not be set up",
    )
}

/// An address the log may be asked at: https, a host, and no credentials
/// that a client would turn into an Authorization header.
fn allowed(address: &str) -> Result<Url, Failure> {
    let url = Url::parse(address).map_err(|_| refusal("the address is not a URL"))?;
    if url.scheme() != "https" || url.host().is_none() {
        return Err(refusal("the address is not an https address"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refusal("the address carries credentials"));
    }
    Ok(url)
}

fn refusal(why: &str) -> Failure {
    Failure::new(FailureKind::Config, why)
}

fn failed(error: reqwest::Error) -> Failure {
    let kind = if error.is_timeout() {
        FailureKind::Timeout
    } else {
        FailureKind::Unreachable
    };
    // The error's text can repeat the URL; say only what kind it was.
    Failure::new(
        kind,
        if error.is_timeout() {
            "timed out"
        } else if error.is_connect() {
            "could not connect"
        } else {
            "the request failed"
        },
    )
}

impl Fetch for Client {
    async fn get(&self, url: &str) -> Result<Fetched, Failure> {
        let url = allowed(url)?;
        let mut response = self
            .http
            .get(url)
            .header(http::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(failed)?;
        let status = response.status().as_u16();
        // An error's body is never read.
        if status != 200 {
            return Ok(Fetched {
                status,
                body: Vec::new(),
            });
        }
        let mut body = Vec::new();
        // Count bytes as they arrive; a chunked body has no length to trust.
        while let Some(chunk) = response.chunk().await.map_err(failed)? {
            let room = MOST_ENTRY_BYTES - body.len();
            if chunk.len() >= room {
                // A body at the cap is returned at the cap, which never
                // matches.
                body.extend_from_slice(&chunk[..room]);
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Fetched { status, body })
    }
}
