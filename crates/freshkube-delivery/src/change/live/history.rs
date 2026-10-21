//! Where a build was read, kept separate from what its manifest reports.
use crate::change::{Gap, Hop, Missing, Shows, Target, Value};
use crate::join::{Evidence, Hop as Joined, Link};
use crate::results::{Cap, Error, History, Skip};
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

fn runs(count: usize) -> String {
    format!("{count} {}", if count == 1 { "run" } else { "runs" })
}

fn capped(history: &History) -> Option<String> {
    let read = runs(history.runs);
    history.cap.map(|cap| match (cap, history.total) {
        (Cap::Tasks, _) => format!("Showing {read}; more TaskRuns weren't read ({cap})"),
        (_, Some(total)) if history.runs < total => {
            format!("Showing {} of {total} runs ({cap})", history.runs)
        }
        _ => format!("Showing {read}; more weren't read ({cap})"),
    })
}

pub(super) fn reason(evidence: &Evidence, entry: &str) -> Option<String> {
    let prefix = format!("Tekton Results on {entry}");
    Some(match &evidence.build_history {
        Err(Error::Off | Error::Skipped(Skip::LiveFailed)) => return None,
        Err(Error::Skipped(why)) => format!("{prefix} wasn't read: {why}"),
        Err(Error::NotForwarded) => format!("{prefix} isn't forwarded yet"),
        Err(Error::ForwardEnded(why)) => format!("the Tekton Results forward on {entry} ended: {why}"),
        Err(Error::NotInstalled) => format!("{prefix} isn't installed or served"),
        Err(Error::LogsNotServed) => format!("{prefix} doesn't serve logs"),
        Err(Error::LogMissing) => format!("{prefix} kept no log for this task"),
        Err(Error::Unreachable(cause)) => format!("{prefix} didn't answer: {cause}"),
        Err(Error::Refused { status, token_set }) => {
            let mut why = format!("{prefix} refused the request ({status})");
            if !token_set {
                why.push_str("; no token is set for this address");
            }
            why
        }
        Err(Error::Http(status)) => format!("{prefix} returned HTTP {status}"),
        Err(Error::Unreadable(why)) => format!("{prefix} could not be read: {why}"),
        Ok(history) => capped(history).unwrap_or_else(|| {
            if history.runs == 0 && evidence.sha.is_empty() {
                format!("{prefix} has no run to search for the image digest")
            } else if history.runs == 0 {
                "Tekton Results has no run for this commit".into()
            } else if evidence.sha.is_empty() {
                format!(
                    "{prefix} was searched for the image digest in the first {} it returned, so a build outside them isn't found",
                    runs(history.runs)
                )
            } else {
                format!("{prefix} read {}", runs(history.runs))
            }
        }),
    })
}

pub(super) fn decorate(hop: &mut Hop, evidence: &Evidence, link: &Link, entry: &str) {
    if matches!(
        evidence.build_history,
        Err(Error::Off | Error::Skipped(Skip::LiveFailed))
    ) || !matches!(link.to, Joined::PipelineRun | Joined::SupplyChain)
    {
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
    } else if found.is_none()
        && link.to == Joined::PipelineRun
        && let Some(why) = reason(evidence, entry)
    {
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
        Err(error) => {
            if let Some(why) = reason(evidence, entry) {
                lines.push(Missing::new(gap(error), format!("{why}.")));
            }
        }
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
