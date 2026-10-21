//! The namespace builds run in, found from where PipelineRuns are.
//!
//! One metadata-only list of PipelineRuns across the cluster
//! ([`Reader::namespace_counts`]) is counted by namespace. Without Kargo's
//! projects to compare against, a namespace is suggested only when every
//! run is in it; several namespaces are listed and nothing is suggested.
//! A list cut at the cap suggests nothing, since the namespace that holds
//! the rest may be missing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::entry::connect_entry;
use crate::read::{NAMESPACE_COUNT_LIMIT, Reader};
use crate::source::printable;
use crate::versions::resolve;
use crate::{tekton, words};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::Entry;

/// The most namespaces an item lists before it says how many more.
pub const MAX_LISTED: usize = 10;

/// How many runs each namespace holds, and whether the list was cut.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamespaceCounts {
    pub counts: BTreeMap<String, u64>,
    pub capped: bool,
}

/// The counts of a list's `items[].metadata.namespace`, or of a
/// `PartialObjectMetadataList`. Only the namespace is read.
pub fn count_namespaces(list: &Value) -> NamespaceCounts {
    let items = list
        .get("items")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut counts = BTreeMap::new();
    for item in items.iter().take(NAMESPACE_COUNT_LIMIT) {
        if let Some(namespace) = item
            .pointer("/metadata/namespace")
            .and_then(Value::as_str)
            .filter(|namespace| !namespace.is_empty())
        {
            *counts.entry(namespace.to_owned()).or_default() += 1;
        }
    }
    let more = list
        .pointer("/metadata/continue")
        .and_then(Value::as_str)
        .is_some_and(|token| !token.is_empty())
        || list
            .pointer("/metadata/remainingItemCount")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0);
    NamespaceCounts {
        counts,
        capped: items.len() >= NAMESPACE_COUNT_LIMIT || more,
    }
}

/// What the read of PipelineRuns came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildRead {
    /// Nothing was read: Settings knows no entry builds are read from.
    NotRead(String),
    /// The entry couldn't be reached.
    Unreached(String),
    /// The entry doesn't serve PipelineRuns.
    NotServed(String),
    /// The identity may not list PipelineRuns.
    Refused(String),
    /// Any other failure, such as a deadline.
    Failed(String),
    Done(NamespaceCounts),
}

/// What a find kept: the entry builds are read from, and what it read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFindings {
    pub entry: String,
    pub read: BuildRead,
}

/// Counts PipelineRuns by namespace through `entry`, within the entry's
/// connect deadline and the read's own.
pub async fn find_builds(kubeconfigs: Vec<PathBuf>, entry: Entry) -> BuildFindings {
    let id = entry.id.clone();
    let read = match connect_entry(kubeconfigs, entry).await {
        Ok(reader) => read_builds(&reader, &id).await,
        Err(failure) => BuildRead::Unreached(printable(&failure)),
    };
    BuildFindings { entry: id, read }
}

pub(crate) async fn read_builds<R: Reader>(reader: &R, entry: &str) -> BuildRead {
    let resource = match resolve(
        reader,
        tekton::TEKTON_GROUP,
        "pipelineruns",
        tekton::TEKTON_VERSIONS,
        true,
    )
    .await
    {
        Ok(resource) => resource,
        Err(failure) => return classify(failure, entry),
    };
    match reader.namespace_counts(&resource).await {
        Ok(counts) => BuildRead::Done(counts),
        Err(failure) => classify(failure, entry),
    }
}

fn classify(failure: Failure, entry: &str) -> BuildRead {
    match failure.kind {
        FailureKind::NotFound => {
            BuildRead::NotServed(format!("{entry} doesn't serve PipelineRuns"))
        }
        FailureKind::Forbidden | FailureKind::Unauthorized => BuildRead::Refused(words::failed(
            FailureKind::Forbidden,
            "PipelineRuns",
            entry,
            &printable(&failure),
        )),
        _ => BuildRead::Failed(printable(&failure)),
    }
}

/// What the runs' namespaces come to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildVerdict {
    /// Every run is in this namespace.
    Suggest(String),
    /// The runs are in several namespaces, the busiest first and at most
    /// [`MAX_LISTED`] named; no namespace is suggested.
    Several {
        listed: Vec<(String, u64)>,
        more: usize,
    },
    /// The list was cut, so which namespace holds the rest is unknown.
    Capped,
}

impl BuildVerdict {
    pub fn is_suggestion(&self) -> bool {
        matches!(self, BuildVerdict::Suggest(_))
    }
}

/// The one item a find makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildItem {
    /// What identifies it: the suggested namespace, else `""`.
    pub key: String,
    pub found_from: String,
    pub verdict: BuildVerdict,
    /// Every namespace that holds runs, with its count, for the Inspector.
    pub evidence: Vec<(String, u64)>,
}

/// A line about the read, or about the value set now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub warn: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildSuggestions {
    pub item: Option<BuildItem>,
    pub notes: Vec<Note>,
}

/// The suggestion `findings` make against `current`, the setup's
/// `delivery.build_namespace`. Pure.
pub fn suggest_build_namespace(
    findings: &BuildFindings,
    current: Option<&str>,
) -> BuildSuggestions {
    let mut out = BuildSuggestions::default();
    let note = |out: &mut BuildSuggestions, warn, text: String| out.notes.push(Note { warn, text });
    let entry = &findings.entry;
    let counts = match &findings.read {
        BuildRead::NotRead(why) => {
            note(&mut out, false, format!("Nothing was read: {why}"));
            return out;
        }
        BuildRead::Unreached(why) => {
            note(
                &mut out,
                true,
                format!("{entry} couldn't be reached to count PipelineRuns: {why}"),
            );
            return out;
        }
        BuildRead::NotServed(why) | BuildRead::Refused(why) => {
            note(&mut out, true, why.clone());
            return out;
        }
        BuildRead::Failed(why) => {
            note(
                &mut out,
                true,
                format!("Counting PipelineRuns on {entry} failed: {why}"),
            );
            return out;
        }
        BuildRead::Done(counts) => counts,
    };
    if counts.counts.is_empty() {
        note(&mut out, false, "No PipelineRuns found".to_owned());
        return out;
    }
    if let Some(current) = current
        && !counts.counts.contains_key(current)
        && !counts.capped
    {
        note(
            &mut out,
            true,
            format!("delivery.build_namespace is {current}, which holds no PipelineRuns"),
        );
    }
    let mut evidence: Vec<(String, u64)> = counts
        .counts
        .iter()
        .map(|(namespace, count)| (namespace.clone(), *count))
        .collect();
    evidence.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let found_from = format!("PipelineRuns on {entry}");
    let verdict = if counts.capped {
        note(
            &mut out,
            true,
            format!(
                "More than {NAMESPACE_COUNT_LIMIT} PipelineRuns on {entry}; no namespace is \
                 suggested from a partial list"
            ),
        );
        BuildVerdict::Capped
    } else if let [(only, _)] = evidence.as_slice() {
        if current == Some(only.as_str()) {
            return out;
        }
        BuildVerdict::Suggest(only.clone())
    } else {
        let more = evidence.len().saturating_sub(MAX_LISTED);
        BuildVerdict::Several {
            listed: evidence.iter().take(MAX_LISTED).cloned().collect(),
            more,
        }
    };
    let key = match &verdict {
        BuildVerdict::Suggest(namespace) => namespace.clone(),
        _ => String::new(),
    };
    out.item = Some(BuildItem {
        key,
        found_from,
        verdict,
        evidence,
    });
    out
}
