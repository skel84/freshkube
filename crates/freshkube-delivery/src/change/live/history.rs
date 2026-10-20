//! Where a build was read, kept separate from what its manifest reports.
use crate::change::{Gap, Hop, Missing, Shows, Target, Value};
use crate::join::{Evidence, Hop as Joined, Link};
use crate::results::{Cap, Error, History};
use crate::tekton::{self, Build};

/// Names may be reused. Prefer the UIDs the link's observations carry, and
/// otherwise return a name only when it identifies exactly one read build.
pub(super) fn build<'a>(evidence: &'a Evidence, link: &Link) -> Option<&'a Build> {
    if !matches!(link.to, Joined::PipelineRun | Joined::SupplyChain) {
        return None;
    }
    let candidates: Vec<_> = evidence
        .builds
        .read()?
        .iter()
        .filter(|build| format!("{}/{}", build.run.namespace, build.run.name) == link.subject)
        .collect();
    let matched = candidates
        .iter()
        .find(|build| {
            link.evidence.iter().any(|seen| {
                let Some(uid) = seen.object.uid.as_deref().filter(|uid| !uid.is_empty()) else {
                    return false;
                };
                (seen.object.kind == "PipelineRun" && build.run.meta.uid.as_deref() == Some(uid))
                    || (seen.object.kind == "TaskRun"
                        && build
                            .tasks
                            .iter()
                            .any(|task| task.meta.uid.as_deref() == Some(uid)))
            })
        })
        .copied();
    matched.or(match candidates.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

fn capped(history: &History) -> Option<String> {
    history.cap.map(|cap| match (cap, history.total) {
        (Cap::Tasks, _) => format!(
            "Showing {} runs; more TaskRuns weren't read ({cap})",
            history.runs
        ),
        (Cap::Logs, _) => format!(
            "Showing {} runs; more logs weren't read ({cap})",
            history.runs
        ),
        (_, Some(total)) if history.runs < total => {
            format!("Showing {} of {total} runs ({cap})", history.runs)
        }
        _ => format!("Showing {} runs; more weren't read ({cap})", history.runs),
    })
}

pub(super) fn reason(evidence: &Evidence, entry: &str) -> String {
    match &evidence.build_history {
        Err(Error::Off) => format!("Tekton Results isn't set up for {entry}"),
        Err(error) => {
            let why = error.to_string();
            format!(
                "Tekton Results on {entry} {}",
                why.strip_prefix("Tekton Results ").unwrap_or(&why)
            )
        }
        Ok(history) => capped(history).unwrap_or_else(|| {
            if history.runs == 0 {
                "Tekton Results has no run for this commit".into()
            } else {
                format!("Tekton Results on {entry} read {} runs", history.runs)
            }
        }),
    }
}

pub(super) fn decorate(hop: &mut Hop, evidence: &Evidence, link: &Link, entry: &str) {
    if !matches!(link.to, Joined::PipelineRun | Joined::SupplyChain) {
        return;
    }
    let found = build(evidence, link);
    let mut from = match found.map(|build| build.origin) {
        Some(tekton::Origin::Results) => format!("Tekton Results · {entry}"),
        _ => format!("Tekton · {entry}"),
    };
    if found.is_some_and(|build| build.origin == tekton::Origin::Cluster && build.archive.is_some())
    {
        from.push_str(" · also in Tekton Results");
    }
    if let Ok(history) = &evidence.build_history
        && let Some(cap) = capped(history)
    {
        from.push_str(&format!(" · {cap}"));
    }
    let Shows::Hop(detail) = &mut hop.shows else {
        return;
    };
    if let Some(field) = detail
        .fields
        .iter_mut()
        .find(|field| field.label == "Read from")
    {
        field.value = Value::Mono(from);
    }
    if found.is_some_and(|build| build.origin == tekton::Origin::Results) {
        for action in &mut detail.actions {
            if matches!(action.target, Target::Resource { .. }) {
                action.disabled = Some("the cluster no longer holds this run".into());
            }
        }
    } else if found.is_none() && link.to == Joined::PipelineRun {
        let why = reason(evidence, entry);
        hop.detail.push_str(&format!("; {why}"));
        if let Some(link) = &mut detail.link {
            link.why.push_str(&format!("; {why}"));
        }
        if let Some(notice) = &mut detail.notice {
            notice.body.push_str(&format!(" {why}."));
        }
    }
}

pub(super) fn missing(evidence: &Evidence, entry: &str) -> Vec<Missing> {
    let mut lines = Vec::new();
    let gap = |error: &Error| match error {
        Error::Refused { .. } => Gap::Refused,
        Error::NotInstalled | Error::LogsNotServed => Gap::NotServed,
        _ => Gap::Unreadable,
    };
    match &evidence.build_history {
        Err(Error::Off) => {}
        Err(error) => lines.push(Missing::new(
            gap(error),
            format!("{}.", reason(evidence, entry)),
        )),
        Ok(history) => {
            if let Some(cap) = capped(history) {
                lines.push(Missing::new(
                    Gap::Unreadable,
                    format!("Tekton Results on {entry}: {cap}."),
                ));
            }
        }
    }
    for build in evidence
        .builds
        .read()
        .into_iter()
        .flatten()
        .filter(|build| build.origin == tekton::Origin::Results)
    {
        if let Some(logs) = build
            .archive
            .as_ref()
            .and_then(|archive| archive.logs.as_ref())
        {
            let note = match logs {
                Err(error) => Some((gap(error), error.to_string())),
                Ok(listing) => listing
                    .cap
                    .map(|cap| (Gap::Unreadable, format!("more logs weren't read ({cap})"))),
            };
            if let Some((gap, why)) = note {
                let line = Missing::new(gap, format!("Tekton Results on {entry}: {why}."));
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
        }
    }
    lines
}
