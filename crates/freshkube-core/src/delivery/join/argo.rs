use crate::delivery::argocd::{
    Application, DestinationMatch, IN_CLUSTER_NAME, ManagedObject, StageClaim,
};
use crate::delivery::kargo::{Freight, Promotion, Stage};
use crate::delivery::source::cap_note;

use super::workload::rollout_links;
use super::*;

pub(super) fn application_links(
    evidence: &Evidence,
    freight: &Freight,
    stage: &Stage,
) -> Vec<Link> {
    let stage_id = id(&stage.project, &stage.name);
    let Some(apps) = evidence.applications.read() else {
        return vec![Link::new(
            Hop::Stage,
            Hop::Application,
            stage_id,
            Key::None,
            Confidence::Unknown,
            evidence.applications.why_not_read().unwrap_or_default(),
        )];
    };
    let owned: Vec<&Application> = apps
        .iter()
        .filter(|app| {
            app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                .is_some()
        })
        .collect();
    if owned.is_empty() {
        return vec![Link::new(
            Hop::Stage,
            Hop::Application,
            stage_id,
            Key::None,
            Confidence::Unknown,
            format!(
                "no Application names this Stage (read {}){}",
                apps.len(),
                cap_note(evidence.applications.capped())
            ),
        )];
    }
    let mut links = Vec::new();
    for app in owned {
        let app_id = id(&app.namespace, &app.name);
        let destination = evidence
            .destinations
            .get(&app_id)
            .map(describe_destination)
            .unwrap_or_else(|| "destination not matched".to_owned());
        let (confidence, key, proof) = stage_application_proof(evidence, freight, stage, app, apps);
        links.push(Link::new(
            Hop::Stage,
            Hop::Application,
            app_id.clone(),
            key.unwrap_or_else(|| Key::Name(format!("{}:{}", stage.project, stage.name))),
            confidence,
            format!(
                "{}; {proof}; sync {}, health {}; {destination}",
                app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                    .map_or("claimed", StageClaim::describe),
                app.sync.as_deref().unwrap_or("?"),
                app.health.as_deref().unwrap_or("?")
            ),
        ));
        links.extend(rollout_links(evidence, freight, app));
    }
    links
}

/// Whether a succeeded Promotion of this Freight to this Stage pushed the very
/// commit the Application synced. That is a SHA join, and the only thing that
/// confirms the Application receives this Stage's changes. It stays a claim
/// when the revision moved on, when no Promotion recorded a commit, or when an
/// Application of another stage in the project is at the same revision (they
/// may share a branch, so the commit alone would not tell them apart).
fn stage_application_proof(
    evidence: &Evidence,
    freight: &Freight,
    stage: &Stage,
    app: &Application,
    apps: &[Application],
) -> (Confidence, Option<Key>, String) {
    let promotions: Vec<&Promotion> = evidence
        .kargo
        .promotions
        .read()
        .map(|all| {
            all.iter()
                .filter(|p| {
                    p.project == stage.project
                        && p.stage.as_deref() == Some(stage.name.as_str())
                        && p.freight.as_deref() == Some(freight.name.as_str())
                        && p.phase.as_deref() == Some("Succeeded")
                })
                .collect()
        })
        .unwrap_or_default();
    let pushed: Vec<&String> = promotions
        .iter()
        .flat_map(|p| p.pushed_commits.iter())
        .collect();
    if pushed.is_empty() {
        return (
            Confidence::Claimed,
            None,
            format!(
                "no succeeded Promotion recorded a pushed commit{}",
                cap_note(evidence.kargo.promotions.capped())
            ),
        );
    }
    let at = |application: &Application, commit: &str| {
        application
            .sync_revisions
            .iter()
            .any(|revision| revision.eq_ignore_ascii_case(commit))
    };
    let Some(commit) = pushed.iter().find(|commit| at(app, commit)) else {
        return (
            Confidence::Claimed,
            None,
            format!(
                "the Promotion pushed {} but the Application's synced revision is {}",
                short_commit(pushed[0]),
                app.sync_revisions
                    .first()
                    .map_or("unknown".to_owned(), |r| short_commit(r))
            ),
        );
    };
    let shared = apps.iter().any(|other| {
        (other.namespace != app.namespace || other.name != app.name)
            && at(other, commit)
            && other
                .claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                .is_none()
            && other.is_of_project(&stage.project, evidence.stage_naming.as_ref())
    });
    if let Some(truncation) = evidence.applications.capped() {
        // An Application of another stage at the same commit may be among
        // those not read, so the commit can't single this one out.
        return (
            Confidence::Claimed,
            Some(Key::Sha((*commit).clone())),
            format!(
                "the Application synced the commit the Promotion pushed ({}), but the Application listing stopped at the page cap after {} items, so another stage's Application may be at it too",
                short_commit(commit),
                truncation.read
            ),
        );
    }
    if shared {
        return (
            Confidence::Claimed,
            Some(Key::Sha((*commit).clone())),
            format!(
                "the Application synced the commit the Promotion pushed ({}), but another stage's Application is at it too",
                short_commit(commit)
            ),
        );
    }
    (
        Confidence::Confirmed,
        Some(Key::Sha((*commit).clone())),
        format!(
            "the Application synced the commit the Promotion pushed ({})",
            short_commit(commit)
        ),
    )
}

/// Whether `revision`, a full or abbreviated commit id, names the commit
/// `sha`. An abbreviation of seven or more characters is the start of the
/// full id, case aside. `None` when the two can't be compared: a shorter
/// abbreviation, an id longer than the commit's (another hash algorithm),
/// or one that differs from a SHA-256 commit, which may also be known by a
/// SHA-1 id.
pub(super) fn same_commit(revision: &str, sha: &str) -> Option<bool> {
    const SHORTEST: usize = 7;
    let (revision, sha) = (revision.as_bytes(), sha.as_bytes());
    if revision.len() < SHORTEST || revision.len() > sha.len() {
        None
    } else if revision.eq_ignore_ascii_case(&sha[..revision.len()]) {
        Some(true)
    } else if revision.len() == sha.len() || sha.len() == 40 {
        Some(false)
    } else {
        None
    }
}

pub(super) fn short_commit(commit: &str) -> String {
    commit.chars().take(12).collect()
}

fn describe_destination(destination: &DestinationMatch) -> String {
    match destination {
        DestinationMatch::One(context) => format!("destination is context {context}"),
        DestinationMatch::Ambiguous(contexts) => format!(
            "destination matches several contexts ({}); the user must choose",
            contexts.join(", ")
        ),
        DestinationMatch::ArgoCd(context) => {
            format!("destination is Argo CD's own cluster ({IN_CLUSTER_NAME}), context {context}")
        }
        DestinationMatch::Named { context, .. } => {
            format!("destination by a cluster name, mapped to context {context}")
        }
        DestinationMatch::None => "destination matches no known context".to_owned(),
        DestinationMatch::ByName(_) => {
            "destination by a cluster name no context is mapped to".to_owned()
        }
        DestinationMatch::Unspecified => "no destination".to_owned(),
    }
}

/// The namespace a managed Rollout runs in: its own, else the
/// Application's destination namespace. `None` when neither names one, and
/// then it is not read.
pub(super) fn rollout_namespace(app: &Application, managed: &ManagedObject) -> Option<String> {
    managed
        .namespace
        .clone()
        .or_else(|| app.destination_namespace.clone())
        .filter(|namespace| !namespace.is_empty())
}

/// Why nothing was read in the environment cluster for an Application, and
/// how the user would say otherwise when its destination is that cluster.
pub(super) fn not_the_environment(evidence: &Evidence, app_id: &str) -> String {
    let env = &evidence.environment;
    let why = match evidence.destinations.get(app_id) {
        Some(DestinationMatch::ByName(_)) => format!(
            "its cluster name is mapped to no context; if it is the environment cluster, map the name the Application gives with --known-as-name {env}=<cluster name>"
        ),
        Some(DestinationMatch::None) => format!(
            "its server matches no context; if it is the environment cluster's, add it with --known-as {env}=<server>"
        ),
        Some(DestinationMatch::Ambiguous(contexts)) => format!(
            "it matches several contexts ({}); the user must choose",
            contexts.join(", ")
        ),
        Some(DestinationMatch::Unspecified) => "the Application names no destination".to_owned(),
        Some(matched) => format!(
            "it is context {}, and the environment is {env}",
            matched.context().unwrap_or("?")
        ),
        None => "its destination was not matched".to_owned(),
    };
    format!(
        "the destination is not known to be the environment cluster, so nothing was read there for it: {why}"
    )
}
