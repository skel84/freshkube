//! The Deployment controller chain: an Application's Deployment, the
//! ReplicaSet the controller reports as its current one, and that
//! ReplicaSet's pods. It mirrors the Rollout chain in `workload.rs`: the
//! spec's pin is declared, and only pods, reached through owner UIDs,
//! report a digest.

use crate::delivery::argocd::Application;
use crate::delivery::deployments::{Deployment, DeploymentSet, current_set};
use crate::delivery::digest::{Digest, repository};
use crate::delivery::kargo::Freight;
use crate::delivery::pods::RunningImage;
use crate::delivery::source::cap_note;

use super::argo::{not_the_environment, rollout_namespace};
use super::observe::{concluded, deployment_revision, deployment_state, deployment_tie, manages};
use super::workload::{Controller, judge_revision, pinned_containers, pinned_link, running};
use super::*;

pub(super) fn deployment_links(
    evidence: &Evidence,
    freight: &Freight,
    app: &Application,
) -> Vec<Link> {
    let app_id = id(&app.namespace, &app.name);
    let managed: Vec<_> = app
        .managed
        .iter()
        .filter(|m| m.group == "apps" && m.kind == "Deployment")
        .collect();
    let unknown = |subject: String, why: String| {
        Link::new(
            Hop::Application,
            Hop::Deployment,
            subject,
            Key::None,
            Confidence::Unknown,
            why,
        )
    };
    if !evidence.deploys_to_environment(&app_id) {
        return vec![unknown(
            app_id.clone(),
            not_the_environment(evidence, &app_id),
        )];
    }
    let Some(deployments) = evidence.deployments.read() else {
        return vec![unknown(
            app_id,
            evidence.deployments.why_not_read().unwrap_or_default(),
        )];
    };
    let mut links = Vec::new();
    let mut elsewhere: Vec<&str> = Vec::new();
    for object in managed {
        let Some(namespace) = rollout_namespace(app, object) else {
            links.push(unknown(
                format!("{app_id}: {}", object.name),
                "neither the Deployment nor the Application's destination names a namespace, so it was not read".into(),
            ));
            continue;
        };
        let deployment_id = id(&namespace, &object.name);
        let Some(deployment) = deployments
            .iter()
            .find(|d| d.namespace == namespace && d.name == object.name)
        else {
            links.push(unknown(
                deployment_id,
                format!(
                    "the Application lists it but it was not found in the environment cluster{}",
                    cap_note(evidence.deployments.capped())
                ),
            ));
            continue;
        };
        // A Deployment that pins another repository is not part of this hop.
        if !pins_repository_of(deployment, freight) {
            elsewhere.push(deployment.name.as_str());
            continue;
        }
        let pinned = deployment
            .images
            .iter()
            .filter_map(|image| Digest::from_reference(image))
            .find(|digest| {
                freight
                    .images
                    .iter()
                    .any(|image| image.digest.as_ref() == Some(digest))
            });
        let state = deployment_summary(deployment);
        let mut seen = vec![manages(app, "Deployment", &deployment.name)];
        seen.extend(deployment_state(deployment, pinned.as_ref()));
        if let Some(digest) = &pinned {
            seen.extend(super::observe::freight_side(
                freight,
                &Key::Digest(digest.clone()),
            ));
        }
        let pods = pod_link(evidence, freight, deployment, &deployment_id);
        links.push(match pinned {
            Some(digest) => pinned_link(
                app,
                Controller {
                    kind: "Deployment",
                    hop: Hop::Deployment,
                    images: &deployment.images,
                },
                deployment_id.clone(),
                digest,
                &pods,
                seen,
                &state,
            ),
            None => Link::new(
                Hop::Application,
                Hop::Deployment,
                deployment_id.clone(),
                Key::Name(deployment.name.clone()),
                Confidence::Claimed,
                format!("managed by the Application; its spec does not pin the digest; {state}"),
            )
            .observed(seen),
        });
        links.push(pods);
    }
    if links.is_empty() {
        links.push(unknown(
            app_id,
            format!(
                "no Deployment the Application manages pins an image of the Freight's repository (read {})",
                elsewhere.join(", ")
            ),
        ));
    }
    links
}

/// Whether the Deployment's pod template names an image of one of the
/// Freight's repositories.
fn pins_repository_of(deployment: &Deployment, freight: &Freight) -> bool {
    deployment.images.iter().any(|image| {
        freight
            .images
            .iter()
            .any(|wanted| repository(&wanted.repo_url) == repository(image))
    })
}

/// What the Deployment controller reports of the Deployment, for a reason.
fn deployment_summary(deployment: &Deployment) -> String {
    let generation = match (deployment.observed_latest(), deployment.observed_generation) {
        (Some(true), _) => "generation observed".to_owned(),
        (Some(false), Some(observed)) => format!(
            "generation not observed yet (observedGeneration {observed}, generation {})",
            deployment.generation.unwrap_or_default()
        ),
        _ => "generation unreported".to_owned(),
    };
    let mut parts = vec![
        generation,
        format!(
            "{} of {} replicas updated, {} available",
            deployment.updated_replicas, deployment.replicas, deployment.available_replicas
        ),
    ];
    if deployment.paused {
        parts.push("paused".into());
    }
    parts.join(", ")
}

/// The pods of the Deployment's current ReplicaSet, judged by digest. The
/// ReplicaSet is the one the controller reports as current by revision, and
/// owns by UID; only its pods count.
fn pod_link(
    evidence: &Evidence,
    freight: &Freight,
    deployment: &Deployment,
    deployment_id: &str,
) -> Link {
    let subject = deployment_id.to_owned();
    let unknown = |why: String| {
        Link::new(
            Hop::Deployment,
            Hop::Pod,
            subject.clone(),
            Key::None,
            Confidence::Unknown,
            why,
        )
    };
    let sets = evidence.deployment_sets.read().map(Vec::as_slice);
    let tie = match sets {
        Some(sets) => current_set(deployment, sets)
            .map_err(|why| format!("{why}{}", cap_note(evidence.deployment_sets.capped()))),
        None => Err(format!(
            "its ReplicaSets were not read ({})",
            evidence.deployment_sets.why_not_read().unwrap_or_default()
        )),
    };
    let Some(read) = evidence.deployment_pods.get(deployment_id) else {
        return unknown(match &tie {
            Err(why) => format!("{why}, so its current pods were not read"),
            Ok(_) => "pods were not read".into(),
        });
    };
    let Some(pods) = read.pods.read() else {
        return unknown(read.pods.why_not_read().unwrap_or_default());
    };
    // Tied by owner UIDs: a pod that carries the hash label but another
    // owner is not the Deployment's.
    let tied: Vec<&RunningImage> = pods
        .iter()
        .filter(|pod| match &tie {
            Ok(set) => {
                set.meta.uid.is_some()
                    && pod.owner_uid == set.meta.uid
                    && pod.template_hash == set.pod_hash
            }
            Err(_) => true,
        })
        .collect();
    let containers = pinned_containers(&deployment.containers, freight, &tied);
    let mut link = judge_revision(
        Hop::Deployment,
        subject,
        &tied,
        &containers,
        read.pods.capped(),
        freight,
    );
    let which = which_pods(&read.hash, deployment, &tie, pods.len() - tied.len());
    let unobserved = deployment.observed_latest() != Some(true);
    match &tie {
        Ok(set) => {
            if link.confidence == Confidence::Confirmed {
                if unobserved {
                    link.confidence = Confidence::Claimed;
                    link.key = Key::None;
                    link.reason = format!(
                        "{}; the Deployment controller has not reported acting on the latest spec ({}), so its status doesn't describe the pods that spec asks for",
                        link.reason,
                        deployment_summary(deployment)
                    );
                } else {
                    let mut seen = deployment_tie(deployment, set, &running(&containers, freight));
                    seen.append(&mut link.evidence);
                    link.evidence = seen;
                }
            }
            link.reason = format!("{}; {which}", link.reason);
        }
        Err(why) => {
            if link.confidence == Confidence::Confirmed {
                link.confidence = Confidence::Claimed;
                link.key = Key::None;
            }
            link.reason = format!(
                "{why}, so no owner reference ties the pods to the Deployment; {}; {which}",
                link.reason
            );
        }
    }
    let mut seen = deployment_revision(deployment);
    seen.append(&mut link.evidence);
    seen.extend(rolling(evidence, deployment, &read.hash));
    link.evidence = seen;
    link
}

/// The pods of the Deployment's older ReplicaSets that still exist, as one
/// observation: a rolling update is not a mismatch, so it is never a reason
/// for a claim, only something to see.
fn rolling(evidence: &Evidence, deployment: &Deployment, hash: &str) -> Option<Observation> {
    let older: Vec<&DeploymentSet> = evidence
        .deployment_sets
        .read()
        .into_iter()
        .flatten()
        .filter(|set| {
            set.replicas > 0
                && set.namespace == deployment.namespace
                && deployment.meta.uid.is_some()
                && set.owner_uid == deployment.meta.uid
                && set.pod_hash.as_deref() != Some(hash)
        })
        .collect();
    let pods: u64 = older.iter().map(|set| set.replicas).sum();
    if pods == 0 {
        return None;
    }
    let names: Vec<&str> = older.iter().map(|set| set.name.as_str()).collect();
    Some(concluded(
        &format!(
            "rolling: {pods} pod(s) of the previous revision, ReplicaSet {}",
            names.join(", ")
        ),
        hash,
    ))
}

/// Which pods were judged.
fn which_pods(
    hash: &str,
    deployment: &Deployment,
    tie: &Result<&DeploymentSet, String>,
    foreign: usize,
) -> String {
    let mut which = match tie {
        Ok(set) => format!(
            "the pods of the Deployment's current revision {}, ReplicaSet {} (pod-template-hash {hash})",
            deployment.revision.as_deref().unwrap_or("?"),
            set.name
        ),
        Err(_) => format!("the pods labelled with pod-template-hash {hash}"),
    };
    if foreign > 0 {
        which.push_str(&format!(
            "; {foreign} pod container(s) with that hash label belong to another owner and were not judged"
        ));
    }
    which
}
