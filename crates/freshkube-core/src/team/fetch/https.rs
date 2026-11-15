//! A team file at an HTTPS address. Redirects are followed by hand, only to
//! the same host; the optional token goes in one header and nowhere else.

use std::collections::BTreeMap;

use reqwest::header::{AUTHORIZATION, HeaderValue, LOCATION};
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use super::Limits;
use crate::secrets::Secrets;
use crate::team::file::Version;
use crate::team::membership::FetchFailed;
use crate::team::source::{HttpsSource, check_redirect};

/// At most this many redirects are followed (each to the same host).
const MOST_REDIRECTS: usize = 3;

/// A token read from the key store. It is wiped when dropped, and `Debug`
/// never shows it.
struct Token(Zeroizing<String>);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

pub struct HttpsFetch {
    client: reqwest::Client,
    secrets: Option<Secrets>,
    limits: Limits,
}

impl HttpsFetch {
    pub fn new(secrets: Option<Secrets>, limits: Limits) -> Self {
        let client = Self::builder(limits)
            .build()
            .expect("a client with a fixed configuration");
        Self {
            client,
            secrets,
            limits,
        }
    }

    fn builder(limits: Limits) -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            // Redirects are checked by hand, so none leaves the host.
            .redirect(Policy::none())
            .https_only(true)
            .connect_timeout(limits.deadline)
            .user_agent("freshkube")
    }

    /// A client that trusts one certificate alone and no proxy, for a test's
    /// loopback server.
    #[cfg(test)]
    pub(crate) fn trusting(pem: &[u8], secrets: Option<Secrets>, limits: Limits) -> Self {
        let root = reqwest::Certificate::from_pem(pem).expect("a certificate");
        let client = Self::builder(limits)
            .add_root_certificate(root)
            .no_proxy()
            .build()
            .expect("a client");
        Self {
            client,
            secrets,
            limits,
        }
    }

    fn failed(what: &str) -> FetchFailed {
        FetchFailed(what.to_string())
    }

    async fn token(&self, source: &HttpsSource) -> Result<Option<Token>, FetchFailed> {
        let Some(secrets) = self.secrets.clone() else {
            return Ok(None);
        };
        let account = source.token_account();
        // A key-store call can block, so it runs off the async worker.
        let read = tokio::task::spawn_blocking(move || secrets.read(&account))
            .await
            .map_err(|_| Self::failed("the key store didn’t answer"))?;
        match read {
            Ok(token) => Ok(token
                .filter(|t| !t.is_empty())
                .map(|t| Token(Zeroizing::new(t)))),
            Err(_) => Err(Self::failed("the key store couldn’t be read")),
        }
    }

    pub async fn fetch(&self, source: &HttpsSource) -> Result<Version, FetchFailed> {
        let token = self.token(source).await?;
        match tokio::time::timeout(self.limits.deadline, self.get(source, token)).await {
            Ok(result) => result,
            Err(_) => Err(Self::failed("the host took too long")),
        }
    }

    async fn get(
        &self,
        source: &HttpsSource,
        token: Option<Token>,
    ) -> Result<Version, FetchFailed> {
        let mut url = source.url().clone();
        for _ in 0..=MOST_REDIRECTS {
            let mut request = self.client.get(url.clone());
            if let Some(token) = &token {
                let mut value = HeaderValue::from_str(&format!("Bearer {}", token.0.as_str()))
                    .map_err(|_| Self::failed("the token can’t be sent as a header"))?;
                value.set_sensitive(true);
                request = request.header(AUTHORIZATION, value);
            }
            let mut response = request.send().await.map_err(|error| {
                // `reqwest` errors carry the address; only words are kept.
                Self::failed(if error.is_timeout() {
                    "the host took too long"
                } else if error.is_connect() {
                    "the host didn’t answer"
                } else {
                    "the request failed"
                })
            })?;
            let status = response.status();
            if status.is_redirection() {
                let to = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| url.join(v).ok())
                    .ok_or_else(|| {
                        Self::failed("the host redirected to an address that isn’t valid")
                    })?;
                check_redirect(source.url(), &to).map_err(|why| FetchFailed(why.to_string()))?;
                url = strip(to);
                continue;
            }
            if !status.is_success() {
                return Err(FetchFailed(format!(
                    "the host answered {}",
                    status.as_u16()
                )));
            }
            let most = self.limits.file_bytes;
            if response.content_length().is_some_and(|n| n > most) {
                return Err(Self::failed("the team file is larger than 256 KiB"));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| Self::failed("the answer was cut off"))?
            {
                if (body.len() + chunk.len()) as u64 > most {
                    return Err(Self::failed("the team file is larger than 256 KiB"));
                }
                body.extend_from_slice(&chunk);
            }
            let id: String = Sha256::digest(&body)
                .iter()
                .take(6)
                .map(|b| format!("{b:02x}"))
                .collect();
            return Ok(Version::parse(id, &body, BTreeMap::new())?);
        }
        Err(Self::failed("the host redirected too many times"))
    }
}

/// An address without a fragment, as a request is sent to it.
fn strip(mut url: Url) -> Url {
    url.set_fragment(None);
    url
}
