use crate::delivery::argocd::Application;
use crate::delivery::digest::{Digest, repository, tag};
use crate::delivery::kargo::Freight;
use crate::delivery::observation::{Fact, Observation};
use crate::delivery::pods::RunningImage;
use crate::delivery::rollouts::{AnalysisRun, Container, ReplicaSet, Rollout};
use crate::delivery::source::{Truncation, cap_note};

use super::argo::{not_the_environment, rollout_namespace};
use super::deployment::deployment_links;
use super::observe::{
    freight_side, manages, pods_running, pods_running_other, revision_tie, rollout_hash,
    rollout_state as rollout_seen, summary_entries, summary_images,
};
use super::*;

pub(super) fn rollout_links(
    evidence: &Evidence,
    freight: &Freight,
    app: &Application,
) -> Vec<Link> {
    let app_id = id(&app.namespace, &app.name);
    let managed: Vec<_> = app.managed.iter().filter(|m| m.kind == "Rollout").collect();
    let deployments = app
        .managed
        .iter()
        .any(|m| m.group == "apps" && m.kind == "Deployment");
    if managed.is_empty() && deployments {
        return deployment_links(evidence, freight, app);
    }
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
        let mut seen = vec![manages(app, "Rollout", &rollout.name)];
        seen.extend(rollout_seen(rollout, pinned.as_ref()));
        if let Some(digest) = &pinned {
            seen.extend(freight_side(freight, &Key::Digest(digest.clone())));
        }
        let pods = pod_link(evidence, freight, rollout, &rollout_id);
        links.push(match pinned {
            Some(digest) => pinned_link(
                app,
                Controller::of(rollout),
                rollout_id.clone(),
                digest,
                &pods,
                seen,
                &state,
            ),
            None => Link::new(
                Hop::Application,
                Hop::Rollout,
                rollout_id.clone(),
                Key::Name(rollout.name.clone()),
                Confidence::Claimed,
                format!("managed by the Application; its spec does not pin the digest; {state}"),
            )
            .observed(seen),
        });
        links.push(pods);
    }
    links
}

/// What a pinned link needs to know of the workload controller it names: its
/// kind, the hop it is, and the images its pod template names.
pub(super) struct Controller<'a> {
    pub kind: &'static str,
    pub hop: Hop,
    pub images: &'a [String],
}

impl<'a> Controller<'a> {
    fn of(rollout: &'a Rollout) -> Self {
        Self {
            kind: "Rollout",
            hop: Hop::Rollout,
            images: &rollout.images,
        }
    }
}

/// The link to a workload controller whose spec pins the Freight's digest.
/// The pin is declared; the link is Confirmed only when Argo CD's image
/// summary, which it compiles from the live pods, lists the digest for the
/// Application, and the controller's own current pods report it (`pods`, its
/// link to them).
pub(super) fn pinned_link(
    app: &Application,
    controller: Controller<'_>,
    subject: String,
    digest: Digest,
    pods: &Link,
    mut seen: Vec<Observation>,
    state: &str,
) -> Link {
    // Argo CD lists the pods' images as their spec names them, so the entry
    // names the pin's repository too; the digest under another is not it.
    let repo = controller
        .images
        .iter()
        .find(|image| Digest::from_reference(image).as_ref() == Some(&digest))
        .map(|image| repository(image));
    let same: Vec<&str> = app
        .images
        .iter()
        .filter(|image| repo.as_deref() == Some(repository(image).as_str()))
        .map(String::as_str)
        .collect();
    let listed = same
        .iter()
        .any(|image| Digest::from_reference(image).as_ref() == Some(&digest));
    let running = pods.confidence == Confidence::Confirmed
        && pods.evidence.iter().any(|seen| {
            seen.object.kind == "Pod"
                && seen.fact == Fact::Reported
                && seen.value.as_deref() == Some(digest.as_str())
        });
    let mut missing = Vec::new();
    if listed {
        seen.extend(summary_images(app, std::slice::from_ref(&digest)));
    } else {
        seen.extend(summary_entries(app, &same));
        missing.push(if app.images.is_empty() {
            "Argo CD reports no image summary for the Application".to_owned()
        } else if same.is_empty() {
            "Argo CD's image summary does not list its image".to_owned()
        } else {
            let named: Vec<String> = same.iter().map(|image| summary_name(image)).collect();
            format!("Argo CD's image summary lists {} instead", named.join(", "))
        });
    }
    if !running {
        missing.push(format!("its current pods: {}", pods.reason));
    }
    seen.extend(pods.evidence.iter().cloned());
    if missing.is_empty() {
        Link::new(
            Hop::Application,
            controller.hop,
            subject,
            Key::Digest(digest),
            Confidence::Confirmed,
            format!(
                "the {kind}'s spec pins the Freight's digest, Argo CD's image summary lists it, and the {kind}'s current pods run it; {state}",
                kind = controller.kind
            ),
        )
        .observed(seen)
    } else {
        Link::new(
            Hop::Application,
            controller.hop,
            subject,
            Key::Digest(digest),
            Confidence::Claimed,
            format!(
                "the {}'s spec pins the Freight's digest, declared only; {}; {state}",
                controller.kind,
                missing.join("; ")
            ),
        )
        .observed(seen)
    }
}

/// An image as Argo CD's summary lists it, for a reason: its short digest,
/// else its tag.
pub(super) fn summary_name(image: &str) -> String {
    match Digest::from_reference(image) {
        Some(digest) => digest.short(),
        None => tag(image).map_or_else(|| image.to_owned(), |tag| format!("tag {tag}")),
    }
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
    let unknown = |why: String| {
        Link::new(
            Hop::Rollout,
            Hop::Pod,
            subject.clone(),
            Key::None,
            Confidence::Unknown,
            why,
        )
    };
    let Some(read) = evidence.pods.get(rollout_id) else {
        return unknown(if rollout.current_pod_hash.is_none() {
            "the Rollout reports no current pod hash".into()
        } else {
            "pods were not read".into()
        });
    };
    let Some(pods) = read.pods.read() else {
        return unknown(read.pods.why_not_read().unwrap_or_default());
    };
    let tie = current_set(evidence, rollout, &read.hash);
    // Tied by owner UIDs, only the current ReplicaSet's own pods count; a pod
    // that carries the hash label but another owner is not the Rollout's.
    let tied: Vec<&RunningImage> = pods
        .iter()
        .filter(|pod| match &tie {
            Ok(set) => set.meta.uid.is_some() && pod.owner_uid == set.meta.uid,
            Err(_) => true,
        })
        .collect();
    let containers = pinned_containers(&rollout.containers, freight, &tied);
    let mut link = judge_revision(
        Hop::Rollout,
        subject,
        &tied,
        &containers,
        read.pods.capped(),
        freight,
    );
    let which = which_pods(evidence, rollout, &read.hash, &tie, pods.len() - tied.len());
    match &tie {
        Ok(set) if link.confidence == Confidence::Confirmed => {
            let mut seen = revision_tie(rollout, set, &running(&containers, freight));
            seen.append(&mut link.evidence);
            link.evidence = seen;
            link.reason = format!("{}; {which}", link.reason);
        }
        Ok(_) => link.reason = format!("{}; {which}", link.reason),
        Err(why) => {
            if link.confidence == Confidence::Confirmed {
                link.confidence = Confidence::Claimed;
                link.key = Key::None;
            }
            link.reason = format!("{why}; {}; {which}", link.reason);
        }
    }
    let mut seen = rollout_hash(rollout);
    seen.append(&mut link.evidence);
    link.evidence = seen;
    link
}

/// The ReplicaSet the Rollout owns, by UID, with its current pod hash: the
/// tie from the Rollout to its current pods. Why there is none otherwise.
fn current_set<'a>(
    evidence: &'a Evidence,
    rollout: &'a Rollout,
    hash: &str,
) -> Result<&'a ReplicaSet, String> {
    let untied = "so no owner reference ties the pods to the Rollout";
    let Some(sets) = evidence.replica_sets.read() else {
        let why = evidence.replica_sets.why_not_read().unwrap_or_default();
        return Err(format!("its ReplicaSets were not read ({why}), {untied}"));
    };
    if rollout.meta.uid.is_none() {
        return Err(format!("the Rollout reports no UID, {untied}"));
    }
    owned_replica_sets(rollout, Some(sets))
        .find(|set| set.pod_hash.as_deref() == Some(hash) && set.meta.uid.is_some())
        .ok_or_else(|| {
            format!(
                "no ReplicaSet the Rollout owns has its current pod hash {hash}{}, {untied}",
                cap_note(evidence.replica_sets.capped())
            )
        })
}

/// The containers of `pods` that run the pin's image: those of the Freight's
/// repository, or named as the Rollout's container of that repository is.
/// A sidecar is neither, so what it runs doesn't count.
pub(super) fn pinned_containers<'a>(
    template: &[Container],
    freight: &Freight,
    pods: &[&'a RunningImage],
) -> Vec<&'a RunningImage> {
    let repos: Vec<String> = freight
        .images
        .iter()
        .map(|image| repository(&image.repo_url))
        .collect();
    let names: Vec<&str> = template
        .iter()
        .filter(|container| repos.contains(&repository(&container.image)))
        .map(|container| container.name.as_str())
        .collect();
    pods.iter()
        .copied()
        .filter(|pod| {
            names.contains(&pod.container.as_str())
                || pod
                    .image
                    .as_deref()
                    .is_some_and(|image| repos.contains(&repository(image)))
        })
        .collect()
}

/// The containers that report one of the Freight's digests, ready ones first.
pub(super) fn running<'a>(
    containers: &[&'a RunningImage],
    freight: &Freight,
) -> Vec<(&'a RunningImage, &'a Digest)> {
    let mut found: Vec<(&RunningImage, &Digest)> = containers
        .iter()
        .filter_map(|pod| {
            let digest = pod.digest.as_ref()?;
            freight
                .images
                .iter()
                .any(|image| image.digest.as_ref() == Some(digest))
                .then_some((*pod, digest))
        })
        .collect();
    found.sort_by_key(|(pod, _)| !pod.ready);
    found
}

/// What the current revision's pods run of the pin's image. Confirmed needs
/// a ready container that reports the Freight's digest and none that
/// reports another.
pub(super) fn judge_revision(
    from: Hop,
    subject: String,
    pods: &[&RunningImage],
    containers: &[&RunningImage],
    capped: Option<Truncation>,
    freight: &Freight,
) -> Link {
    let link = |confidence, key, reason: String| {
        Link::new(from, Hop::Pod, subject.clone(), key, confidence, reason)
    };
    let matching = running(containers, freight);
    let other: Vec<(&RunningImage, Option<Digest>)> = containers
        .iter()
        .filter(|pod| {
            pod.digest
                .as_ref()
                .is_some_and(|digest| !matching.iter().any(|(_, d)| *d == digest))
        })
        .map(|pod| (*pod, pod.digest.clone()))
        .collect();
    if !other.is_empty() {
        let digests: Vec<String> = other
            .iter()
            .map(|(_, digest)| digest.as_ref().map_or("no digest".into(), Digest::short))
            .collect();
        return link(
            Confidence::Claimed,
            Key::None,
            format!(
                "the Stage claims this Freight but the current revision's pods run another digest ({}); a moved tag is not a match{}",
                digests.join(", "),
                cap_note(capped)
            ),
        )
        .observed(pods_running_other(&other));
    }
    let ready = matching.iter().filter(|(pod, _)| pod.ready).count();
    // That none runs another digest is known only of the pods read.
    if let (true, Some(truncation)) = (ready > 0, capped) {
        return link(
            Confidence::Claimed,
            Key::None,
            format!(
                "{} container(s) run the Freight's digest, {ready} ready of {} pod container(s) read; the pod listing stopped at the page cap after {} items, so a pod not read may run another digest",
                matching.len(),
                containers.len(),
                truncation.read
            ),
        )
        .observed(pods_running(&matching));
    }
    if ready > 0 {
        let (_, digest) = matching[0];
        return link(
            Confidence::Confirmed,
            Key::Digest(digest.clone()),
            format!(
                "{} container(s) run the Freight's digest, {ready} ready of {} pod container(s) read",
                matching.len(),
                containers.len()
            ),
        )
        .observed(pods_running(&matching));
    }
    if !matching.is_empty() {
        return link(
            Confidence::Claimed,
            Key::None,
            format!(
                "{} container(s) of the current revision report the Freight's digest, none ready yet",
                matching.len()
            ),
        )
        .observed(pods_running(&matching));
    }
    if containers.is_empty() && !pods.is_empty() {
        return link(
            Confidence::Unknown,
            Key::None,
            format!(
                "read {} pod container(s); none runs the Freight's image{}",
                pods.len(),
                cap_note(capped)
            ),
        );
    }
    link(
        Confidence::Claimed,
        Key::None,
        format!(
            "no pod of the current revision reports a digest yet{}",
            cap_note(capped)
        ),
    )
}

/// Which pods were judged, and which of the Rollout's other ReplicaSets with
/// pods were not.
fn which_pods(
    evidence: &Evidence,
    rollout: &Rollout,
    hash: &str,
    tie: &Result<&ReplicaSet, String>,
    foreign: usize,
) -> String {
    let mut which = match tie {
        Ok(set) => format!(
            "the pods of the Rollout's current pod hash {hash}, ReplicaSet {}",
            set.name
        ),
        Err(_) => format!("the pods of the Rollout's current pod hash {hash}"),
    };
    if foreign > 0 {
        which.push_str(&format!(
            "; {foreign} pod container(s) with that hash label belong to another owner and were not judged"
        ));
    }
    let others: Vec<&str> =
        owned_replica_sets(rollout, evidence.replica_sets.read().map(Vec::as_slice))
            .filter(|other| other.replicas > 0 && other.pod_hash.as_deref() != Some(hash))
            .map(|other| other.name.as_str())
            .collect();
    if others.is_empty() {
        which
    } else {
        format!(
            "{which}; not judged: the pods of ReplicaSet {}",
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
            // What Argo CD's summary lists of the Freight's images: nothing
            // when it doesn't list the digest.
            let listed: Vec<Digest> = app
                .digests()
                .into_iter()
                .filter(|d| freight.images.iter().any(|i| i.digest.as_ref() == Some(d)))
                .collect();
            let from = summary_images(app, &listed);
            let mut link = judge_pods(
                Hop::Application,
                app_id.to_owned(),
                pods,
                source.capped(),
                freight,
                from,
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
    from_side: Vec<Observation>,
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
        let mut seen = from_side;
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
