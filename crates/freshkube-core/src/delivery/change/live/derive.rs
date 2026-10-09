//! What was read, as the page's [`Change`]: the join's links in trail order,
//! one hop each, grouped Build, Freight, then each Stage the Freight can
//! reach in promotion order, with that Stage's three gates first.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::super::*;
use super::Place;
use super::gates::{self, Row, instant};
use crate::delivery::argocd::DestinationMatch;
use crate::delivery::join::{self, Confidence, Evidence, Hop as Joined, Key, Link, freight_links};
use crate::delivery::kargo::{self, Freight, Stage as KargoStage};
use crate::delivery::observation::{ObjectRef, Observation};
use crate::indicators::HealthIndicator::{self, *};

/// The change `place` names, from what was read and the join's links.
/// `freight` is the Freight as first read; the one the project's list
/// holds wins, as the join used it.
pub fn derive(place: &Place, evidence: &Evidence, freight: &Freight) -> Change {
    let freight = evidence
        .kargo
        .freight
        .read()
        .and_then(|all| all.iter().find(|item| item.name == freight.name))
        .unwrap_or(freight);
    let trail = (!evidence.sha.is_empty()).then(|| join::join(evidence));
    let links = freight_links(evidence, freight);
    let stages = reachable(freight, evidence);

    let mut groups = vec![
        Group {
            phase: Phase::Build,
            detail: format!("Tekton on {}", place.label),
        },
        Group {
            phase: Phase::Freight,
            detail: format!("Kargo on {}", place.label),
        },
    ];
    let mut hops = build_hops(place, evidence, freight, trail.as_ref());
    hops.push(freight_hop(place, evidence, freight, trail.as_ref()));
    if let Some(warehouse) = links.iter().find(|link| link.to == Joined::Warehouse) {
        let mut hop = link_hop(place, evidence, freight, "warehouse".into(), 1, warehouse);
        // The link's subject is the Freight; the row names its Warehouse.
        if let Some(name) = &freight.warehouse {
            hop.name = format!("Warehouse {name}");
            if let Shows::Hop(detail) = &mut hop.shows {
                detail.title = name.clone();
                detail
                    .fields
                    .retain(|field| field.label != "Namespace" && field.label != "Name");
                detail.fields.insert(
                    0,
                    Field {
                        label: "Name".into(),
                        value: Value::Mono(name.clone()),
                    },
                );
            }
        }
        hops.push(hop);
    }
    if let Some(why) = evidence.kargo.stages.why_not_read() {
        hops.push(unread_stages(place, &why));
    }

    let mut model = Vec::new();
    for (ix, stage) in stages.iter().enumerate() {
        let group = groups.len();
        let gates = gates::gates(freight, stage, &evidence.kargo.promotions);
        let stage_id = format!("{}/{}", stage.project, stage.name);
        let runs = stage_run(&links, &stage_id);
        let holds = stage.current_freight.contains(&freight.name);
        let cluster = deploys_to(place, evidence, holds, &runs);
        groups.push(Group {
            phase: Phase::Stage(ix),
            detail: cluster.words(),
        });
        for (gate, row) in [
            ("Eligible", &gates.eligible_row),
            ("Promotion", &gates.promotion_row),
            ("Verification", &gates.verification_row),
        ] {
            hops.push(gate_hop(place, &stage.name, group, ix, gate, row));
        }
        for (n, link) in runs.iter().skip(1).enumerate() {
            let key = format!("{}-{}-{n}", stage.name, kind_key(link.to));
            hops.push(link_hop(place, evidence, freight, key, group, link));
        }
        model.push(Stage {
            name: stage.name.clone(),
            cluster,
            state: gates.state,
            words: gates.words,
            running: (!holds)
                .then(|| stage.current_freight.first().cloned())
                .flatten(),
            eligible: gates.eligible,
            promotion: gates.promotion,
            approvers: None,
            promoters: None,
            steps: Vec::new(),
            verification: gates.verification,
            link: runs.first().map(|link| link_detail(link)),
            object: object(
                place,
                &ObjectRef::new(
                    kargo::GROUP,
                    "Stage",
                    Some(&stage.project),
                    &stage.name,
                    &stage.meta,
                ),
            ),
        });
    }

    Change {
        project: place.project.clone(),
        freight: freight
            .alias
            .clone()
            .unwrap_or_else(|| freight.name.clone()),
        kargo_cluster: place.label.clone(),
        observed_at: evidence.observed_at,
        groups,
        stages: model,
        hops,
    }
}

/// The Stages the Freight can reach, in promotion order: those that take
/// Freight from its Warehouse, then each Stage after the Stages it takes
/// Freight from. Stages of other Warehouses are left out; a Stage in a loop
/// comes last, by name.
fn reachable<'a>(freight: &Freight, evidence: &'a Evidence) -> Vec<&'a KargoStage> {
    let Some(stages) = evidence.kargo.stages.read() else {
        return Vec::new();
    };
    let mut ordered: Vec<&KargoStage> = Vec::new();
    let mut placed: BTreeSet<&str> = BTreeSet::new();
    let fed = |stage: &KargoStage| {
        stage.warehouses.is_empty()
            || freight
                .warehouse
                .as_ref()
                .is_none_or(|warehouse| stage.warehouses.contains(warehouse))
    };
    let mut sorted: Vec<&KargoStage> = stages.iter().filter(|stage| fed(stage)).collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    loop {
        let level: Vec<&KargoStage> = sorted
            .iter()
            .filter(|stage| !placed.contains(stage.name.as_str()))
            .filter(|stage| {
                stage
                    .upstream
                    .iter()
                    .all(|upstream| placed.contains(upstream.as_str()))
            })
            .copied()
            .collect();
        if level.is_empty() {
            break;
        }
        for stage in level {
            placed.insert(&stage.name);
            ordered.push(stage);
        }
    }
    ordered.extend(
        sorted
            .into_iter()
            .filter(|stage| !placed.contains(stage.name.as_str())),
    );
    ordered
}

/// A Stage's Freight → Stage link and every link after it, up to the next
/// Stage's. Empty when the Stage doesn't run the Freight.
fn stage_run<'a>(links: &'a [Link], stage_id: &str) -> Vec<&'a Link> {
    let Some(start) = links
        .iter()
        .position(|link| link.to == Joined::Stage && link.subject == stage_id)
    else {
        return Vec::new();
    };
    let mut run = vec![&links[start]];
    run.extend(
        links[start + 1..]
            .iter()
            .take_while(|link| link.to != Joined::Stage && link.to != Joined::Promotion)
            .filter(|link| link.from != Joined::Freight),
    );
    run
}

/// Where a Stage deploys: its first Application's destination, by the
/// cluster's name when it is this one. Unknown, with why, when its
/// Application wasn't found or its destination isn't matched to a cluster;
/// with no kubeconfig contexts to match, a server other than Argo CD's own
/// is never known to be this cluster.
fn deploys_to(place: &Place, evidence: &Evidence, holds: bool, run: &[&Link]) -> Destination {
    use Destination::Unknown as Not;
    if run.is_empty() {
        return Not(if holds {
            "its record wasn't joined to this Freight".into()
        } else {
            "it doesn't run this Freight, so its Application wasn't looked for".into()
        });
    }
    let Some(link) = run.iter().find(|link| link.to == Joined::Application) else {
        return Not("no Application was joined to it".into());
    };
    let namespace = &place.argocd_namespace;
    let Some(app) = evidence.applications.read().and_then(|apps| {
        apps.iter()
            .find(|app| format!("{}/{}", app.namespace, app.name) == link.subject)
    }) else {
        return Not(match evidence.applications.why_not_read() {
            Some(why) => format!("Argo CD Applications in {namespace} weren't read: {why}"),
            None => format!("no Argo CD Application in {namespace} was found for it"),
        });
    };
    let shown = |context: &str| {
        Destination::Cluster(if context == place.cluster {
            place.label.clone()
        } else {
            context.to_owned()
        })
    };
    match evidence.destinations.get(&link.subject) {
        Some(DestinationMatch::One(context) | DestinationMatch::ArgoCd(context)) => shown(context),
        Some(DestinationMatch::Named { context, .. }) => shown(context),
        Some(DestinationMatch::ByName(name)) => Not(format!(
            "Argo CD's cluster {name}, not known to be this one"
        )),
        Some(DestinationMatch::None) => Not(format!(
            "server {}, not known to be this cluster",
            app.destination_server.as_deref().unwrap_or("?")
        )),
        Some(DestinationMatch::Ambiguous(_)) => {
            Not("its destination matches more than one cluster".into())
        }
        Some(DestinationMatch::Unspecified) => {
            Not(format!("Application {} names no destination", app.name))
        }
        None => Not(format!(
            "Application {}'s destination wasn't matched",
            app.name
        )),
    }
}

fn kind_key(hop: Joined) -> &'static str {
    match hop {
        Joined::PullRequest => "pr",
        Joined::Commit => "commit",
        Joined::PipelineRun => "run",
        Joined::SupplyChain => "chain",
        Joined::Freight => "freight",
        Joined::Warehouse => "warehouse",
        Joined::Promotion => "promotion",
        Joined::Stage => "stage",
        Joined::Application => "argocd",
        Joined::Rollout => "rollout",
        Joined::Deployment => "deployment",
        Joined::Pod => "pods",
    }
}

/// The Kubernetes kind on a hop's side, as observations name it.
fn kind_of(hop: Joined) -> Option<&'static str> {
    Some(match hop {
        Joined::PipelineRun | Joined::SupplyChain => "PipelineRun",
        Joined::Freight => "Freight",
        Joined::Warehouse => "Warehouse",
        Joined::Promotion => "Promotion",
        Joined::Stage => "Stage",
        Joined::Application => "Application",
        Joined::Rollout => "Rollout",
        Joined::Deployment => "Deployment",
        Joined::Pod => "Pod",
        Joined::PullRequest | Joined::Commit => return None,
    })
}

/// A hop's caption in the Inspector.
fn caption(hop: Joined) -> &'static str {
    match hop {
        Joined::PullRequest => "Pull request",
        Joined::Commit => "Commit",
        Joined::PipelineRun => "PipelineRun",
        Joined::SupplyChain => "Supply chain",
        Joined::Freight => "Kargo Freight",
        Joined::Warehouse => "Kargo Warehouse",
        Joined::Promotion => "Kargo Promotion",
        Joined::Stage => "Kargo Stage",
        Joined::Application => "Argo CD Application",
        Joined::Rollout => "Argo Rollout",
        Joined::Deployment => "Deployment",
        Joined::Pod => "Pods",
    }
}

/// The name in a `namespace/name` subject.
fn name_of(subject: &str) -> &str {
    subject.rsplit('/').next().unwrap_or(subject)
}

fn row_name(link: &Link) -> String {
    if link.subject == "-" {
        return caption(link.to).to_owned();
    }
    let name = name_of(&link.subject);
    match link.to {
        Joined::PipelineRun => format!("Run {name}"),
        Joined::SupplyChain => "Supply chain".into(),
        Joined::Warehouse => format!("Warehouse {name}"),
        Joined::Application => name.to_owned(),
        Joined::Rollout => format!("Rollout {name}"),
        Joined::Deployment => format!("Deployment {name}"),
        Joined::Pod => "Pods".into(),
        other => format!("{} {name}", other.word()),
    }
}

/// An object to open, on this cluster, in the version the app reads it.
fn object(place: &Place, object: &ObjectRef) -> Object {
    let version = match object.group.as_str() {
        "" | "apps" => "v1",
        "tekton.dev" => "v1",
        _ => "v1alpha1",
    };
    Object {
        cluster: place.cluster.clone(),
        group: object.group.clone(),
        version: version.into(),
        kind: object.kind.clone(),
        plural: format!("{}s", object.kind.to_lowercase()),
        namespace: object.namespace.clone().unwrap_or_default(),
        name: object.name.clone(),
    }
}

/// What a side of a link says: each observation of it, one clause each.
fn says(evidence: &[&Observation]) -> String {
    if evidence.is_empty() {
        return "nothing was read".into();
    }
    evidence
        .iter()
        .map(|seen| {
            let field = seen.field.rsplit('/').next().unwrap_or(&seen.field);
            let field = field.replace("~1", "/").replace("~0", "~");
            match &seen.value {
                Some(value) => format!("{field} = {}", short(value)),
                None => field,
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A digest or commit as the page shows them: its first twelve hex digits.
fn short(value: &str) -> String {
    let hex = |text: &str| text.len() >= 40 && text.bytes().all(|b| b.is_ascii_hexdigit());
    match value.strip_prefix("sha256:") {
        Some(digest) if hex(digest) => format!("sha256:{}", &digest[..12]),
        _ if hex(value) => value[..12].to_owned(),
        _ => value.to_owned(),
    }
}

fn key_word(key: &Key) -> &'static str {
    match key {
        Key::Sha(_) => "commit",
        Key::Digest(_) => "digest",
        Key::Name(_) => "name",
        Key::None => "nothing",
    }
}

fn link_detail(link: &Link) -> LinkDetail {
    let before_kind = kind_of(link.from);
    let (before, here): (Vec<&Observation>, Vec<&Observation>) = link
        .evidence
        .iter()
        .partition(|seen| Some(seen.object.kind.as_str()) == before_kind);
    let before_says = match (&link.key, before.is_empty()) {
        (Key::Sha(sha), true) => format!("commit {}", short(sha)),
        _ => says(&before),
    };
    LinkDetail {
        confidence: link.confidence,
        by: key_word(&link.key).into(),
        before: sentence(link.from.word()).trim_end_matches('.').into(),
        before_says,
        here_says: says(&here),
        why: sentence(&link.reason),
    }
}

/// `text` as a sentence: a capital first and a full stop last.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out: String = first.to_uppercase().chain(chars).collect();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// The state of what a link leads to, as far as what was read says.
fn state_of(evidence: &Evidence, freight: &Freight, link: &Link) -> (HealthIndicator, String) {
    if link.confidence == Confidence::Unknown {
        return (Unknown, "Unknown".into());
    }
    let id = |namespace: &str, name: &str| format!("{namespace}/{name}") == link.subject;
    match link.to {
        Joined::PipelineRun => {
            let run = evidence
                .builds
                .read()
                .and_then(|builds| builds.iter().find(|b| id(&b.run.namespace, &b.run.name)));
            match run.map(|build| build.run.succeeded.as_deref()) {
                Some(Some("True")) => (Healthy, "Succeeded".into()),
                Some(Some("False")) => (Error, "Failed".into()),
                Some(Some("Unknown")) => (Pending, "Running".into()),
                Some(_) => (Unknown, "No condition reported".into()),
                None => (Unknown, "Not read".into()),
            }
        }
        Joined::Application => {
            let app = evidence
                .applications
                .read()
                .and_then(|apps| apps.iter().find(|app| id(&app.namespace, &app.name)));
            let sync = app.and_then(|app| app.sync.clone());
            let health = app.and_then(|app| app.health.clone());
            let words = [sync.as_deref(), health.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let state = match (sync.as_deref(), health.as_deref()) {
                (None, None) => return (Unknown, "No sync or health reported".into()),
                (_, Some("Degraded" | "Missing")) => Warning,
                (Some("OutOfSync"), _) => Warning,
                (_, Some("Progressing" | "Suspended")) => Info,
                (_, Some("Healthy")) => Healthy,
                _ => Unknown,
            };
            (state, words)
        }
        Joined::Rollout => {
            let rollout = evidence
                .rollouts
                .read()
                .and_then(|all| all.iter().find(|r| id(&r.namespace, &r.name)));
            let Some(rollout) = rollout else {
                return (Unknown, "Not read".into());
            };
            if rollout.aborted {
                return (Error, "Aborted".into());
            }
            if rollout.paused {
                return (Info, "Paused".into());
            }
            match rollout.phase.as_deref() {
                Some("Healthy") => (Healthy, "Healthy".into()),
                Some("Progressing") => (Pending, "Progressing".into()),
                Some("Paused") => (Info, "Paused".into()),
                Some("Degraded") => (Error, "Degraded".into()),
                Some(phase) => (Unknown, phase.to_owned()),
                None => (Unknown, "No phase reported".into()),
            }
        }
        Joined::Deployment => {
            let deployment = evidence
                .deployments
                .read()
                .and_then(|all| all.iter().find(|d| id(&d.namespace, &d.name)));
            let Some(d) = deployment else {
                return (Unknown, "Not read".into());
            };
            let behind = matches!(
                (d.generation, d.observed_generation),
                (Some(wanted), Some(seen)) if seen < wanted
            );
            if behind {
                (Pending, "Progressing".into())
            } else if d.paused {
                (Info, "Paused".into())
            } else if d.updated_replicas < d.replicas {
                (
                    Pending,
                    format!("{} of {} updated", d.updated_replicas, d.replicas),
                )
            } else if d.available_replicas < d.replicas {
                (
                    Warning,
                    format!("{} of {} available", d.available_replicas, d.replicas),
                )
            } else {
                (Healthy, "Available".into())
            }
        }
        Joined::Pod => pods_state(evidence, freight, link),
        _ => {
            let mut word = link.confidence.word().to_owned();
            word[..1].make_ascii_uppercase();
            (Healthy, word)
        }
    }
}

/// The pods' state: how many of the containers that run the Freight's
/// digest are ready, of those the link names.
fn pods_state(evidence: &Evidence, freight: &Freight, link: &Link) -> (HealthIndicator, String) {
    let digests: BTreeSet<&str> = freight
        .images
        .iter()
        .filter_map(|image| image.digest.as_ref().map(|digest| digest.as_str()))
        .collect();
    let named: BTreeSet<(Option<&str>, &str)> = link
        .evidence
        .iter()
        .filter(|seen| seen.object.kind == "Pod")
        .filter(|seen| seen.value.as_deref().is_some_and(|v| digests.contains(v)))
        .map(|seen| (seen.object.namespace.as_deref(), seen.object.name.as_str()))
        .collect();
    if named.is_empty() {
        return if link.evidence.iter().any(|seen| seen.object.kind == "Pod") {
            (Warning, "Another digest".into())
        } else {
            (Unknown, "No pods reported".into())
        };
    }
    let read = [
        evidence.pods.get(&link.subject).map(|read| &read.pods),
        evidence
            .deployment_pods
            .get(&link.subject)
            .map(|read| &read.pods),
        evidence.namespace_pods.get(&link.subject),
    ];
    let containers: Vec<_> = read
        .into_iter()
        .flatten()
        .filter_map(|source| source.read())
        .flatten()
        .filter(|pod| named.contains(&(pod.namespace.as_deref(), pod.pod.as_str())))
        .filter(|pod| {
            pod.digest
                .as_ref()
                .is_some_and(|digest| digests.contains(digest.as_str()))
        })
        .collect();
    let ready = containers.iter().filter(|pod| pod.ready).count();
    match (ready, containers.len()) {
        (_, 0) => (Unknown, "No pods reported".into()),
        (0, _) => (Warning, "None ready".into()),
        (ready, all) if ready == all => (Healthy, "Ready".into()),
        (ready, all) => (Pending, format!("{ready} of {all} ready")),
    }
}

/// When a link's object finished, when it records it.
fn time_of(evidence: &Evidence, link: &Link) -> Option<DateTime<Utc>> {
    if link.to != Joined::PipelineRun {
        return None;
    }
    evidence.builds.read().and_then(|builds| {
        builds
            .iter()
            .find(|build| format!("{}/{}", build.run.namespace, build.run.name) == link.subject)
            .and_then(|build| instant(build.run.completed.as_deref()))
    })
}

fn link_hop(
    place: &Place,
    evidence: &Evidence,
    freight: &Freight,
    key: String,
    group: usize,
    link: &Link,
) -> Hop {
    let (state, words) = state_of(evidence, freight, link);
    let to_kind = kind_of(link.to);
    // An Unknown link to an object it never found names the side it came
    // from: the row says what wasn't found, and shows that side as context.
    let missing = link.confidence == Confidence::Unknown
        && link.subject != "-"
        && matches!(
            link.to,
            Joined::Application | Joined::Rollout | Joined::Deployment
        )
        && !link
            .evidence
            .iter()
            .any(|seen| Some(seen.object.kind.as_str()) == to_kind);
    let mut fields = Vec::new();
    if missing {
        fields.push(Field {
            label: caption(link.from).into(),
            value: Value::Mono(link.subject.clone()),
        });
    } else if link.subject != "-" {
        let (namespace, name) = link
            .subject
            .rsplit_once('/')
            .unwrap_or(("", link.subject.as_str()));
        if !namespace.is_empty() {
            fields.push(Field {
                label: "Namespace".into(),
                value: Value::Mono(namespace.into()),
            });
        }
        fields.push(Field {
            label: "Name".into(),
            value: Value::Mono(name.into()),
        });
    }
    if link.key != Key::None {
        fields.push(Field {
            label: "Joined on".into(),
            value: Value::Mono(link.key.to_string()),
        });
    }
    fields.push(Field {
        label: "Read from".into(),
        value: Value::Mono(place.label.clone()),
    });
    let actions = link
        .evidence
        .iter()
        .find(|seen| Some(seen.object.kind.as_str()) == to_kind)
        .map(|seen| {
            Action::new(Target::Resource {
                what: format!("the {}", seen.object.kind),
                object: object(place, &seen.object),
            })
        })
        .into_iter()
        .collect();
    let notice = (link.confidence == Confidence::Unknown).then(|| Notice {
        state: Warning,
        lead: "Not joined.".into(),
        body: sentence(&link.reason),
    });
    Hop {
        key,
        group,
        state,
        name: if missing {
            format!("{} not found", link.to.word())
        } else {
            row_name(link)
        },
        detail: link.reason.clone(),
        from: place.label.clone(),
        at: time_of(evidence, link),
        link: Some(link.confidence),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: caption(link.to).into(),
            title: if missing {
                format!("{} not found", link.to.word())
            } else if link.subject == "-" || link.to == Joined::Pod {
                caption(link.to).into()
            } else {
                name_of(&link.subject).into()
            },
            state: words,
            notice,
            fields,
            link: Some(link_detail(link)),
            unlinked: None,
            actions,
        })),
    }
}

/// The builds of the Freight's commit: every link the join made before it
/// reached a Freight, one hop each.
fn build_hops(
    place: &Place,
    evidence: &Evidence,
    freight: &Freight,
    trail: Option<&join::Trail>,
) -> Vec<Hop> {
    let Some(trail) = trail else {
        return vec![Hop {
            key: "build".into(),
            group: 0,
            state: Unknown,
            name: "Builds".into(),
            detail: "The Freight names no commit or image revision, so no build is looked for"
                .into(),
            from: place.label.clone(),
            at: None,
            link: Some(Confidence::Unknown),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Builds".into(),
                title: "Builds".into(),
                state: "Unknown".into(),
                notice: Some(Notice {
                    state: Warning,
                    lead: "Not joined.".into(),
                    body: "The Freight names no commit and no image revision, and builds \
                           are found by commit only."
                        .into(),
                }),
                fields: Vec::new(),
                link: None,
                unlinked: None,
                actions: Vec::new(),
            })),
        }];
    };
    trail
        .links
        .iter()
        .take_while(|link| link.to != Joined::Freight)
        .enumerate()
        .map(|(n, link)| {
            link_hop(
                place,
                evidence,
                freight,
                format!("build-{}-{n}", kind_key(link.to)),
                0,
                link,
            )
        })
        .collect()
}

/// The Freight itself, joined to its build by the link the join made into
/// it, when it made one.
fn freight_hop(
    place: &Place,
    evidence: &Evidence,
    freight: &Freight,
    trail: Option<&join::Trail>,
) -> Hop {
    let subject = format!("{}/{}", freight.project, freight.name);
    let into = trail.and_then(|trail| {
        trail
            .links
            .iter()
            .find(|link| link.to == Joined::Freight && link.subject == subject)
            .or_else(|| {
                // The join's general word that no Freight matched speaks of
                // every Freight; this one says why itself.
                trail.links.iter().find(|link| {
                    link.to == Joined::Freight
                        && link.subject == "-"
                        && link.confidence != Confidence::Unknown
                })
            })
    });
    let mut fields = vec![Field {
        label: "Name".into(),
        value: Value::Mono(freight.name.clone()),
    }];
    if let Some(alias) = &freight.alias {
        fields.push(Field {
            label: "Alias".into(),
            value: Value::Mono(alias.clone()),
        });
    }
    if let Some(warehouse) = &freight.warehouse {
        fields.push(Field {
            label: "Warehouse".into(),
            value: Value::Mono(warehouse.clone()),
        });
    }
    for commit in &freight.commits {
        fields.push(Field {
            label: "Commit".into(),
            value: Value::Mono(commit.id.chars().take(12).collect()),
        });
    }
    for image in &freight.images {
        let shown = match (&image.digest, &image.tag) {
            (Some(digest), _) => format!("{} {}", image.repo_url, digest.short()),
            (None, Some(tag)) => format!("{}:{tag} (tag, shown only)", image.repo_url),
            (None, None) => image.repo_url.clone(),
        };
        fields.push(Field {
            label: "Image".into(),
            value: Value::Mono(shown),
        });
    }
    let (state, words) = match into.map(|link| link.confidence) {
        Some(Confidence::Unknown) | None => (Unknown, "Not joined to a build"),
        Some(Confidence::Claimed) => (Healthy, "Claimed"),
        Some(Confidence::Confirmed) => (Healthy, "Confirmed"),
    };
    let title = freight
        .alias
        .clone()
        .unwrap_or_else(|| freight.name.clone());
    Hop {
        key: "freight".into(),
        group: 1,
        state,
        name: format!("Freight {title}"),
        detail: into.map_or_else(
            || format!("Followed from its Stage; {}", unjoined(evidence, freight)),
            |link| link.reason.clone(),
        ),
        from: place.label.clone(),
        at: None,
        link: into.map(|link| link.confidence),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Kargo Freight".into(),
            title,
            state: words.into(),
            notice: None,
            fields,
            link: into.map(link_detail),
            unlinked: into.is_none().then(|| {
                sentence(&format!(
                    "Followed from its Stage: {}",
                    unjoined(evidence, freight)
                ))
            }),
            actions: vec![Action::new(Target::Resource {
                what: "the Freight".into(),
                object: object(
                    place,
                    &ObjectRef::new(
                        kargo::GROUP,
                        "Freight",
                        Some(&freight.project),
                        &freight.name,
                        &freight.meta,
                    ),
                ),
            })],
        })),
    }
}

/// The row that says the project's Stages weren't read, so none is shown.
fn unread_stages(place: &Place, why: &str) -> Hop {
    let body = sentence(&format!("The project's Stages weren't read: {why}"));
    Hop {
        key: "stages".into(),
        group: 1,
        state: Unknown,
        name: "Stages".into(),
        detail: format!("Not read: {why}"),
        from: place.label.clone(),
        at: None,
        link: Some(Confidence::Unknown),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Kargo Stages".into(),
            title: "Stages".into(),
            state: "Unknown".into(),
            notice: Some(Notice {
                state: Warning,
                lead: "Not read.".into(),
                body,
            }),
            fields: Vec::new(),
            link: None,
            unlinked: None,
            actions: Vec::new(),
        })),
    }
}

/// Why no build leads to the Freight, when the join made no link into it.
fn unjoined(evidence: &Evidence, freight: &Freight) -> String {
    if evidence.sha.is_empty() {
        return "the Freight names no commit or image revision, so no build was looked for".into();
    }
    let commit = short(&evidence.sha);
    let from = if freight.commits.is_empty() {
        format!("commit {commit} (from an image's revision annotation)")
    } else {
        format!("commit {commit}")
    };
    match &evidence.builds {
        source if source.read().is_some_and(Vec::is_empty) => {
            format!("no PipelineRun of {from} was found")
        }
        source if source.read().is_some() => {
            format!("builds of {from} were read, but none was joined to it")
        }
        source => format!(
            "builds of {from} weren't read: {}",
            source.why_not_read().unwrap_or_default()
        ),
    }
}

fn gate_hop(place: &Place, stage: &str, group: usize, ix: usize, gate: &str, row: &Row) -> Hop {
    Hop {
        key: format!("{stage}-{}", gate.to_lowercase()),
        group,
        state: row.state,
        name: gate.into(),
        detail: row.detail.clone(),
        from: place.label.clone(),
        at: row.at,
        link: None,
        shows: Shows::Stage(ix),
    }
}
