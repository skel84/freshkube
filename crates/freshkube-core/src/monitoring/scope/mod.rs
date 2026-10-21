//! Where the open entry's metrics are read, from the setup file's `metrics`
//! block (`docs/SETUP_FILE.md`, "Metrics").
//!
//! [`resolve`] is pure: the setup and the open entry's id in, one
//! [`Resolved`] out. An entry's own `source` wins under any mode. Under
//! `central` every entry reads the one store, told apart by the label's
//! [`Matcher`]. Under `per-entry` an entry with no source has no metrics, and
//! says so. `automatic`, and a context that is no entry, keep discovery as
//! it was before the file.
//!
//! **A wrong scope draws another cluster's numbers as this one's**, so every
//! doubt is [`Resolved::Unknown`], never a guess.
use super::{
    Matcher, Prometheus, PrometheusService, QueryError, cluster_client, confirm, confirm_url,
    forget_after, normalise_url, prometheus::response,
};
use crate::cluster_source::ClusterAccess;
use crate::setup::{self, Mode, Setup};

/// URLs whose tokens the setup names, including stores retained under an
/// inactive mode. Equivalent URL spellings name the same token account.
pub fn token_urls(setup: &Setup) -> std::collections::BTreeSet<String> {
    let central = setup.metrics.store.iter().filter_map(|store| {
        store.token.as_ref()?;
        store.url.as_deref()
    });
    let entries = setup.workspace.clusters.iter().filter_map(|entry| {
        let source = entry.metrics.as_ref()?.source.as_ref()?;
        source.token.as_ref()?;
        source.url.as_deref()
    });
    central
        .chain(entries)
        .filter_map(|url| normalise_url(url).ok())
        .collect()
}

/// Where a store is: a Service in a cluster, or a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// Reached through its cluster's API server proxy: the hosting entry's.
    Service(PrometheusService),
    /// A URL as [`normalise_url`] returns it. `token` says the bearer token
    /// is kept in `keys.json`.
    Url { url: String, token: bool },
}

/// How the open entry reads its metrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// Find Prometheus in the open cluster, as before the file.
    Discovery,
    /// The entry's own store, in its own cluster or at a URL; unscoped.
    Own(Place),
    /// The one store every entry reads, scoped to this entry's series.
    Central {
        place: Place,
        /// The workspace entry whose cluster hosts a Service; none for a
        /// URL.
        host: Option<String>,
        matcher: Matcher,
    },
    /// No metrics, and why, for the page to say.
    Unknown(String),
}

impl Resolved {
    /// Whether discovery, and what Settings chose for the context, is how
    /// this entry's metrics are found.
    pub fn is_discovery(&self) -> bool {
        matches!(self, Self::Discovery)
    }

    /// The matcher scoping the store's series to this entry.
    pub fn matcher(&self) -> Option<&Matcher> {
        match self {
            Self::Central { matcher, .. } => Some(matcher),
            _ => None,
        }
    }

    /// The workspace entry whose cluster the store's Service is in, when it
    /// isn't the open entry's own.
    pub fn host(&self) -> Option<&str> {
        match self {
            Self::Central { host, .. } => host.as_deref(),
            _ => None,
        }
    }
}

/// How `entry` (a workspace entry's id) reads its metrics under `setup`. A
/// context outside the workspace has no entry, and keeps discovery.
pub fn resolve(setup: &Setup, entry: Option<&str>) -> Resolved {
    let Some(entry) = entry.and_then(|id| setup.workspace.clusters.iter().find(|e| e.id == id))
    else {
        return Resolved::Discovery;
    };
    let own = entry.metrics.as_deref();
    if let Some(source) = own.and_then(|metrics| metrics.source.as_ref()) {
        return match place(
            source.service.as_ref(),
            source.url.as_deref(),
            source.token.is_some(),
        ) {
            Ok(place) => Resolved::Own(place),
            Err(why) => Resolved::Unknown(format!("{}'s metrics source: {why}", entry.id)),
        };
    }
    match setup.metrics.mode {
        Mode::Automatic => Resolved::Discovery,
        Mode::PerEntry => Resolved::Unknown(format!("{} names no metrics source", entry.id)),
        Mode::Central => {
            let Some(store) = &setup.metrics.store else {
                return Resolved::Unknown("metrics.store is missing".into());
            };
            let host = match (&store.entry, &store.service) {
                (Some(host), Some(_)) => {
                    if !setup.workspace.clusters.iter().any(|e| &e.id == host) {
                        return Resolved::Unknown(format!(
                            "metrics.store.entry {host} isn't in the workspace"
                        ));
                    }
                    Some(host.clone())
                }
                _ => None,
            };
            let place = match place(
                store.service.as_ref(),
                store.url.as_deref(),
                store.token.is_some(),
            ) {
                Ok(place) => place,
                Err(why) => return Resolved::Unknown(format!("metrics.store: {why}")),
            };
            let value = own
                .and_then(|metrics| metrics.value.as_deref())
                .unwrap_or(&entry.id);
            Resolved::Central {
                place,
                host,
                matcher: Matcher::new(setup.metrics.label_name(), value),
            }
        }
    }
}

// String exception: the reason is shown as the page's words for no metrics.
fn place(
    service: Option<&setup::MetricsService>,
    url: Option<&str>,
    token: bool,
) -> Result<Place, String> {
    match (service, url) {
        (Some(service), None) => {
            let mut found = PrometheusService::new(&service.namespace, &service.name, service.port)
                .with_path(&service.path);
            found.https = service.https;
            Ok(Place::Service(found))
        }
        (None, Some(url)) => Ok(Place::Url {
            url: normalise_url(url)?,
            token,
        }),
        _ => Err("it needs exactly one of a service and a url".into()),
    }
}

/// Confirms the store answers, through `host`'s client for a Service. The
/// result is unscoped: the caller adds the entry's matcher.
pub async fn connect(
    place: &Place,
    host: &ClusterAccess,
    token: Option<String>,
) -> Result<(Prometheus, Option<String>), QueryError> {
    match place {
        Place::Service(service) => {
            let client = cluster_client(host).await?;
            confirm(&client, service.clone())
                .await
                .inspect_err(|error| forget_after(host, error))
        }
        Place::Url { url, .. } => confirm_url(url.clone(), token).await,
    }
}

/// How one check of a central store came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Not known, and never a pass: a store that answers but selects nothing
    /// is not proof that it holds this entry's series.
    Unknown(String),
    Fail(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub what: &'static str,
    pub verdict: Verdict,
}

/// What Check sources says of a central store: that it answers, and that the
/// entry's matcher selects series. Other modes have no checks here.
pub async fn check(resolved: &Resolved, host: &ClusterAccess, token: Option<String>) -> Vec<Check> {
    let Resolved::Central { place, matcher, .. } = resolved else {
        return Vec::new();
    };
    let store = match connect(place, host, token).await {
        Ok((prometheus, _)) => prometheus,
        Err(error) => {
            return vec![
                Check {
                    what: STORE,
                    verdict: Verdict::Fail(error.message),
                },
                Check {
                    what: SERIES,
                    verdict: Verdict::Unknown("the store didn't answer".into()),
                },
            ];
        }
    };
    vec![
        Check {
            what: STORE,
            verdict: Verdict::Pass,
        },
        Check {
            what: SERIES,
            verdict: series(&store, matcher).await,
        },
    ]
}

const STORE: &str = "The store answers";
const SERIES: &str = "The entry's series are in the store";

/// `count(up{label="value"})`, read unscoped. An empty answer is unknown.
async fn series(store: &Prometheus, matcher: &Matcher) -> Verdict {
    let query = format!("count(up{})", matcher.selector());
    let answer = store
        .get(&super::prometheus::api::ApiRequest::new(
            "query",
            vec![("query".into(), query)],
        ))
        .await;
    let value = match answer {
        Ok(value) => value,
        Err(error) => return Verdict::Unknown(error.message),
    };
    let count = response::envelope(&value)
        .ok()
        .and_then(|(data, _)| data.get("result")?.as_array()?.first())
        .and_then(|sample| sample.get("value")?.get(1)?.as_str())
        .and_then(|count| count.parse::<f64>().ok())
        .filter(|count| *count > 0.);
    match count {
        Some(_) => Verdict::Pass,
        None => Verdict::Unknown(format!(
            "No series in the store match {}=\"{}\"; check the label the cluster's agent adds",
            matcher.label(),
            matcher.value()
        )),
    }
}

mod labels;
pub use labels::{
    EntryLabel, EntryLabels, LabelCounts, LabelState, MAX_UNCLAIMED, entry_labels,
    example_label_counts, label_counts,
};

#[cfg(test)]
mod tests;
