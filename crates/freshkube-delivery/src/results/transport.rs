//! GET-only transport, URL confinement, TLS and bounded paging.
use std::collections::BTreeSet;
use std::error::Error as _;
use std::io::Read;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;
use url::{Host, Url};

use super::{Cap, Cause, Client, Error, Listing, REQUEST_DEADLINE, Record};
use crate::read::{MAX_BODY_BYTES, MAX_PAGES, PAGE_LIMIT};

const MOST_CA_BYTES: u64 = 256 * 1024;
const MOST_PAGE_TOKEN_BYTES: usize = 4096;

pub(super) fn base_url(address: &str) -> Result<Url, Error> {
    let mut url = Url::parse(address.trim()).map_err(|_| Error::Unreadable("invalid address"))?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Unreadable(
            "use an HTTP(S) address without credentials, query or fragment",
        ));
    }
    if url.scheme() == "http" && !is_loopback(&url) {
        return Err(Error::Unreadable("plain HTTP is allowed only on loopback"));
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

pub(super) fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        // A literal IP avoids DNS deciding where plaintext credentials go.
        _ => false,
    }
}

pub(super) fn read_ca(path: PathBuf) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MOST_CA_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| Error::Unreadable("the CA file could not be read"))?;
    if bytes.len() as u64 > MOST_CA_BYTES {
        return Err(Error::Unreadable("the CA file exceeds 256 KiB"));
    }
    Ok(bytes)
}

pub(super) fn segment(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.len() > 253
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::Unreadable("invalid Results resource name"));
    }
    Ok(())
}

pub(super) fn request_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        return Error::Unreachable(Cause::TimedOut);
    }
    let mut source = error.source();
    while let Some(cause) = source {
        if matches!(
            cause.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented)
        ) {
            return Error::Unreachable(Cause::Certificate);
        }
        // io::Error's source may skip the wrapped TLS error itself.
        source = if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn std::error::Error + 'static))
        } else {
            cause.source()
        };
    }
    Error::Unreachable(if error.is_connect() {
        Cause::Connect
    } else {
        Cause::Transport
    })
}

#[derive(Deserialize)]
struct RecordPage {
    #[serde(default)]
    records: Vec<Value>,
    #[serde(default, rename = "nextPageToken", alias = "next_page_token")]
    next: String,
}

/// A byte cap is a partial listing outcome, private to this transport.
enum Page {
    Read(RecordPage),
    Capped,
}

impl Client {
    async fn page(&self, url: Url, until: Instant) -> Result<Page, Error> {
        let ends = until.min(Instant::now() + REQUEST_DEADLINE);
        tokio::time::timeout_at(ends, async {
            let mut request = self
                .http
                .get(url)
                .header(http::header::ACCEPT, "application/json");
            if let Some(token) = &self.token {
                request = request.header(http::header::AUTHORIZATION, token.clone());
            }
            let mut response = request.send().await.map_err(request_error)?;
            match response.status().as_u16() {
                200 => {}
                401 | 403 => {
                    return Err(Error::Refused {
                        status: response.status().as_u16(),
                        token_set: self.token.is_some(),
                    });
                }
                404 => return Err(Error::NotInstalled),
                status @ 502..=504 => {
                    return Err(Error::Unreachable(Cause::Gateway(status)));
                }
                status => return Err(Error::Http(status)),
            }
            // Count bytes as they arrive, including a chunked body with no
            // Content-Length. Never read an error body: it may repeat a key.
            if response
                .content_length()
                .is_some_and(|len| len > MAX_BODY_BYTES as u64)
            {
                return Ok(Page::Capped);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(request_error)? {
                if chunk.len() > MAX_BODY_BYTES - body.len() {
                    return Ok(Page::Capped);
                }
                body.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|_| Error::Unreadable("the response is not a Results record list"))?;
            let object = value.as_object().ok_or(Error::Unreadable(
                "the response is not a Results record list",
            ))?;
            // Protobuf JSON may omit an empty records array ({}). A foreign
            // error/login response must not turn into a successful empty read.
            if !object.is_empty()
                && !object.contains_key("records")
                && !object.contains_key("nextPageToken")
                && !object.contains_key("next_page_token")
            {
                return Err(Error::Unreadable(
                    "the response is not a Results record list",
                ));
            }
            serde_json::from_value(value)
                .map(Page::Read)
                .map_err(|_| Error::Unreadable("the Results record list has an invalid shape"))
        })
        .await
        .map_err(|_| Error::Unreachable(Cause::TimedOut))?
    }

    pub(super) async fn list<T>(
        &self,
        collection: &str,
        filter: &str,
        cap: usize,
        reason: Cap,
        order: Option<&str>,
        until: Instant,
        parse: impl Fn(&Value) -> Result<Record<T>, Error>,
    ) -> Result<Listing<T>, Error> {
        let mut records = Vec::new();
        let mut next = String::new();
        let mut observed = 0;
        let mut tokens = BTreeSet::new();
        for page_ix in 0..MAX_PAGES {
            if Instant::now() >= until {
                return Err(Error::Unreachable(Cause::TimedOut));
            }
            let mut url = self.base.clone();
            url.path_segments_mut()
                .map_err(|_| Error::Unreadable("invalid address"))?
                .pop_if_empty()
                .extend(["apis", "results.tekton.dev", "v1alpha2", "parents"])
                .extend(collection.split('/'));
            url.query_pairs_mut()
                .append_pair("filter", filter)
                .append_pair(
                    "page_size",
                    &(cap - records.len()).min(PAGE_LIMIT as usize).to_string(),
                );
            if let Some(order) = order {
                url.query_pairs_mut().append_pair("order_by", order);
            }
            if !next.is_empty() {
                url.query_pairs_mut().append_pair("page_token", &next);
            }
            let page = match self.page(url, until).await {
                Ok(Page::Read(page)) => page,
                Ok(Page::Capped) => {
                    return Ok(Listing {
                        records,
                        cap: Some(Cap::Bytes),
                        total: None,
                    });
                }
                Err(error) => return Err(error),
            };
            observed += page.records.len();
            if page.next.len() > MOST_PAGE_TOKEN_BYTES {
                return Err(Error::Unreadable(
                    "the next-page token exceeds the byte cap",
                ));
            }
            let remaining = cap - records.len();
            let over = page.records.len() > remaining;
            for raw in page.records.iter().take(remaining) {
                records.push(parse(raw)?);
            }
            let more = !page.next.is_empty();
            if over || (more && (records.len() == cap || page_ix + 1 == MAX_PAGES)) {
                let stopped = if over || records.len() == cap {
                    reason
                } else {
                    Cap::Pages
                };
                return Ok(Listing {
                    records,
                    cap: Some(stopped),
                    total: (!more).then_some(observed),
                });
            }
            if !more {
                return Ok(Listing {
                    records,
                    cap: None,
                    total: Some(observed),
                });
            }
            if !tokens.insert(page.next.clone()) {
                return Err(Error::Unreadable("the server repeated a next-page token"));
            }
            next = page.next;
        }
        Ok(Listing {
            records,
            cap: Some(Cap::Pages),
            total: None,
        })
    }
}
