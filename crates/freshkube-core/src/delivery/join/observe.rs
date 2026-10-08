//! The observations behind each link: what was read, from which object, to
//! say what the link says. Each observation is of something read, with the
//! field it was read from. A conclusion of the join's own is a [`concluded`]
//! observation attributed to the join, never to a field it didn't read.
//!
//! A link between two hops stands on two sides. Where a side's only
//! evidence is declared (a label, a spec field), the link shows just that;
//! nothing here makes up an observation to fill the gap.

use crate::delivery::argocd::{AUTHORIZED_STAGE, Application};
use crate::delivery::digest::Digest;
use crate::delivery::github::PullRequest;
use crate::delivery::kargo::{Freight, Promotion, Stage};
use crate::delivery::observation::{Meta, ObjectRef, Observation, pointer_segment, role};
use crate::delivery::pods::RunningImage;
use crate::delivery::rollouts::{POD_HASH_LABEL, ReplicaSet, Rollout};
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

/// The commit side of a link: the change itself, which no cluster holds and
/// the caller named.
pub(super) fn change(sha: &str) -> Observation {
    Observation::derived(
        role::CHANGE,
        ObjectRef::new("", "Commit", None, sha, &Meta::default()),
        "/sha",
        Some(sha),
    )
}

/// What the join concluded from the observations beside it, named by the
/// rule it applied (such as "label == witness"), with the key it found.
pub(super) fn concluded(rule: &str, key: &str) -> Observation {
    Observation::derived(
        role::JOIN,
        ObjectRef::new("", "Join", None, rule, &Meta::default()),
        "/key",
        Some(key),
    )
}

// ---- GitHub ----------------------------------------------------------

/// What GitHub reports of the pull request's head commit, and of its merge
/// commit once it has merged.
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

// ---- builds ----------------------------------------------------------

fn sha_label_field() -> String {
    format!("/metadata/labels/{}", pointer_segment(SHA_LABEL))
}

/// What a PipelineRun says of the commit: the PaC label, declared, and the
/// run's own witness, a parameter (declared) or a result (reported).
pub(super) fn run_commit(build: &Build, names: &CommitNames, sha: &str) -> Vec<Observation> {
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
    if let Some(witness) = build.witness(names, sha) {
        let commit = Some(witness.commit.as_str());
        seen.push(match witness.source {
            WitnessSource::Param => {
                Observation::declared(role::TEKTON, run, "/spec/params", commit)
            }
            WitnessSource::RunResult => {
                Observation::reported(role::TEKTON, run, "/status/results", commit)
            }
            WitnessSource::TaskResult(index) => Observation::reported(
                role::TEKTON,
                build.tasks[index].object_ref(),
                "/status/results",
                commit,
            ),
        });
    }
    seen
}

/// What the build's results say of an image it built: reported by the run,
/// or by the task that wrote the result.
pub(super) fn built_image(build: &Build, digest: &Digest) -> Option<Observation> {
    let has = |results| {
        built_images(results)
            .iter()
            .any(|image| &image.digest == digest)
    };
    let value = Some(digest.as_str());
    if has(&build.run.results) {
        return Some(Observation::reported(
            role::TEKTON,
            build.run.object_ref(),
            "/status/results",
            value,
        ));
    }
    build
        .tasks
        .iter()
        .find(|task| has(&task.results))
        .map(|task| {
            Observation::reported(role::TEKTON, task.object_ref(), "/status/results", value)
        })
}

/// What Chains wrote on the run or the task it signed: an annotation, so
/// declared.
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

/// The PipelineRun side of a link on `key`: what the builds that carry it
/// report. Empty when none does (a commit that joined a Freight alone).
pub(super) fn builds_side(builds: &[Build], names: &CommitNames, key: &Key) -> Vec<Observation> {
    match key {
        Key::Digest(digest) => builds
            .iter()
            .filter_map(|build| built_image(build, digest))
            .collect(),
        Key::Sha(sha) => builds
            .iter()
            .flat_map(|build| run_commit(build, names, sha))
            .filter(|seen| {
                seen.value
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(sha))
            })
            .collect(),
        _ => Vec::new(),
    }
}

// ---- Kargo -----------------------------------------------------------

/// The Freight side of a link on `key`: the digest or the commit that the
/// Freight holds, as Kargo wrote it.
pub(super) fn freight_side(freight: &Freight, key: &Key) -> Vec<Observation> {
    let Some(value) = key_value(key) else {
        return Vec::new();
    };
    let field = match key {
        Key::Digest(_) => freight.pointer("images"),
        _ => freight.pointer("commits"),
    };
    vec![Observation::reported(
        role::KARGO,
        freight.object_ref(),
        &field,
        Some(&value),
    )]
}

/// What a Promotion's status records of the Freight's image.
pub(super) fn promotion_digest(promotion: &Promotion, digest: &Digest) -> Vec<Observation> {
    vec![Observation::reported(
        role::KARGO,
        promotion.object_ref(),
        promotion.digest_pointer(digest),
        Some(digest.as_str()),
    )]
}

/// The Promotion's naming of the Freight: declared in its spec, else
/// reported in its status.
pub(super) fn promotion_names(promotion: &Promotion) -> Vec<Observation> {
    let name = promotion.freight.as_deref();
    vec![if promotion.freight_declared {
        Observation::declared(role::KARGO, promotion.object_ref(), "/spec/freight", name)
    } else {
        Observation::reported(
            role::KARGO,
            promotion.object_ref(),
            "/status/freight/name",
            name,
        )
    }]
}

/// What a Stage's status records of its current Freight's image.
pub(super) fn stage_digest(stage: &Stage, digest: &Digest) -> Vec<Observation> {
    vec![Observation::reported(
        role::KARGO,
        stage.object_ref(),
        stage.digest_pointer(digest),
        Some(digest.as_str()),
    )]
}

/// The Stage's naming of a Freight as current: reported in its status.
pub(super) fn stage_names(stage: &Stage, freight: &str) -> Vec<Observation> {
    vec![Observation::reported(
        role::KARGO,
        stage.object_ref(),
        stage.freight_pointer(freight),
        Some(freight),
    )]
}

// ---- Argo CD and the environment ------------------------------------

/// The Application's claim to a Stage: declared, an annotation or a name.
pub(super) fn application_claim(app: &Application, stage: &Stage, annotated: bool) -> Observation {
    if annotated {
        let field = format!(
            "/metadata/annotations/{}",
            pointer_segment(AUTHORIZED_STAGE)
        );
        let claim = format!("{}:{}", stage.project, stage.name);
        Observation::declared(role::ARGOCD, app.object_ref(), &field, Some(&claim))
    } else {
        Observation::declared(
            role::ARGOCD,
            app.object_ref(),
            "/metadata/name",
            Some(&app.name),
        )
    }
}

/// The Stage-to-Application link on a pushed commit: the Promotions that
/// recorded the push, the revision Argo CD reports it synced, and the join's
/// match of the two.
pub(super) fn pushed_and_synced(
    promotions: &[&Promotion],
    app: &Application,
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
    seen.extend(application_synced(app, commit));
    seen.push(concluded("pushed commit == synced revision", commit));
    seen
}

/// A revision Argo CD reports the Application synced, if it holds `commit`.
pub(super) fn application_synced(app: &Application, commit: &str) -> Option<Observation> {
    let at = app
        .sync_revisions
        .iter()
        .position(|revision| revision.eq_ignore_ascii_case(commit))?;
    Some(Observation::reported(
        role::ARGOCD,
        app.object_ref(),
        app.sync_revisions_at[at],
        Some(commit),
    ))
}

/// The Application's first synced revision, whatever it is.
pub(super) fn application_revision(app: &Application) -> Option<Observation> {
    Some(Observation::reported(
        role::ARGOCD,
        app.object_ref(),
        app.sync_revisions_at.first()?,
        app.sync_revisions.first().map(String::as_str),
    ))
}

/// The Application's side of a link to a Rollout it manages: Argo CD lists
/// it among the Application's resources.
pub(super) fn manages(app: &Application, rollout: &Rollout) -> Observation {
    Observation::reported(
        role::ARGOCD,
        app.object_ref(),
        "/status/resources",
        Some(&format!("Rollout/{}", rollout.name)),
    )
}

/// What the Rollout reports of itself, and its spec's pin of `digest` when
/// the join found one.
pub(super) fn rollout_state(rollout: &Rollout, pinned: Option<&Digest>) -> Vec<Observation> {
    let mut seen = vec![Observation::reported(
        role::ENVIRONMENT,
        rollout.object_ref(),
        "/status/phase",
        rollout.phase.as_deref(),
    )];
    if let Some(digest) = pinned {
        seen.push(Observation::declared(
            role::ENVIRONMENT,
            rollout.object_ref(),
            "/spec/template/spec/containers",
            Some(digest.as_str()),
        ));
        seen.push(concluded(
            "Rollout spec pins the Freight's digest",
            digest.as_str(),
        ));
    }
    seen
}

/// The pod-template hash the Rollout reports as current.
pub(super) fn rollout_hash(rollout: &Rollout) -> Vec<Observation> {
    rollout
        .current_pod_hash
        .as_deref()
        .map(|hash| {
            Observation::reported(
                role::ENVIRONMENT,
                rollout.object_ref(),
                "/status/currentPodHash",
                Some(hash),
            )
        })
        .into_iter()
        .collect()
}

/// What ties a Rollout to the pods of its current revision, every step
/// reported by a controller: the ReplicaSet whose owner reference names the
/// Rollout's UID and whose hash label is the current one, and each pod whose
/// owner reference names that ReplicaSet's UID and whose label carries the
/// hash too. The first few pods of `pods`, as [`pods_running`] names them.
pub(super) fn revision_tie(
    rollout: &Rollout,
    set: &ReplicaSet,
    pods: &[(&RunningImage, &Digest)],
) -> Vec<Observation> {
    let label = format!("/metadata/labels/{}", pointer_segment(POD_HASH_LABEL));
    let Some(hash) = rollout.current_pod_hash.as_deref() else {
        return Vec::new();
    };
    let mut seen = vec![
        Observation::reported(
            role::ENVIRONMENT,
            set.object_ref(),
            "/metadata/ownerReferences",
            set.owner_uid.as_deref(),
        ),
        Observation::reported(role::ENVIRONMENT, set.object_ref(), &label, Some(hash)),
    ];
    for (pod, _) in pods.iter().take(MOST_PODS) {
        seen.push(Observation::reported(
            role::ENVIRONMENT,
            pod.object_ref(),
            "/metadata/ownerReferences",
            pod.owner_uid.as_deref(),
        ));
        seen.push(Observation::reported(
            role::ENVIRONMENT,
            pod.object_ref(),
            &label,
            pod.pod_hash.as_deref(),
        ));
    }
    seen.push(concluded(
        "pod -> ReplicaSet -> Rollout by owner UID; pod-template-hash == currentPodHash",
        hash,
    ));
    seen
}

/// What Argo CD's image summary lists of `digests`.
pub(super) fn summary_images(app: &Application, digests: &[Digest]) -> Vec<Observation> {
    digests
        .iter()
        .map(|digest| {
            Observation::reported(
                role::ARGOCD,
                app.object_ref(),
                "/status/summary/images",
                Some(digest.as_str()),
            )
        })
        .collect()
}

/// The pods' side of a link: the containers that run a digest, as the pods
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
