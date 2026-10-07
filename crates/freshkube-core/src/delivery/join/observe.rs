//! The observations behind each link: what was read, from which object, to
//! say what the link says. A link between two hops stands on two sides, and
//! a confirmed link has, on each side, an observation that is not merely
//! declared and carries the key both sides share.

use crate::delivery::digest::Digest;
use crate::delivery::github::PullRequest;
use crate::delivery::kargo::{Freight, Promotion, Stage};
use crate::delivery::observation::{Fact, Meta, ObjectRef, Observation, pointer_segment, role};
use crate::delivery::pods::RunningImage;
use crate::delivery::tekton::{
    Build, CHAINS_SIGNED, CommitNames, SHA_LABEL, WitnessSource, built_images,
};

use super::Key;

/// Most pod containers one link names. The reason says how many there are.
const MOST_PODS: usize = 5;

/// What a key reads as in an observation's value.
pub(super) fn key_value(key: &Key) -> Option<String> {
    match key {
        Key::Sha(sha) => Some(sha.clone()),
        Key::Digest(digest) => Some(digest.to_string()),
        Key::Name(_) | Key::None => None,
    }
}

/// The change itself, which no cluster holds: the caller named it.
fn change_ref(sha: &str) -> ObjectRef {
    ObjectRef::new("", "Commit", None, sha, &Meta::default())
}

/// The commit side of a link: the join's input, derived from nothing read.
pub(super) fn change(sha: &str) -> Observation {
    Observation::derived(role::CHANGE, change_ref(sha), "/sha", Some(sha))
}

/// An image, known by its digest alone: its repository names a registry.
fn image_ref(digest: &Digest) -> ObjectRef {
    ObjectRef::new("", "Image", None, digest.as_str(), &Meta::default())
}

/// One side of a link: what was `seen`, and, when none of it, from an object
/// of `object`'s API group, is more than declared and carries `key`, the join's own conclusion that this side
/// holds `key`, at `field` of `object`. The pointer says where it was
/// matched.
pub(super) fn side(
    mut seen: Vec<Observation>,
    key: &str,
    cluster: &str,
    object: ObjectRef,
    field: &str,
) -> Vec<Observation> {
    let proves = |seen: &Observation| {
        seen.object.group == object.group
            && seen.fact != Fact::Declared
            && seen
                .value
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(key))
    };
    if !seen.iter().any(proves) {
        seen.push(Observation::derived(cluster, object, field, Some(key)));
    }
    seen
}

// ---- GitHub ----------------------------------------------------------

/// The pull request's side: what GitHub reports of its head commit, and of
/// its merge commit when it has merged.
pub(super) fn pull_request(pr: &PullRequest) -> Vec<Observation> {
    let mut seen = vec![Observation::reported(
        role::GITHUB,
        pr.object_ref(),
        "/head/sha",
        Some(&pr.head_sha),
    )];
    if let Some(merge) = &pr.merge_sha {
        seen.push(Observation::reported(
            role::GITHUB,
            pr.object_ref(),
            "/merge_commit_sha",
            Some(merge),
        ));
    }
    seen
}

/// The pull request's side of a link on `key`.
pub(super) fn pull_request_side(pr: &PullRequest, key: &str) -> Vec<Observation> {
    side(
        pull_request(pr),
        key,
        role::GITHUB,
        pr.object_ref(),
        "/head/sha",
    )
}

// ---- builds ----------------------------------------------------------

fn sha_label_field() -> String {
    format!("/metadata/labels/{}", pointer_segment(SHA_LABEL))
}

/// What a PipelineRun says of the commit: the PaC label, declared, and the
/// run's own witness, a parameter (declared) or a result (reported).
pub(super) fn run_commit(build: &Build, names: &CommitNames) -> Vec<Observation> {
    let run = build.run.object_ref();
    let mut seen = Vec::new();
    if let Some(label) = &build.run.sha {
        seen.push(Observation::declared(
            role::TEKTON,
            run.clone(),
            &sha_label_field(),
            Some(label),
        ));
    }
    if let Some(witness) = build.witness(names) {
        seen.push(match witness.source {
            WitnessSource::Param => {
                Observation::declared(role::TEKTON, run, "/spec/params", Some(&witness.commit))
            }
            WitnessSource::RunResult => {
                Observation::reported(role::TEKTON, run, "/status/results", Some(&witness.commit))
            }
            WitnessSource::TaskResult(index) => Observation::reported(
                role::TEKTON,
                build.tasks[index].object_ref(),
                "/status/results",
                Some(&witness.commit),
            ),
        });
    }
    seen
}

/// The PipelineRun's side of a link on the commit `sha`.
pub(super) fn run_commit_side(build: &Build, names: &CommitNames, sha: &str) -> Vec<Observation> {
    side(
        run_commit(build, names),
        sha,
        role::TEKTON,
        build.run.object_ref(),
        &sha_label_field(),
    )
}

/// What the build's results say of an image it built: reported by the run,
/// or by the task that wrote the result.
pub(super) fn built_image(build: &Build, digest: &Digest) -> Option<Observation> {
    let has = |results| {
        built_images(results)
            .iter()
            .any(|image| &image.digest == digest)
    };
    if has(&build.run.results) {
        return Some(Observation::reported(
            role::TEKTON,
            build.run.object_ref(),
            "/status/results",
            Some(digest.as_str()),
        ));
    }
    build
        .tasks
        .iter()
        .find(|task| has(&task.results))
        .map(|task| {
            Observation::reported(
                role::TEKTON,
                task.object_ref(),
                "/status/results",
                Some(digest.as_str()),
            )
        })
}

/// What Chains wrote on the run or the task it signed: declared, an
/// annotation.
pub(super) fn chains_signed(build: &Build) -> Option<Observation> {
    let field = format!("/metadata/annotations/{}", pointer_segment(CHAINS_SIGNED));
    if let Some(state) = &build.run.chains_state {
        return Some(Observation::declared(
            role::TEKTON,
            build.run.object_ref(),
            &field,
            Some(state),
        ));
    }
    build.tasks.iter().find_map(|task| {
        task.chains_state.as_ref().map(|state| {
            Observation::declared(role::TEKTON, task.object_ref(), &field, Some(state))
        })
    })
}

/// The PipelineRun side of a link on `key`, from the builds that carry it.
/// With no build (the commit joined a Freight alone) the side is the commit.
pub(super) fn builds_side(builds: &[Build], names: &CommitNames, key: &Key) -> Vec<Observation> {
    let Some(value) = key_value(key) else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    match key {
        Key::Digest(digest) => {
            seen.extend(builds.iter().filter_map(|build| built_image(build, digest)));
        }
        Key::Sha(sha) => {
            for build in builds {
                seen.extend(run_commit(build, names).into_iter().filter(|o| {
                    o.value
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(sha))
                }));
            }
        }
        _ => {}
    }
    match builds.first() {
        Some(build) => side(
            seen,
            &value,
            role::TEKTON,
            build.run.object_ref(),
            &sha_label_field(),
        ),
        None => side(seen, &value, role::CHANGE, change_ref(&value), "/sha"),
    }
}

/// The supply-chain side: the image the build reported, and what Chains
/// wrote of it.
pub(super) fn supply_chain_side(build: &Build, digest: &Digest) -> Vec<Observation> {
    let mut seen: Vec<Observation> = chains_signed(build).into_iter().collect();
    seen.push(Observation::derived(
        role::TEKTON,
        image_ref(digest),
        "/digest",
        Some(digest.as_str()),
    ));
    seen
}

// ---- Kargo -----------------------------------------------------------

/// The Freight side of a link on `key`: the digest of an image, or the
/// commit, that the Freight holds. Kargo reports both.
pub(super) fn freight_side(freight: &Freight, key: &Key) -> Vec<Observation> {
    let Some(value) = key_value(key) else {
        return Vec::new();
    };
    let field = match key {
        Key::Digest(_) => "/images",
        _ => "/commits",
    };
    let seen = vec![Observation::reported(
        role::KARGO,
        freight.object_ref(),
        field,
        Some(&value),
    )];
    side(seen, &value, role::KARGO, freight.object_ref(), field)
}

/// What a Promotion's status records of the Freight's image.
pub(super) fn promotion_digest(promotion: &Promotion, digest: &Digest) -> Vec<Observation> {
    vec![Observation::reported(
        role::KARGO,
        promotion.object_ref(),
        "/status",
        Some(digest.as_str()),
    )]
}

/// The Promotion's own naming of the Freight: declared in its spec.
pub(super) fn promotion_names(promotion: &Promotion) -> Vec<Observation> {
    vec![Observation::declared(
        role::KARGO,
        promotion.object_ref(),
        "/spec/freight",
        promotion.freight.as_deref(),
    )]
}

/// What a Stage's status records of its current Freight's image.
pub(super) fn stage_digest(stage: &Stage, digest: &Digest) -> Vec<Observation> {
    vec![Observation::reported(
        role::KARGO,
        stage.object_ref(),
        "/status",
        Some(digest.as_str()),
    )]
}

/// The Stage's own naming of a Freight as current: reported in its status.
pub(super) fn stage_names(stage: &Stage, freight: &str) -> Vec<Observation> {
    vec![Observation::reported(
        role::KARGO,
        stage.object_ref(),
        "/status",
        Some(freight),
    )]
}

/// The Stage side of the Promotion-to-Application link: the commit a
/// succeeded Promotion to the Stage pushed, as its status records it, and
/// the join's reading of it as the Stage's.
pub(super) fn stage_pushed(
    stage: &Stage,
    promotions: &[&Promotion],
    commit: &str,
) -> Vec<Observation> {
    let mut seen: Vec<Observation> = promotions
        .iter()
        .filter(|promotion| {
            promotion
                .pushed_commits
                .iter()
                .any(|pushed| pushed.eq_ignore_ascii_case(commit))
        })
        .map(|promotion| {
            Observation::reported(
                role::KARGO,
                promotion.object_ref(),
                "/status/state",
                Some(commit),
            )
        })
        .collect();
    seen.extend(side(
        Vec::new(),
        commit,
        role::KARGO,
        stage.object_ref(),
        "/status",
    ));
    seen
}

// ---- Argo CD and the environment ------------------------------------

/// The Application's claim to a Stage: declared, an annotation or a name.
pub(super) fn application_claim(
    app: &crate::delivery::argocd::Application,
    stage: &Stage,
    annotated: bool,
) -> Observation {
    if annotated {
        Observation::declared(
            role::ARGOCD,
            app.object_ref(),
            &format!(
                "/metadata/annotations/{}",
                pointer_segment(crate::delivery::argocd::AUTHORIZED_STAGE)
            ),
            Some(&format!("{}:{}", stage.project, stage.name)),
        )
    } else {
        Observation::declared(
            role::ARGOCD,
            app.object_ref(),
            "/metadata/name",
            Some(&app.name),
        )
    }
}

/// The revision Argo CD reports it synced.
pub(super) fn application_synced(
    app: &crate::delivery::argocd::Application,
    commit: &str,
) -> Observation {
    Observation::reported(role::ARGOCD, app.object_ref(), "/status/sync", Some(commit))
}

/// The pods' side of a link: the containers that run `digest`, as the pods
/// report it, the first few of them.
pub(super) fn pods_running(pods: &[(&RunningImage, &Digest)]) -> Vec<Observation> {
    pods.iter()
        .take(MOST_PODS)
        .map(|(pod, digest)| {
            Observation::reported(
                role::ENVIRONMENT,
                pod.object_ref(),
                "/status/containerStatuses",
                Some(digest.as_str()),
            )
        })
        .collect()
}

/// The pods that run another image of the Freight's repository, with the
/// digest each reports.
pub(super) fn pods_running_other(pods: &[(&RunningImage, Option<Digest>)]) -> Vec<Observation> {
    pods.iter()
        .take(MOST_PODS)
        .map(|(pod, digest)| {
            Observation::reported(
                role::ENVIRONMENT,
                pod.object_ref(),
                "/status/containerStatuses",
                digest.as_ref().map(Digest::as_str),
            )
        })
        .collect()
}
