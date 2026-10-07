use crate::delivery::argocd::Application;
use crate::delivery::digest::{Digest, repository};
use crate::delivery::kargo::Freight;
use crate::delivery::observation::{ObjectRef, Observation, role};
use crate::delivery::pods::RunningImage;
use crate::delivery::rollouts::{AnalysisRun, Rollout};
use crate::delivery::source::{Truncation, cap_note};

use super::argo::{not_the_environment, rollout_namespace};
use super::observe::{pods_running, pods_running_other, side};
use super::*;

/// The hop a pod link starts from, as the evidence for the link's first side
/// reads: what was seen of it, and where the join would match the digest.
struct Origin {
    cluster: &'static str,
    object: ObjectRef,
    field: &'static str,
    seen: Vec<Observation>,
}

pub(super) fn rollout_links(
    evidence: &Evidence,
    freight: &Freight,
    app: &Application,
) -> Vec<Link> {
    let app_id = id(&app.namespace, &app.name);
    let managed: Vec<_> = app.managed.iter().filter(|m| m.kind == "Rollout").collect();
    if managed.is_empty() {
        return vec![workload_pod_link(evidence, freight, app, &app_id)];
    }
    if !evidence.deploys_to_environment(&app_id) {
        let why = not_the_environment(evidence, &app_id);
        return vec![Link::new(
            Hop::Application,
            Hop::Rollout,
            app_id,
            Key::None,
            Confidence::Unknown,
            why,
        )];
    }
    let Some(rollouts) = evidence.rollouts.read() else {
        return vec![Link::new(
            Hop::Application,
            Hop::Rollout,
            app_id,
            Key::None,
            Confidence::Unknown,
            evidence.rollouts.why_not_read().unwrap_or_default(),
        )];
    };
    let mut links = Vec::new();
    for object in managed {
        let Some(namespace) = rollout_namespace(app, object) else {
            links.push(Link::new(
                Hop::Application,
                Hop::Rollout,
                format!("{app_id}: {}", object.name),
                Key::None,
                Confidence::Unknown,
                "neither the Rollout nor the Application's destination names a namespace, so it was not read",
            ));
            continue;
        };
        let rollout_id = id(&namespace, &object.name);
        let Some(rollout) = rollouts
            .iter()
            .find(|r| r.namespace == namespace && r.name == object.name)
        else {
            links.push(Link::new(
                Hop::Application,
                Hop::Rollout,
                rollout_id,
                Key::None,
                Confidence::Unknown,
                format!(
                    "the Application lists it but it was not found in the environment cluster{}",
                    cap_note(evidence.rollouts.capped())
                ),
            ));
            continue;
        };
        let pinned = rollout
            .images
            .iter()
            .filter_map(|image| Digest::from_reference(image))
            .find(|digest| {
                freight
                    .images
                    .iter()
                    .any(|image| image.digest.as_ref() == Some(digest))
            });
        let state = rollout_state(rollout, evidence.analysis_runs.read());
        let managed = Observation::reported(
            role::ARGOCD,
            app.object_ref(),
            "/status/resources",
            Some(&format!("Rollout/{}", rollout.name)),
        );
        let phase = Observation::reported(
            role::ENVIRONMENT,
            rollout.object_ref(),
            "/status/phase",
            rollout.phase.as_deref(),
        );
        links.push(match pinned {
            Some(digest) => {
                let value = digest.to_string();
                let field = "/spec/template/spec/containers";
                let mut seen = side(
                    vec![managed],
                    &value,
                    role::ARGOCD,
                    app.object_ref(),
                    "/status/resources",
                );
                seen.push(phase);
                seen.extend(side(
                    vec![Observation::declared(
                        role::ENVIRONMENT,
                        rollout.object_ref(),
                        field,
                        Some(&value),
                    )],
                    &value,
                    role::ENVIRONMENT,
                    rollout.object_ref(),
                    field,
                ));
                Link::new(
                    Hop::Application,
                    Hop::Rollout,
                    rollout_id.clone(),
                    Key::Digest(digest),
                    Confidence::Confirmed,
                    format!("the Rollout's spec pins the Freight's digest; {state}"),
                )
                .observed(seen)
            }
            None => Link::new(
                Hop::Application,
                Hop::Rollout,
                rollout_id.clone(),
                Key::Name(rollout.name.clone()),
                Confidence::Claimed,
                format!("managed by the Application; its spec does not pin the digest; {state}"),
            )
            .observed(vec![managed, phase]),
        });
        links.push(pod_link(evidence, freight, rollout, &rollout_id));
    }
    links
}

fn rollout_state(rollout: &Rollout, analysis: Option<&Vec<AnalysisRun>>) -> String {
    let mut parts = vec![format!(
        "phase {}",
        rollout.phase.as_deref().unwrap_or("unreported")
    )];
    if rollout.aborted {
        parts.push("aborted".into());
    }
    if rollout.paused {
        parts.push("paused".into());
    }
    if let Some(runs) = analysis {
        let own: Vec<String> = runs
            .iter()
            .filter(|run| run.rollout.as_deref() == Some(rollout.name.as_str()))
            .map(|run| format!("{}={}", run.name, run.phase.as_deref().unwrap_or("?")))
            .collect();
        if !own.is_empty() {
            parts.push(format!("analysis {}", own.join(", ")));
        }
    }
    parts.join(", ")
}

fn pod_link(evidence: &Evidence, freight: &Freight, rollout: &Rollout, rollout_id: &str) -> Link {
    let subject = rollout_id.to_owned();
    let read = evidence.pods.get(rollout_id);
    let (pods, which) = match read {
        Some(read) if read.pods.read().is_some() => (
            read.pods.read().unwrap(),
            which_pods(evidence, rollout, &read.set),
        ),
        Some(other) => {
            return Link::new(
                Hop::Rollout,
                Hop::Pod,
                subject,
                Key::None,
                Confidence::Unknown,
                other.pods.why_not_read().unwrap_or_default(),
            );
        }
        None => {
            return Link::new(
                Hop::Rollout,
                Hop::Pod,
                subject,
                Key::None,
                Confidence::Unknown,
                if rollout.current_pod_hash.is_none() {
                    "the Rollout reports no current pod hash"
                } else {
                    "pods were not read"
                },
            );
        }
    };
    let origin = Origin {
        cluster: role::ENVIRONMENT,
        object: rollout.object_ref(),
        field: "/spec/template/spec/containers",
        seen: Vec::new(),
    };
    let mut link = judge_pods(
        Hop::Rollout,
        subject,
        pods,
        read.and_then(|read| read.pods.capped()),
        freight,
        origin,
    );
    link.reason = format!("{}; {which}", link.reason);
    link
}

/// Which pods were judged, and which of the Rollout's other ReplicaSets with
/// pods were not.
fn which_pods(evidence: &Evidence, rollout: &Rollout, set: &PodSet) -> String {
    let read = evidence.replica_sets.read().map(Vec::as_slice);
    let which = match set {
        PodSet::Pinned(sets) => format!(
            "the pods of ReplicaSet {}, whose template pins the Freight's digest{}",
            sets.iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            cap_note(evidence.replica_sets.capped())
        ),
        PodSet::Current(hash) => {
            let why = match evidence.replica_sets.why_not_read() {
                Some(why) => format!("its ReplicaSets were not read ({why})"),
                None => format!(
                    "no ReplicaSet of it pins the Freight's digest{}",
                    cap_note(evidence.replica_sets.capped())
                ),
            };
            format!("the pods of the Rollout's current pod hash {hash}; {why}")
        }
    };
    let judged = set.hashes();
    let unlike = match set {
        PodSet::Pinned(_) => ", whose template does not pin it",
        PodSet::Current(_) => "",
    };
    let others: Vec<&str> = owned_replica_sets(rollout, read)
        .filter(|other| {
            other.replicas > 0
                && other
                    .pod_hash
                    .as_deref()
                    .is_none_or(|hash| !judged.contains(&hash))
        })
        .map(|other| other.name.as_str())
        .collect();
    if others.is_empty() {
        which
    } else {
        format!(
            "{which}; not judged: the pods of ReplicaSet {}{unlike}",
            others.join(", ")
        )
    }
}

/// The Application's pods when it manages no Rollout: whatever runs in its
/// destination namespace, judged by digest like a Rollout's pods. Argo CD's
/// own image summary is reported but is not what joins.
fn workload_pod_link(
    evidence: &Evidence,
    freight: &Freight,
    app: &Application,
    app_id: &str,
) -> Link {
    let unknown = |reason: String| {
        Link::new(
            Hop::Application,
            Hop::Pod,
            app_id.to_owned(),
            Key::None,
            Confidence::Unknown,
            reason,
        )
    };
    if !evidence.deploys_to_environment(app_id) {
        return unknown(not_the_environment(evidence, app_id));
    }
    match evidence.namespace_pods.get(app_id) {
        Some(source) if source.read().is_some() => {
            let pods = source.read().unwrap();
            let summary = if app
                .digests()
                .iter()
                .any(|d| freight.images.iter().any(|i| i.digest.as_ref() == Some(d)))
            {
                "Argo CD's image summary lists the Freight's digest"
            } else {
                "Argo CD's image summary does not list the Freight's digest"
            };
            // What Argo CD's summary lists of the Freight's images.
            let listed: Vec<Observation> = app
                .digests()
                .iter()
                .filter(|d| freight.images.iter().any(|i| i.digest.as_ref() == Some(d)))
                .map(|digest| {
                    Observation::reported(
                        role::ARGOCD,
                        app.object_ref(),
                        "/status/summary/images",
                        Some(digest.as_str()),
                    )
                })
                .collect();
            let origin = Origin {
                cluster: role::ARGOCD,
                object: app.object_ref(),
                field: "/status/summary/images",
                seen: listed,
            };
            let mut link = judge_pods(
                Hop::Application,
                app_id.to_owned(),
                pods,
                source.capped(),
                freight,
                origin,
            );
            link.reason = format!("{summary}; {}", link.reason);
            link
        }
        Some(other) => unknown(other.why_not_read().unwrap_or_default()),
        None => unknown(
            "the pods of the destination namespace were not read (the destination has no namespace)"
                .to_owned(),
        ),
    }
}

fn judge_pods(
    from: Hop,
    subject: String,
    pods: &[RunningImage],
    capped: Option<Truncation>,
    freight: &Freight,
    origin: Origin,
) -> Link {
    let wanted: Vec<&Digest> = freight
        .images
        .iter()
        .filter_map(|image| image.digest.as_ref())
        .collect();
    let repos: Vec<String> = freight
        .images
        .iter()
        .map(|image| repository(&image.repo_url))
        .collect();
    let mut matching = Vec::new();
    let mut other: Vec<(&RunningImage, Option<Digest>)> = Vec::new();
    for pod in pods {
        match &pod.digest {
            Some(digest) if wanted.contains(&digest) => matching.push((pod, digest)),
            digest => {
                let same_repo = pod
                    .image
                    .as_deref()
                    .is_some_and(|image| repos.contains(&repository(image)));
                if same_repo {
                    other.push((pod, digest.clone()));
                }
            }
        }
    }
    let ready = matching.iter().filter(|(pod, _)| pod.ready).count();
    if let Some((_, digest)) = matching.first() {
        let extra = if other.is_empty() {
            String::new()
        } else {
            format!(
                "; {} other container(s) of the same image run a different digest",
                other.len()
            )
        };
        let mut seen = side(
            origin.seen,
            digest.as_str(),
            origin.cluster,
            origin.object,
            origin.field,
        );
        seen.extend(pods_running(&matching));
        Link::new(
            from,
            Hop::Pod,
            subject,
            Key::Digest((*digest).clone()),
            Confidence::Confirmed,
            format!(
                "{} container(s) run the Freight's digest, {ready} ready of {} pod container(s) read{extra}",
                matching.len(),
                pods.len()
            ),
        )
        .observed(seen)
    } else if !other.is_empty() {
        let digests: Vec<String> = other
            .iter()
            .map(|(_, digest)| {
                digest
                    .as_ref()
                    .map_or("no digest".to_owned(), Digest::short)
            })
            .collect();
        Link::new(
            from,
            Hop::Pod,
            subject,
            Key::None,
            Confidence::Claimed,
            format!(
                "the Stage claims this Freight but the pods run another digest ({}); a moved tag is not a match{}",
                digests.join(", "),
                cap_note(capped)
            ),
        )
        .observed(pods_running_other(&other))
    } else {
        Link::new(
            from,
            Hop::Pod,
            subject,
            Key::None,
            Confidence::Unknown,
            format!(
                "read {} pod container(s); none runs the Freight's image{}",
                pods.len(),
                cap_note(capped)
            ),
        )
    }
}
