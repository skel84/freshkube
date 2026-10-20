//! Log metadata is paged during the archived build read; content is fetched
//! only on demand. Storage paths and annotated URLs are never fetched.
use tokio::time::Instant;

use super::records::{manifest, result_parts};
use super::transport::{Query, request_error, segment};
use super::{Cap, Cause, Client, Error, Listing, MOST_RESULT_TASKS, REQUEST_DEADLINE, Record};

/// A whole task log, never inferred by splitting step prefixes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Log {
    pub(crate) name: String,
    pub(crate) task_uid: String,
    pub(crate) stored: bool,
}

/// One MiB is enough for inspection without retaining an unbounded log.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "slice 9 consumes the on-demand Results log API")
)]
pub(super) const MOST_LOG_BYTES: usize = 1024 * 1024;

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "slice 9 consumes the on-demand Results log API")
)]
#[derive(PartialEq, Eq)]
pub(super) struct Content {
    pub(super) text: String,
    pub(super) capped: bool,
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Content")
            .field("bytes", &self.text.len())
            .field("capped", &self.capped)
            .finish()
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "slice 9 consumes the on-demand Results log API")
)]
impl Content {
    pub(super) fn note(&self) -> Option<&'static str> {
        self.capped.then_some("Showing the first 1 MiB")
    }
}

impl Client {
    pub(crate) async fn logs(&self, result: &str, until: Instant) -> Result<Listing<Log>, Error> {
        result_parts(result)?;
        self.list(
            &format!("{result}/logs"),
            Query {
                filter: "",
                order: None,
            },
            MOST_RESULT_TASKS,
            Cap::Logs,
            until,
            |raw| {
                let (record, actual, name, value) = manifest(raw, "Log")?;
                if actual != result {
                    return Err(Error::Unreadable(
                        "the log does not belong to the requested result",
                    ));
                }
                let uid = value
                    .pointer("/spec/resource/uid")
                    .and_then(|v| v.as_str())
                    .filter(|uid| !uid.is_empty())
                    .ok_or(Error::Unreadable("the log names no resource UID"))?;
                let size = value
                    .pointer("/status/size")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let old = value["apiVersion"] == "results.tekton.dev/v1alpha2";
                let stored = size > 0
                    && (old
                        || value.pointer("/status/isStored").and_then(|v| v.as_bool())
                            == Some(true));
                Ok(Record {
                    name: record,
                    result: actual,
                    run: Log {
                        name,
                        task_uid: uid.into(),
                        stored,
                    },
                })
            },
        )
        .await
        .map_err(|error| match error {
            Error::NotInstalled | Error::Http(501) => Error::LogsNotServed,
            error => error,
        })
    }

    /// The name comes from the checked Log record, never its storage path.
    /// Read one byte beyond the content cap to distinguish exactly 1 MiB
    /// from a longer stream. The HTTP body is dropped as soon as it is known.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "slice 9 consumes the on-demand Results log API")
    )]
    pub(super) async fn log(
        &self,
        result: &str,
        name: &str,
        until: Instant,
    ) -> Result<Content, Error> {
        result_parts(result)?;
        if Instant::now() >= until {
            return Err(Error::Unreachable(Cause::TimedOut));
        }
        segment(name)?;
        if name == "-" {
            return Err(Error::Unreadable("the log has a wildcard name"));
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Unreadable("invalid address"))?
            .pop_if_empty()
            .extend(["apis", "results.tekton.dev", "v1alpha2", "parents"])
            .extend(result.split('/'))
            .extend(["logs", name]);
        tokio::time::timeout_at(until.min(Instant::now() + REQUEST_DEADLINE), async {
            let mut request = self
                .http
                .get(url)
                .header(http::header::ACCEPT, "text/plain");
            if let Some(token) = &self.token {
                request = request.header(http::header::AUTHORIZATION, token.clone());
            }
            let mut response = request.send().await.map_err(request_error)?;
            match response.status().as_u16() {
                200 => {}
                404 => return Err(Error::LogMissing),
                status @ (401 | 403) => {
                    return Err(Error::Refused {
                        status,
                        token_set: self.token.is_some(),
                    });
                }
                status @ 502..=504 => return Err(Error::Unreachable(Cause::Gateway(status))),
                status => return Err(Error::Http(status)),
            }
            if response
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_none_or(|kind| {
                    kind.split(';').next().unwrap_or_default().trim() != "text/plain"
                })
            {
                return Err(Error::Unreadable("the log response is not plain text"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(request_error)? {
                let take = chunk.len().min(MOST_LOG_BYTES + 1 - bytes.len());
                bytes.extend_from_slice(&chunk[..take]);
                if bytes.len() > MOST_LOG_BYTES {
                    break;
                }
            }
            let capped = bytes.len() > MOST_LOG_BYTES;
            bytes.truncate(MOST_LOG_BYTES);
            Ok(Content {
                text: String::from_utf8_lossy(&bytes).into_owned(),
                capped,
            })
        })
        .await
        .map_err(|_| Error::Unreachable(Cause::TimedOut))?
    }
}
