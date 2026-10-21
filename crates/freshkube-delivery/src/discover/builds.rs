//! The namespace builds run in, found from where PipelineRuns are.
//!
//! One metadata-only list of PipelineRuns across the cluster
//! ([`Reader::namespace_counts`]) is counted by namespace, and Kargo's
//! Projects are listed by name. A `{project}` template
//! (`delivery.build_namespace`, #820) is found when the namespaces line up
//! with the projects' names: each namespace that contains a project name
//! gives a candidate, and a candidate's fit is the projects whose resolved
//! namespace holds runs. The best fit of at least two projects is suggested;
//! a tie chooses neither. Without a fit, a namespace is suggested only when
//! every run is in it. When the run list was cut at its cap, the best
//! candidatess are confirmed by one `limit=1` read per project namespace
//! ([`Reader::any_in`]) for the two best candidates, so a tie shows, and
//! nothing is suggested from a partial list otherwise.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use super::destinations::Note;
use crate::entry::connect_entry;
use crate::kargo::read_project_names;
use crate::read::Resource;
use crate::read::{NAMESPACE_COUNT_LIMIT, Reader};
use crate::source::Source;
use crate::source::printable;
use crate::versions::resolve;
use crate::{tekton, words};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::setup::resolve_build_namespace;
use freshkube_core::workspace::Entry;

/// The most namespaces an item lists before it says how many more.
pub const MAX_LISTED: usize = 10;
/// The most Kargo projects whose namespaces are probed when the run list
/// was cut; with more, nothing is suggested.
pub const MAX_PROBED: usize = 50;
/// How many candidate templates are probed when the run list was cut, so a
/// tie between the best two shows.
const PROBED_TEMPLATES: usize = 2;

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

/// What the read of Kargo's Projects came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Projects {
    /// Nothing was read, with why.
    NotRead(String),
    /// Every Project's name.
    Read(Vec<String>),
    /// The listing was cut at its cap: no template is suggested from it.
    Capped,
    Refused(String),
    Failed(String),
}

/// What reading one project namespace for a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    Runs,
    Empty,
    Failed(String),
}

/// What a find kept: the entry builds are read from, and what it read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFindings {
    pub entry: String,
    pub read: BuildRead,
    pub projects: Projects,
    /// Per project namespace, when the run list was cut.
    pub probes: Vec<(String, Probe)>,
}

impl BuildFindings {
    /// Findings with the runs alone, as when no entry reads Kargo.
    pub fn runs_only(entry: impl Into<String>, read: BuildRead) -> Self {
        Self {
            entry: entry.into(),
            read,
            projects: Projects::NotRead("no entry reads Kargo".to_owned()),
            probes: Vec::new(),
        }
    }
}

/// Counts PipelineRuns by namespace through `builds`, and lists Kargo's
/// Projects through `kargo`, within each entry's connect deadline and the
/// reads' own.
pub async fn find_builds(
    kubeconfigs: Vec<PathBuf>,
    builds: Entry,
    kargo: Option<Entry>,
) -> BuildFindings {
    let id = builds.id.clone();
    let same = kargo.as_ref().is_some_and(|kargo| kargo.id == builds.id);
    let built = connect_entry(kubeconfigs.clone(), builds).await;
    let other = match (&kargo, same) {
        (Some(kargo), false) => Some(connect_entry(kubeconfigs, kargo.clone()).await),
        _ => None,
    };
    let reader = match &built {
        Ok(reader) => Ok(reader),
        Err(failure) => Err(printable(failure)),
    };
    let kargo_reader = match (&kargo, &other, &built) {
        (None, _, _) => None,
        (Some(_), None, Ok(reader)) => Some(Ok(reader)),
        (Some(_), None, Err(failure)) => Some(Err(printable(failure))),
        (Some(_), Some(Ok(reader)), _) => Some(Ok(reader)),
        (Some(_), Some(Err(failure)), _) => Some(Err(printable(failure))),
    };
    read_all(reader, kargo_reader, &id).await
}

/// The whole read: the runs, the projects, and, for a cut run list, the
/// probes that confirm the best template.
pub(crate) async fn read_all<B: Reader, K: Reader>(
    builds: Result<&B, String>,
    kargo: Option<Result<&K, String>>,
    entry: &str,
) -> BuildFindings {
    let (read, resource) = match &builds {
        Ok(reader) => read_runs(*reader, entry).await,
        Err(why) => (BuildRead::Unreached(why.clone()), None),
    };
    let projects = match kargo {
        None => Projects::NotRead("no entry reads Kargo".to_owned()),
        Some(Err(why)) => Projects::Failed(why),
        Some(Ok(reader)) => match read_project_names(reader).await {
            Source::Read(names) => Projects::Read(names),
            Source::Capped(..) => Projects::Capped,
            Source::Refused(why) => Projects::Refused(why),
            Source::NotInstalled(why) | Source::Unreadable(why) => Projects::Failed(why),
        },
    };
    let mut probes = Vec::new();
    if let (Ok(reader), BuildRead::Done(counts), Projects::Read(names), Some(resource)) =
        (builds, &read, &projects, resource)
        && counts.capped
        && names.len() <= MAX_PROBED
    {
        let candidates = rank(
            counts.counts.keys().map(String::as_str),
            names,
            |namespace| counts.counts.contains_key(namespace),
        );
        for candidate in candidates.iter().take(PROBED_TEMPLATES) {
            for name in names {
                let Ok(namespace) = resolve_build_namespace(&candidate.template, name) else {
                    continue;
                };
                if probes.iter().any(|(probed, _)| *probed == namespace) {
                    continue;
                }
                let probe = match reader.any_in(&resource, &namespace).await {
                    Ok(true) => Probe::Runs,
                    Ok(false) => Probe::Empty,
                    Err(failure) => Probe::Failed(printable(&failure)),
                };
                probes.push((namespace, probe));
            }
        }
    }
    BuildFindings {
        entry: entry.to_owned(),
        read,
        projects,
        probes,
    }
}

#[cfg(test)]
pub(crate) async fn read_builds<R: Reader>(reader: &R, entry: &str) -> BuildRead {
    read_runs(reader, entry).await.0
}

async fn read_runs<R: Reader>(reader: &R, entry: &str) -> (BuildRead, Option<Resource>) {
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
        Err(failure) => return (classify(failure, entry), None),
    };
    let read = match reader.namespace_counts(&resource).await {
        Ok(counts) => BuildRead::Done(counts),
        Err(failure) => classify(failure, entry),
    };
    (read, Some(resource))
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

/// One candidate template and the projects it fits.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fit {
    template: String,
    fits: Vec<String>,
}

/// The candidate templates, best fit first (then by text). A candidate is
/// the namespace with a project's name replaced by `{project}`, kept only
/// when #820's validation accepts it; it fits the projects whose resolved
/// namespace `has` runs.
fn rank<'a>(
    namespaces: impl Iterator<Item = &'a str>,
    projects: &[String],
    has: impl Fn(&str) -> bool,
) -> Vec<Fit> {
    let mut templates = std::collections::BTreeSet::new();
    for namespace in namespaces {
        for project in projects.iter().filter(|project| !project.is_empty()) {
            for (at, _) in namespace.match_indices(project.as_str()) {
                let template = format!(
                    "{}{{project}}{}",
                    &namespace[..at],
                    &namespace[at + project.len()..]
                );
                if resolve_build_namespace(&template, "p").is_ok() {
                    templates.insert(template);
                }
            }
        }
    }
    let mut fits: Vec<Fit> = templates
        .into_iter()
        .map(|template| {
            let fits = projects
                .iter()
                .filter(|project| {
                    resolve_build_namespace(&template, project)
                        .is_ok_and(|namespace| has(&namespace))
                })
                .cloned()
                .collect();
            Fit { template, fits }
        })
        .collect();
    fits.sort_by(|a, b| {
        b.fits
            .len()
            .cmp(&a.fits.len())
            .then_with(|| a.template.cmp(&b.template))
    });
    fits
}

/// What the runs' namespaces come to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildVerdict {
    /// Every run is in this namespace.
    Suggest(String),
    /// The `{project}` template that fits at least two projects, best.
    Template {
        template: String,
        /// The projects whose namespace holds runs.
        fits: Vec<String>,
        /// The projects whose namespace holds none: their changes find
        /// builds by commit only.
        no_runs: Vec<String>,
        /// Namespaces with runs that the template names for no project, the
        /// busiest first. Any makes the fit partial.
        unexplained: Vec<(String, u64)>,
        /// The run list was cut, so the unexplained are at least these.
        at_least: bool,
    },
    /// Two templates fit equally; neither is chosen.
    Tie(Vec<String>),
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
    /// Whether it can be accepted.
    pub fn is_suggestion(&self) -> bool {
        matches!(
            self,
            BuildVerdict::Suggest(_) | BuildVerdict::Template { .. }
        )
    }

    /// What Accept sets `delivery.build_namespace` to.
    pub fn value(&self) -> Option<&str> {
        match self {
            BuildVerdict::Suggest(value) => Some(value),
            BuildVerdict::Template { template, .. } => Some(template),
            _ => None,
        }
    }
}

/// The one item a find makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildItem {
    /// What identifies it: the value it suggests, else `""`.
    pub key: String,
    pub found_from: String,
    pub verdict: BuildVerdict,
    /// Every namespace that holds runs, with its count, for the Inspector.
    pub evidence: Vec<(String, u64)>,
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
        && !current.contains("{project}")
        && !counts.counts.contains_key(current)
        && !counts.capped
    {
        note(
            &mut out,
            true,
            format!("delivery.build_namespace is {current}, which holds no PipelineRuns"),
        );
    }
    if let Some(current) = current
        && !matches!(findings.projects, Projects::Read(_))
        && counts
            .counts
            .keys()
            .any(|namespace| template_matches(current, namespace))
    {
        note(
            &mut out,
            false,
            format!(
                "delivery.build_namespace is {current}, a template that still names namespaces \
                 with PipelineRuns; Kargo's projects weren't read, so nothing is suggested"
            ),
        );
        return out;
    }
    let mut evidence: Vec<(String, u64)> = counts
        .counts
        .iter()
        .map(|(namespace, count)| (namespace.clone(), *count))
        .collect();
    evidence.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let found_from = format!("PipelineRuns on {entry}");
    let verdict = match template_verdict(findings, counts, &evidence, &mut out, entry) {
        Some(verdict) => verdict,
        None if counts.capped => {
            note(
                &mut out,
                true,
                format!(
                    "More than {NAMESPACE_COUNT_LIMIT} PipelineRuns on {entry}; no namespace is \
                     suggested from a partial list"
                ),
            );
            BuildVerdict::Capped
        }
        None => match evidence.as_slice() {
            [(only, _)] => BuildVerdict::Suggest(only.clone()),
            _ => BuildVerdict::Several {
                listed: evidence.iter().take(MAX_LISTED).cloned().collect(),
                more: evidence.len().saturating_sub(MAX_LISTED),
            },
        },
    };
    if verdict.value().is_some_and(|value| Some(value) == current) {
        return out;
    }
    out.item = Some(BuildItem {
        key: verdict.value().unwrap_or_default().to_owned(),
        found_from,
        verdict,
        evidence,
    });
    out
}

/// Whether `namespace` is what `template` resolves to for some project name.
fn template_matches(template: &str, namespace: &str) -> bool {
    let Some((before, rest)) = template.split_once("{project}") else {
        return false;
    };
    let after = rest
        .rsplit_once("{project}")
        .map_or(rest, |(_, after)| after);
    namespace.len() > before.len() + after.len()
        && namespace.starts_with(before)
        && namespace.ends_with(after)
}

/// The template verdict, when the projects allow one: `None` to fall back to
/// a plain namespace, the several or the cap.
fn template_verdict(
    findings: &BuildFindings,
    counts: &NamespaceCounts,
    evidence: &[(String, u64)],
    out: &mut BuildSuggestions,
    entry: &str,
) -> Option<BuildVerdict> {
    let names = match &findings.projects {
        Projects::Read(names) if names.len() >= 2 => names,
        Projects::Read(_) => return None,
        other => {
            if evidence.len() > 1 {
                let why = match other {
                    Projects::NotRead(why) | Projects::Refused(why) | Projects::Failed(why) => {
                        why.clone()
                    }
                    _ => "the listing was cut at its cap".to_owned(),
                };
                out.notes.push(Note {
                    warn: false,
                    text: format!(
                        "Kargo's projects weren't read ({why}); no template is suggested"
                    ),
                });
            }
            return None;
        }
    };
    if counts.capped && names.len() > MAX_PROBED {
        out.notes.push(Note {
            warn: true,
            text: format!(
                "More than {MAX_PROBED} Kargo projects and more than {NAMESPACE_COUNT_LIMIT} \
                 PipelineRuns on {entry}; nothing is suggested"
            ),
        });
        return Some(BuildVerdict::Capped);
    }
    if counts.capped {
        let mut unchecked: Vec<&str> = findings
            .probes
            .iter()
            .filter(|(_, probe)| matches!(probe, Probe::Failed(_)))
            .map(|(namespace, _)| namespace.as_str())
            .collect();
        unchecked.dedup();
        if !unchecked.is_empty() {
            let listed = unchecked
                .iter()
                .take(MAX_LISTED)
                .copied()
                .collect::<Vec<_>>();
            let more = unchecked.len().saturating_sub(MAX_LISTED);
            out.notes.push(Note {
                warn: true,
                text: format!(
                    "Couldn't check {}{} for PipelineRuns; nothing is suggested",
                    listed.join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                ),
            });
            return Some(BuildVerdict::Capped);
        }
    }
    let has = |namespace: &str| match counts.capped {
        true => findings
            .probes
            .iter()
            .any(|(probed, probe)| probed == namespace && *probe == Probe::Runs),
        false => counts.counts.contains_key(namespace),
    };
    let ranked = rank(counts.counts.keys().map(String::as_str), names, has);
    let best = ranked.first().filter(|best| best.fits.len() >= 2)?;
    let tied: Vec<String> = ranked
        .iter()
        .filter(|fit| fit.fits.len() == best.fits.len())
        .map(|fit| fit.template.clone())
        .collect();
    if tied.len() > 1 {
        return Some(BuildVerdict::Tie(tied));
    }
    if counts.capped
        && rank(
            counts.counts.keys().map(String::as_str),
            names,
            |namespace| counts.counts.contains_key(namespace),
        )
        .len()
            > PROBED_TEMPLATES
    {
        out.notes.push(Note {
            warn: false,
            text: format!(
                "More than {NAMESPACE_COUNT_LIMIT} PipelineRuns on {entry}; only the two most \
                 common templates were compared, so another may fit as well"
            ),
        });
    }
    let resolved: Vec<String> = names
        .iter()
        .filter_map(|name| resolve_build_namespace(&best.template, name).ok())
        .collect();
    let unexplained: Vec<(String, u64)> = evidence
        .iter()
        .filter(|(namespace, _)| !resolved.contains(namespace))
        .cloned()
        .collect();
    let no_runs = names
        .iter()
        .filter(|name| !best.fits.contains(name))
        .cloned()
        .collect();
    Some(BuildVerdict::Template {
        template: best.template.clone(),
        fits: best.fits.clone(),
        no_runs,
        unexplained,
        at_least: counts.capped,
    })
}
