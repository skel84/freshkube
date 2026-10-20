use crate::chains::{Attestation, Form};
use crate::digest::{Digest, tag, written_repository};
use crate::github::PullRequest;
use crate::kargo::Freight;
use crate::observation::ObjectRef;
use crate::source::{Source, cap_note};
use crate::tekton::{Build, CommitNames, EvidenceResult, WitnessSource};
use crate::verify::{NotRead, Trust, Verification, Verified, verify};

use super::argo::short_commit;
use super::observe::{
    attested_commit, attested_digest, built_image, chains_signed, change, concluded, freight_side,
    pull_request, run_commit,
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
                let run = commit_link(sha, build, &evidence.commit_names, &evidence.trust);
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
        let run = commit_link(head, build, &evidence.commit_names, &evidence.trust);
        let run_confidence = run.confidence;
        let head_run = run_commit(build, &evidence.commit_names, head);
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
                        run_confidence,
                        if run_confidence == Confidence::Confirmed {
                            "Kargo holds the image the pull request's head commit built".to_owned()
                        } else {
                            "Kargo holds the image the pull request's head build made; no result of the build reports the head commit, so its tie to it is declared".to_owned()
                        },
                    )
                    .observed(on_freight),
                );
            }
        }
    }
    links
}

pub(super) fn commit_link(sha: &str, build: &Build, names: &CommitNames, trust: &Trust) -> Link {
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
    evidence.extend(run_commit(build, names, sha));
    let label_is = build
        .run
        .sha
        .as_deref()
        .is_some_and(|label| label.eq_ignore_ascii_case(sha));
    let others: Vec<String> = {
        let mut seen = Vec::new();
        for witness in build.reported_witnesses(names) {
            let commit = short_commit(&witness.commit);
            if !seen.contains(&commit) {
                seen.push(commit);
            }
        }
        seen
    };
    let reported_other = !others.is_empty();
    let attested = verified_attestations(build, trust)
        .into_iter()
        .find(|attested| {
            attested
                .verified
                .commit
                .as_deref()
                .is_some_and(|commit| commit.eq_ignore_ascii_case(sha))
        });
    let link = match build.witness(names, sha) {
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
        // A provenance whose signature verifies is a witness the cluster's
        // annotations cannot forge: reported, like a result.
        _ if attested.is_some() && !reported_other => {
            let attested = attested.expect("checked by the guard");
            evidence.push(attested_commit(&attested.object, &attested.verified));
            evidence.push(concluded("a verified provenance == the change", sha));
            Link::new(
                Hop::Commit,
                Hop::PipelineRun,
                subject,
                Key::Sha(sha.to_owned()),
                Confidence::Confirmed,
                format!(
                    "a provenance of the build names the commit, and its signature verifies with key {}; {outcome}",
                    attested.verified.key
                ),
            )
        }
        Some(_) if reported_other => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            format!(
                "the build's results report other commits ({}), not this one; {outcome}",
                others.join(", ")
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
    trust: &Trust,
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
                    .witnessed_commit(names, sha)
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
            let standing = standing(build, trust, &image.digest);
            // The build reports the digest; what else is said about it is
            // the attestation's, reported only once its signature verified,
            // and Chains' annotation, declared.
            let mut evidence: Vec<Observation> =
                built_image(build, &image.digest).into_iter().collect();
            evidence.extend(standing.evidence);
            evidence.extend(chains_signed(build));
            Link::new(
                Hop::PipelineRun,
                Hop::SupplyChain,
                subject.clone(),
                Key::Digest(image.digest.clone()),
                standing.confidence,
                format!(
                    "{} {}; {}; {verdict}",
                    written_repository(&image.url),
                    tag(&image.url)
                        .map(|t| format!("(tag {t})"))
                        .unwrap_or_default(),
                    standing.words
                ),
            )
            .observed(evidence)
            .warned(standing.warn)
        })
        .collect()
}

/// What the supply-chain link says of one image, and what it stands on.
struct Standing {
    confidence: Confidence,
    words: String,
    evidence: Vec<Observation>,
    /// The link's signature is marked failing, see [`Link::signature_warn`].
    warn: bool,
}

/// One attestation of the build and what the verifier made of it.
struct Judged {
    object: ObjectRef,
    attestation: Attestation,
    outcome: Verification,
}

/// A verified attestation of the build.
struct Attested {
    object: ObjectRef,
    verified: Verified,
}

/// Every attestation of the run and its tasks, judged against `digest`. An
/// image's signature is not an attestation of the run and is left out.
fn judge(build: &Build, trust: &Trust, digest: &Digest) -> Vec<Judged> {
    build
        .attestations()
        .into_iter()
        .filter(|(_, attestation)| attestation.form == Form::Run)
        .map(|(object, attestation)| Judged {
            outcome: verify(trust, attestation, digest),
            attestation: attestation.clone(),
            object,
        })
        .collect()
}

/// The attestations of `build` that verify with the person's key for one of
/// the images the build reported.
fn verified_attestations(build: &Build, trust: &Trust) -> Vec<Attested> {
    let images = build.images();
    build
        .attestations()
        .into_iter()
        .filter(|(_, attestation)| attestation.form == Form::Run)
        .filter_map(|(object, attestation)| {
            images
                .iter()
                .find_map(|image| match verify(trust, attestation, &image.digest) {
                    Verification::Verified(verified) => Some(Attested {
                        object: attestation.verified_object(&object),
                        verified,
                    }),
                    Verification::NoTrust
                    | Verification::KeyUnusable(_)
                    | Verification::Malformed(_)
                    | Verification::SignatureFailed { .. }
                    | Verification::SubjectDiffers { .. }
                    | Verification::NotStoredHere(_)
                    | Verification::NotRead(_) => None,
                })
        })
        .collect()
}

/// How much a verdict says, for choosing which of a build's attestations the
/// link reports: a difference or a failure is more than a missing key.
fn weight(outcome: &Verification) -> u8 {
    match outcome {
        Verification::Verified(_) => 7,
        Verification::SubjectDiffers { .. } => 6,
        Verification::SignatureFailed { .. } => 5,
        Verification::Malformed(_) => 4,
        Verification::KeyUnusable(_) => 3,
        Verification::NotStoredHere(_) => 2,
        Verification::NoTrust => 1,
        Verification::NotRead(NotRead::ImageSignature)
        | Verification::NotRead(NotRead::Unread)
        | Verification::NotRead(NotRead::SubjectsCutAtCap) => 0,
    }
}

/// The confidence a verdict gives the link. The one arm that confirms is
/// `Verified`; a new variant does not compile until it is placed here. An
/// attestation that exists but was not verified is a claim.
fn confidence_of(outcome: &Verification) -> Confidence {
    match outcome {
        Verification::Verified(_) => Confidence::Confirmed,
        Verification::NoTrust
        | Verification::KeyUnusable(_)
        | Verification::Malformed(_)
        | Verification::SignatureFailed { .. }
        | Verification::SubjectDiffers { .. }
        | Verification::NotStoredHere(_)
        | Verification::NotRead(_) => Confidence::Claimed,
    }
}

/// Chains' annotation in words.
fn annotation_words(build: &Build) -> (bool, String) {
    match build.chains_state().as_deref() {
        Some("true") => (true, "Chains' annotation says signed".to_owned()),
        Some(state) => (false, format!("Chains reports signed={state}")),
        None => (false, "no Chains annotation".to_owned()),
    }
}

fn standing(build: &Build, trust: &Trust, digest: &Digest) -> Standing {
    let (annotated, annotation) = annotation_words(build);
    let judged = judge(build, trust, digest);
    let chosen = judged
        .iter()
        .find(|judged| judged.outcome.confirms())
        .or_else(|| {
            judged
                .iter()
                .min_by_key(|judged| std::cmp::Reverse(weight(&judged.outcome)))
        });
    let Some(chosen) = chosen else {
        // Nothing of the run's own was stored where it is read. That is not
        // "unsigned": only Chains' annotation, declared, says anything.
        let (confidence, words) = match (annotated, trust) {
            (true, Trust::Unset) => (
                Confidence::Claimed,
                format!("{annotation}; no public key is set, so nothing was verified"),
            ),
            (true, Trust::Unusable(problem)) => (
                Confidence::Claimed,
                format!("{annotation}; the public key can't be used: {problem}"),
            ),
            (true, Trust::Key(_)) => (
                Confidence::Claimed,
                format!("{annotation}; no attestation was found to verify"),
            ),
            (false, _) => (Confidence::Unknown, annotation),
        };
        return Standing {
            confidence,
            words,
            evidence: Vec::new(),
            warn: annotated && matches!(trust, Trust::Unusable(_)),
        };
    };
    let confidence = confidence_of(&chosen.outcome);
    let (words, evidence) = match &chosen.outcome {
        Verification::Verified(verified) => {
            let object = chosen.attestation.verified_object(&chosen.object);
            (
                format!("{}; {annotation}", chosen.outcome),
                vec![attested_digest(&object, &verified.subject)],
            )
        }
        Verification::NoTrust
        | Verification::KeyUnusable(_)
        | Verification::Malformed(_)
        | Verification::SignatureFailed { .. }
        | Verification::SubjectDiffers { .. }
        | Verification::NotStoredHere(_)
        | Verification::NotRead(_) => (
            format!("{}; {annotation}", chosen.outcome),
            // Declared, however it was signed: whoever can patch the run
            // can write the annotation.
            vec![chosen.attestation.observation(chosen.object.clone())],
        ),
    };
    Standing {
        confidence,
        words,
        evidence,
        warn: warns(&chosen.outcome),
    }
}

/// Whether a verdict marks the signature failing: the four rows of the
/// design's table that carry the Warn status. A new variant does not compile
/// until it is placed here.
fn warns(outcome: &Verification) -> bool {
    match outcome {
        Verification::KeyUnusable(_)
        | Verification::SignatureFailed { .. }
        | Verification::Malformed(_)
        | Verification::SubjectDiffers { .. } => true,
        Verification::Verified(_)
        | Verification::NoTrust
        | Verification::NotStoredHere(_)
        | Verification::NotRead(_) => false,
    }
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
