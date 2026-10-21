//! Workloads reported by Argo CD on a destination that was not read.

use super::*;
use freshkube_core::resources::FailureKind;

/// The Application `app_id` names and the workspace cluster it is mapped
/// to, when that is a cluster the workspace lists other than this one, and
/// nothing was read there.
pub(super) fn elsewhere<'a>(
    place: &Place,
    evidence: &'a Evidence,
    app_id: &str,
) -> Option<Away<'a>> {
    let entry = evidence.destination_of(app_id)?;
    if place.is_here(entry) || !place.mapping.lists(entry) {
        return None;
    }
    let (why, refused) = match evidence.environments.get(entry) {
        Some(Ok(_)) => return None,
        Some(Err(why)) => (
            Some(why.message.as_str()),
            matches!(why.kind, FailureKind::Forbidden | FailureKind::Unauthorized),
        ),
        None => (None, false),
    };
    let app = evidence
        .applications
        .read()?
        .iter()
        .find(|app| format!("{}/{}", app.namespace, app.name) == app_id)?;
    Some(Away {
        app,
        entry,
        why,
        refused,
    })
}

/// Whether Argo CD reports a workload for an Application that deploys to
/// `context`.
pub(in super::super) fn reported_on(evidence: &Evidence, context: &str) -> bool {
    evidence.applications.read().is_some_and(|apps| {
        apps.iter().any(|app| {
            reports(app)
                && evidence.destination_of(&format!("{}/{}", app.namespace, app.name))
                    == Some(context)
        })
    })
}

/// The workloads Argo CD reports for an Application on another workspace
/// cluster, from its `status.resources`: what Argo CD says, since that
/// cluster wasn't read, so each link is Argo CD's claim. Each opens on that
/// cluster.
pub(super) fn reported_hops(sides: &Sides, stage: &str, group: usize, away: Away) -> Vec<Hop> {
    let app = away.app;
    app.managed
        .iter()
        .filter(|object| {
            WORKLOADS
                .iter()
                .any(|(g, kind)| object.group == *g && object.kind == *kind)
        })
        .enumerate()
        .flat_map(|(n, object)| {
            let workload = reported_hop(
                sides,
                format!("{stage}-reported-{}-{n}", app.name),
                group,
                &away,
                object,
            );
            let pods = away
                .why
                .filter(|_| object.health.as_deref() != Some("Missing"))
                .map(|why| crate::change::unread::pods(&workload, away.entry, why, away.refused));
            std::iter::once(workload).chain(pods)
        })
        .collect()
}

fn reported_hop(
    sides: &Sides,
    key: String,
    group: usize,
    away: &Away,
    object: &ManagedObject,
) -> Hop {
    let (app, entry) = (away.app, away.entry);
    let namespace = object
        .namespace
        .clone()
        .or_else(|| app.destination_namespace.clone())
        .unwrap_or_default();
    let reports: Vec<&str> = [object.sync.as_deref(), object.health.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let words = if reports.is_empty() {
        "Argo CD reports no state".to_owned()
    } else {
        format!("Argo CD reports {}", reports.join(" · "))
    };
    // Argo CD's word for an object nobody read is never Healthy, so a Stage
    // known only from it doesn't fold as fine.
    let state = match object.health.as_deref() {
        Some("Healthy") => Info,
        Some("Degraded" | "Missing") => Warning,
        Some("Progressing" | "Suspended") => Info,
        _ => Unknown,
    };
    let mut fields = Vec::new();
    if !namespace.is_empty() {
        fields.push(Field {
            label: "Namespace".into(),
            value: Value::Mono(namespace.clone()),
        });
    }
    fields.push(Field {
        label: "Name".into(),
        value: Value::Mono(object.name.clone()),
    });
    fields.push(Field {
        label: "Cluster".into(),
        value: Value::Mono(entry.to_owned()),
    });
    if let Some(message) = &object.health_message {
        fields.push(Field {
            label: "Argo CD says".into(),
            value: Value::Text(message.clone()),
        });
    }
    fields.push(Field {
        label: "Read from".into(),
        value: Value::Mono(format!("Argo CD on {}", sides.argocd.label)),
    });
    let shown = if namespace.is_empty() {
        object.name.clone()
    } else {
        format!("{namespace}/{}", object.name)
    };
    let target = Object {
        cluster: String::new(),
        group: object.group.clone(),
        version: object.version.clone().unwrap_or_else(|| {
            if object.group == "apps" {
                "v1".into()
            } else {
                "v1alpha1".into()
            }
        }),
        kind: object.kind.clone(),
        plural: format!("{}s", object.kind.to_lowercase()),
        namespace,
        name: object.name.clone(),
    };
    Hop {
        cause: None,
        key,
        group,
        state,
        name: format!("{} {}", object.kind, object.name),
        detail: format!("Argo CD reports it on {entry}; not read there"),
        from: sides.argocd.label.clone(),
        at: None,
        link: Some(Confidence::Claimed),
        after: None,
        shows: Shows::Hop(Box::new(HopDetail {
            kind: object.kind.clone(),
            title: object.name.clone(),
            state: words,
            notice: Some(Notice {
                state: Info,
                lead: "Not read.".into(),
                body: match away.why {
                    Some(why) => sentence(&format!(
                        "{why}, so this is what Argo CD reports, not what was read there"
                    )),
                    None => format!(
                        "{entry} wasn't read for this change, so this is what Argo CD reports, not what was read there."
                    ),
                },
            }),
            fields,
            link: Some(LinkDetail {
                cause: None,
                confidence: Confidence::Claimed,
                by: "name".into(),
                before: "Application".into(),
                key: Some(object.name.clone()),
                before_says: format!("status.resources lists {} {shown}", object.kind),
                here_says: "not read".into(),
                why: match away.why {
                    Some(why) => format!(
                        "Only Argo CD's status says so: {why}, so nothing there confirms it."
                    ),
                    None => format!(
                        "Only Argo CD's status says so: {entry} isn't read, so nothing there confirms it."
                    ),
                },
            }),
            unlinked: None,
            actions: vec![Action::new(Target::OnEntry {
                what: format!("the {}", object.kind),
                entry: entry.to_owned(),
                object: target,
            })],
        })),
    }
}
