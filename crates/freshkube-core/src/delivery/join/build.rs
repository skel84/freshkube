use crate::delivery::digest::{Digest, repository, tag};
use crate::delivery::github::PullRequest;
use crate::delivery::kargo::Freight;
use crate::delivery::source::{Source, cap_note};
use crate::delivery::tekton::{Build, CommitNames, EvidenceResult, WitnessSource};

use super::observe::{
    built_image, chains_signed, change, concluded, freight_side, pull_request, run_commit,
};
use super::*;

pub(super) fn pull_request_links(
    evidence: &Evidence,
    builds: &[Build],
    freight: &[Freight],
) -> Vec<Link> {
    let sha = &evidence.sha;
    let prs = match &evidence.pull_requests {
        None => return Vec::new(),
        Some(read) if read.read().is_some() => read.read().unwrap(),
        Some(other) => {
            return vec![Link::new(
                Hop::PullRequest,
                Hop::Commit,
                "-",
                Key::Sha(sha.clone()),
                Confidence::Unknown,
                other.why_not_read().unwrap_or_default(),
            )];
        }
    };
    if prs.is_empty() {
        return vec![Link::new(
            Hop::PullRequest,
            Hop::Commit,
            "-",
            Key::Sha(sha.clone()),
            Confidence::Unknown,
            "GitHub read, no pull request is associated with this commit",
        )];
    }
    let deployed: Vec<Digest> = builds
        .iter()
        .flat_map(|build| build.images())
        .map(|image| image.digest)
        .collect();
    let mut links = Vec::new();
    for pr in prs {
        let subject = format!("{}#{}", pr.repo, pr.number);
        let merge_is = pr
            .merge_sha
            .as_deref()
            .is_some_and(|merge| merge.eq_ignore_ascii_case(sha));
        let head_is = pr.head_sha.eq_ignore_ascii_case(sha);
        let (confidence, how) = if merge_is {
            (Confidence::Confirmed, "its merge commit")
        } else if head_is {
            (Confidence::Confirmed, "its head commit")
        } else {
            (
                Confidence::Claimed,
                "only by ancestry, as GitHub lists it; neither its head nor its merge commit",
            )
        };
        let by_pr = pull_request(pr);
        let mut on_commit = by_pr.clone();
        on_commit.push(change(sha));
        links.push(
            Link::new(
                Hop::PullRequest,
                Hop::Commit,
                subject.clone(),
                Key::Sha(sha.clone()),
                confidence,
                format!(
                    "the commit is {how}; {}{}",
                    pr.state.as_deref().unwrap_or("state unknown"),
                    if pr.merged { ", merged" } else { "" }
                ),
            )
            .observed(on_commit),
        );
        if merge_is || head_is {
            for build in builds {
                let run = commit_link(sha, build, &evidence.commit_names);
                let numbered = match build.run.pull_request {
                    Some(number) if number != pr.number => {
                        format!("; the run says pull request #{number}, not #{}", pr.number)
                    }
                    _ => String::new(),
                };
                let mut on_run = by_pr.clone();
                on_run.extend(without_change(run.evidence));
                links.push(
                    Link::new(
                        Hop::PullRequest,
                        Hop::PipelineRun,
                        run.subject,
                        Key::Sha(sha.clone()),
                        run.confidence,
                        format!("built from {how}; {}{numbered}", run.reason),
                    )
                    .observed(on_run),
                );
            }
        }
        if head_is {
            continue;
        }
        links.extend(head_build_links(evidence, pr, &subject, &deployed, freight));
    }
    links
}

/// What the pull request's own head commit built, and whether it is what
/// shipped: with a squash merge it usually is not.
fn head_build_links(
    evidence: &Evidence,
    pr: &PullRequest,
    subject: &str,
    deployed: &[Digest],
    freight: &[Freight],
) -> Vec<Link> {
    let head = &pr.head_sha;
    let read = evidence.pr_builds.get(&pr.number);
    let builds = match read {
        Some(source) if source.read().is_some() => source.read().unwrap(),
        Some(other) => {
            return vec![Link::new(
                Hop::PullRequest,
                Hop::PipelineRun,
                subject,
                Key::Sha(head.clone()),
                Confidence::Unknown,
                other.why_not_read().unwrap_or_default(),
            )];
        }
        None => return Vec::new(),
    };
    if builds.is_empty() {
        return vec![Link::new(
            Hop::PullRequest,
            Hop::PipelineRun,
            subject,
            Key::Sha(head.clone()),
            Confidence::Unknown,
            format!(
                "CI/CD cluster read, no PipelineRun for the pull request's head commit{}",
                cap_note(read.and_then(Source::capped))
            ),
        )];
    }
    let mut links = Vec::new();
    for build in builds {
        let run = commit_link(head, build, &evidence.commit_names);
        let head_run = run_commit(build, &evidence.commit_names);
        let digests: Vec<Digest> = build.images().into_iter().map(|i| i.digest).collect();
        let shipped = digests.iter().any(|digest| deployed.contains(digest));
        let note = if digests.is_empty() {
            "it reported no image digest".to_owned()
        } else if shipped {
            "its image digest is the one that shipped".to_owned()
        } else {
            format!(
                "its image ({}) is not the digest that shipped; the merge was rebuilt",
                digests
                    .iter()
                    .map(Digest::short)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let mut on_run = pull_request(pr);
        on_run.extend(without_change(run.evidence));
        links.push(
            Link::new(
                Hop::PullRequest,
                Hop::PipelineRun,
                run.subject,
                Key::Sha(head.clone()),
                run.confidence,
                format!("built from the head commit; {note}; {}", run.reason),
            )
            .observed(on_run),
        );
        // A head build whose digest Kargo holds joins the PR to that Freight
        // on the digest, whatever the merge commit was.
        for item in freight {
            if let Some(digest) = item
                .images
                .iter()
                .filter_map(|image| image.digest.as_ref())
                .find(|digest| digests.contains(digest))
            {
                // The pull request's head commit built the image: GitHub
                // reports the head, the build reports the digest.
                // The head build's own tie to the head commit comes first.
                let mut on_freight = pull_request(pr);
                on_freight.extend(head_run.iter().cloned());
                on_freight.extend(built_image(build, digest));
                on_freight.push(concluded(
                    "the head build's digest == the Freight's",
                    digest.as_str(),
                ));
                on_freight.extend(freight_side(item, &Key::Digest(digest.clone())));
                links.push(
                    Link::new(
                        Hop::PullRequest,
                        Hop::Freight,
                        format!("{}/{}", item.project, item.name),
                        Key::Digest(digest.clone()),
                        Confidence::Confirmed,
                        "Kargo holds the image the pull request's head commit built",
                    )
                    .observed(on_freight),
                );
            }
        }
    }
    links
}

pub(super) fn commit_link(sha: &str, build: &Build, names: &CommitNames) -> Link {
    let subject = id(&build.run.namespace, &build.run.name);
    let outcome = match (&build.run.succeeded, &build.run.reason) {
        (Some(status), Some(reason)) => format!("Succeeded={status} ({reason})"),
        _ => "no Succeeded condition".to_owned(),
    };
    let outcome = match &build.tasks_unread {
        Some(why) => format!("{outcome}; its TaskRuns were not read ({why})"),
        None => outcome,
    };
    let mut evidence = vec![change(sha)];
    evidence.extend(run_commit(build, names));
    let label_is = build
        .run
        .sha
        .as_deref()
        .is_some_and(|label| label.eq_ignore_ascii_case(sha));
    let link = match build.witness(names) {
        Some(witness) if witness.reported() && witness.commit.eq_ignore_ascii_case(sha) => {
            let rule = match witness.source {
                WitnessSource::TaskResult(_) => "a task result == the change",
                _ => "a run result == the change",
            };
            evidence.push(concluded(rule, sha));
            Link::new(
                Hop::Commit,
                Hop::PipelineRun,
                subject,
                Key::Sha(sha.to_owned()),
                Confidence::Confirmed,
                format!("a result of the build reports the commit; {outcome}"),
            )
        }
        Some(witness) if witness.reported() => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            format!(
                "a result of the build reports another commit ({}), not this one; {outcome}",
                short(&witness.commit)
            ),
        ),
        Some(witness) if witness.commit.eq_ignore_ascii_case(sha) => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            if label_is {
                format!(
                    "the PaC label and the run's revision parameter agree, both declared; no result reports the commit; {outcome}"
                )
            } else {
                format!(
                    "only the run's revision parameter says so, declared; no result reports the commit; {outcome}"
                )
            },
        ),
        Some(_) => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            format!("the run's own revision differs from its PaC label; {outcome}"),
        ),
        None => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            format!("only the PaC label says so; {outcome}"),
        ),
    };
    link.observed(evidence)
}

fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// A run's evidence without the commit side, which is the change itself.
fn without_change(evidence: Vec<Observation>) -> Vec<Observation> {
    evidence
        .into_iter()
        .filter(|seen| seen.object.kind != "Commit")
        .collect()
}

pub(super) fn supply_chain_links(
    build: &Build,
    evidence_result: Option<&EvidenceResult>,
    names: &CommitNames,
) -> Vec<Link> {
    let verdict = match build.conforma() {
        Some(conforma) => format!(
            "Conforma {} ({} failures, {} warnings)",
            conforma.outcome, conforma.failures, conforma.warnings
        ),
        None => "no Conforma result".to_owned(),
    };
    let verdict = match &build.tasks_unread {
        Some(why) => {
            format!("its TaskRuns were not read ({why}), so their results are unknown; {verdict}")
        }
        None => verdict,
    };
    let images = build.images();
    let subject = id(&build.run.namespace, &build.run.name);
    let verdict = match evidence_result.map(|config| build.evidence_record(config)) {
        None => format!(
            "{verdict}; no evidence result configured{}",
            run_results(build)
        ),
        Some(None) => format!("{verdict}; the configured evidence result is not on the build"),
        Some(Some(record)) => {
            let same_commit = record.commit.as_deref().is_some_and(|sha| {
                build
                    .witnessed_commit(names)
                    .or_else(|| build.run.sha.clone())
                    .is_some_and(|seen| seen.eq_ignore_ascii_case(sha))
            });
            let same_digest = record
                .image_digest
                .as_ref()
                .is_some_and(|digest| images.iter().any(|image| &image.digest == digest));
            format!(
                "{verdict}; the pipeline's evidence record {} the build's commit and digest",
                if same_commit && same_digest {
                    "agrees with"
                } else {
                    "disagrees with"
                }
            )
        }
    };
    if images.is_empty() {
        return vec![Link::new(
            Hop::PipelineRun,
            Hop::SupplyChain,
            subject,
            Key::None,
            Confidence::Unknown,
            format!("the build reported no image digest; {verdict}"),
        )];
    }
    images
        .into_iter()
        .map(|image| {
            let (confidence, chains) = match build.chains_state().as_deref() {
                Some("true") => (Confidence::Confirmed, "signed by Chains".to_owned()),
                Some(state) => (
                    Confidence::Unknown,
                    format!("Chains reports signed={state}"),
                ),
                None => (Confidence::Unknown, "no Chains annotation".to_owned()),
            };
            // The build reports the digest; Chains' annotation, declared, is
            // all that says it signed it.
            let mut evidence: Vec<Observation> =
                built_image(build, &image.digest).into_iter().collect();
            evidence.extend(chains_signed(build));
            Link::new(
                Hop::PipelineRun,
                Hop::SupplyChain,
                subject.clone(),
                Key::Digest(image.digest.clone()),
                confidence,
                format!(
                    "{} {}; {chains}; {verdict}",
                    repository(&image.url),
                    tag(&image.url)
                        .map(|t| format!("(tag {t})"))
                        .unwrap_or_default()
                ),
            )
            .observed(evidence)
        })
        .collect()
}

/// The names of the run's own results, which a configured evidence result
/// could be one of.
fn run_results(build: &Build) -> String {
    const SHOWN: usize = 8;
    let names: Vec<&str> = build.run.results.keys().map(String::as_str).collect();
    match names.len() {
        0 => " (the run has no results)".to_owned(),
        n if n > SHOWN => format!(
            " (the run's results: {}, and {} more)",
            names[..SHOWN].join(", "),
            n - SHOWN
        ),
        _ => format!(" (the run's results: {})", names.join(", ")),
    }
}
