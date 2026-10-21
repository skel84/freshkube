//! What a central store holds under the label that tells entries apart
//! (#821): which values of the label have series now, and which entries
//! those values are. One query, `count by (<label>) (up)`, answers for every
//! entry at once and keeps Check sources' meaning of "has series": `up`.
//!
//! [`label_counts`] reads; [`entry_labels`] is pure, from the answer and the
//! setup, so a save derives it again without another read. A value with no
//! series is **unknown**, not a failure, as [`series`](super) says it: a
//! store that answers but selects nothing proves nothing about the entry.
use std::collections::BTreeMap;

use super::{Resolved, connect};
use crate::cluster_source::ClusterAccess;
use crate::monitoring::prometheus::{api::ApiRequest, response};
use crate::setup::{self, Setup};

/// The most values of the label read; a store with more is cut and says so
/// by `capped`.
const MAX_VALUES: usize = 2_000;
/// The most unclaimed values an answer names.
pub const MAX_UNCLAIMED: usize = 20;

/// Every value of the label that has `up` series now, with how many.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelCounts {
    pub label: String,
    pub counts: BTreeMap<String, u64>,
    /// The store had more values than were read.
    pub capped: bool,
}

/// `count by (label) (up)` against a central store. Nothing else is
/// asked. Fails, with the words to show, for any other mode or label that
/// isn't a Prometheus label name, before any request.
// String exception: the reason is shown as the page's words.
pub async fn label_counts(
    resolved: &Resolved,
    host: &ClusterAccess,
    token: Option<String>,
) -> Result<LabelCounts, String> {
    let Resolved::Central { place, matcher, .. } = resolved else {
        return Err("Check entries needs the Central mode".to_owned());
    };
    let label = matcher.label();
    if !setup::is_label_name(label) {
        return Err("metrics.label isn’t a Prometheus label name".to_owned());
    }
    let (store, _) = connect(place, host, token)
        .await
        .map_err(|error| error.message)?;
    let query = format!("count by ({label}) (up)");
    let answer = store
        .get(&ApiRequest::new("query", vec![("query".into(), query)]))
        .await
        .map_err(|error| error.message)?;
    parse_counts(label, &answer)
}

fn parse_counts(label: &str, answer: &serde_json::Value) -> Result<LabelCounts, String> {
    let (data, _) = response::envelope(answer)
        .map_err(|_| "The store's answer wasn’t a Prometheus query result".to_owned())?;
    let samples = data
        .get("result")
        .and_then(|result| result.as_array())
        .ok_or_else(|| "The store's answer wasn’t a Prometheus query result".to_owned())?;
    let mut counts = BTreeMap::new();
    let mut capped = false;
    for sample in samples {
        let value = sample
            .get("metric")
            .and_then(|metric| metric.get(label))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty());
        let count = sample
            .get("value")
            .and_then(|pair| pair.get(1))
            .and_then(|count| count.as_str())
            .and_then(|count| count.parse::<f64>().ok())
            .filter(|count| *count > 0.);
        let (Some(value), Some(count)) = (value, count) else {
            continue;
        };
        if counts.len() >= MAX_VALUES {
            capped = true;
            break;
        }
        counts.insert(value.to_owned(), count as u64);
    }
    Ok(LabelCounts {
        label: label.to_owned(),
        counts,
        capped,
    })
}

/// What the store says of one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelState {
    /// The entry's value has this many series now.
    HasSeries(u64),
    /// It has none now. When the entry's context name holds series and no
    /// entry claims that value, `suggest` is it: `metrics.value: <context>`.
    NoSeries { suggest: Option<String> },
    /// The entry names its own store: not checked here.
    OwnSource,
}

/// One entry and the value of the label it reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryLabel {
    pub entry: String,
    /// `metrics.value`, else the entry's id.
    pub value: String,
    pub state: LabelState,
}

/// Every entry's state, and the values with series that no entry claims.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryLabels {
    pub entries: Vec<EntryLabel>,
    /// At most [`MAX_UNCLAIMED`], for hand edits; none is suggested.
    pub unclaimed: Vec<(String, u64)>,
    /// How many more unclaimed values there are.
    pub more: usize,
}

/// The entries of `setup` against what the store holds.
pub fn entry_labels(setup: &Setup, counts: &LabelCounts) -> EntryLabels {
    let value_of = |entry: &crate::workspace::Entry| {
        entry
            .metrics
            .as_deref()
            .and_then(|metrics| metrics.value.clone())
            .unwrap_or_else(|| entry.id.clone())
    };
    let own = |entry: &crate::workspace::Entry| {
        entry
            .metrics
            .as_deref()
            .is_some_and(|metrics| metrics.source.is_some())
    };
    let claimed: Vec<String> = setup
        .workspace
        .clusters
        .iter()
        .filter(|entry| !own(entry))
        .map(value_of)
        .collect();
    let entries = setup
        .workspace
        .clusters
        .iter()
        .map(|entry| {
            let value = value_of(entry);
            let state = if own(entry) {
                LabelState::OwnSource
            } else if let Some(count) = counts.counts.get(&value) {
                LabelState::HasSeries(*count)
            } else {
                let suggest = counts
                    .counts
                    .contains_key(&entry.context)
                    .then(|| entry.context.clone())
                    .filter(|context| !claimed.contains(context));
                LabelState::NoSeries { suggest }
            };
            EntryLabel {
                entry: entry.id.clone(),
                value,
                state,
            }
        })
        .collect();
    let mut unclaimed: Vec<(String, u64)> = counts
        .counts
        .iter()
        .filter(|(value, _)| !claimed.contains(value))
        .map(|(value, count)| (value.clone(), *count))
        .collect();
    let more = unclaimed.len().saturating_sub(MAX_UNCLAIMED);
    unclaimed.truncate(MAX_UNCLAIMED);
    EntryLabels {
        entries,
        unclaimed,
        more,
    }
}

/// What example data answers, against the example setup: the first entry's
/// value has series, the second's has none while its context name does, and
/// one value belongs to nobody.
pub fn example_label_counts(setup: &Setup) -> LabelCounts {
    let mut counts = BTreeMap::new();
    for (at, entry) in setup.workspace.clusters.iter().enumerate() {
        match at {
            0 => counts.insert(entry.id.clone(), 14),
            1 => counts.insert(entry.context.clone(), 6),
            _ => None,
        };
    }
    counts.insert("legacy".to_owned(), 2);
    LabelCounts {
        label: setup.metrics.label_name().to_owned(),
        counts,
        capped: false,
    }
}
