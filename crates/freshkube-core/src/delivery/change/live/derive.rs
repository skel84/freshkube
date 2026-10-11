//! What was read, as the page's [`Change`]: the join's links in trail order,
//! one hop each, grouped Build, Freight, then each Stage the Freight can
//! reach in promotion order, with that Stage's three gates first.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::super::*;
use super::gates::{self, Row, instant};
use super::links;
use super::pages::Pages;
use super::{At, Place, Reached};
use crate::delivery::argocd::{Application, DestinationMatch, ManagedObject};
use crate::delivery::deployments::current_set;
use crate::delivery::join::{
    self, Confidence, Evidence, Hop as Joined, Key, Link, Workloads, freight_links,
};
use crate::delivery::kargo::{self, Freight, Stage as KargoStage};
use crate::delivery::observation::{ObjectRef, Observation};
use crate::delivery::source::{Source, cap_note};
use crate::indicators::HealthIndicator::{self, *};
use crate::workspace::Part;

/// Where each part of the change was read, and, for one Stage, where it
/// deploys and what was read there.
#[derive(Clone)]
struct Sides<'a> {
    builds: At,
    kargo: At,
    argocd: At,
    /// The Stage's destination, when it is known.
    environment: Option<At>,
    /// What was read there, when it was.
    workloads: Option<&'a Workloads>,
}

impl<'a> Sides<'a> {
    fn of(place: &Place) -> Self {
        Self {
            builds: place.at(Part::Builds),
            kargo: place.at(Part::Kargo),
            argocd: place.at(Part::ArgoCd),
            environment: None,
            workloads: None,
        }
    }

    /// For the Stage whose run is `run`: its Application's destination.
    fn stage(&self, place: &Place, evidence: &'a Evidence, run: &[&Link]) -> Self {
        let context = run
            .iter()
            .find(|link| link.to == Joined::Application)
            .and_then(|link| evidence.destination_of(&link.subject));
        Self {
            environment: context.map(|context| place.at_context(context)),
            workloads: context
                .and_then(|context| evidence.environments.get(context))
                .and_then(|read| read.as_ref().ok()),
            ..self.clone()
        }
    }

    /// Where the side of a link `hop` names was read.
    fn at(&self, hop: Joined) -> &At {
        match hop {
            Joined::PullRequest | Joined::Commit | Joined::PipelineRun | Joined::SupplyChain => {
                &self.builds
            }
            Joined::Freight | Joined::Warehouse | Joined::Promotion | Joined::Stage => &self.kargo,
            Joined::Application => &self.argocd,
            Joined::Rollout | Joined::Deployment | Joined::Pod => {
                self.environment.as_ref().unwrap_or(&self.argocd)
            }
        }
    }
}

/// The change `place` names, from what was read and the join's links.
/// `freight` is the Freight as first read; the one the project's list
/// holds wins, as the join used it. `reached` says which parts' entries
/// weren't read at all.
pub fn derive(
    place: &Place,
    evidence: &Evidence,
    freight: &Freight,
    pages: &Pages,
    reached: &Reached,
) -> Change {
    let freight = evidence
        .kargo
        .freight
        .read()
        .and_then(|all| all.iter().find(|item| item.name == freight.name))
        .unwrap_or(freight);
    let trail = (!evidence.sha.is_empty()).then(|| join::join(evidence));
    let links = freight_links(evidence, freight);
    let stages = reachable(freight, evidence);
    let sides = Sides::of(place);

    let mut groups = vec![
        Group {
            phase: Phase::Build,
            detail: sides.builds.on("Tekton"),
        },
        Group {
            phase: Phase::Freight,
            detail: sides.kargo.on("Kargo"),
        },
    ];
    let mut hops = build_hops(&sides, evidence, freight, pages, trail.as_ref());
    hops.push(freight_hop(
        &sides,
        evidence,
        freight,
        pages,
        trail.as_ref(),
    ));
    if let Some(warehouse) = links.iter().find(|link| link.to == Joined::Warehouse) {
        let mut hop = link_hop(
            &sides,
            evidence,
            freight,
            pages,
            "warehouse".into(),
            1,
            warehouse,
            &[],
        );
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
        hops.push(unread_stages(&sides, &why));
    }

    let mut model = Vec::new();
    for (ix, stage) in stages.iter().enumerate() {
        let group = groups.len();
        let gates = gates::gates(freight, stage, &evidence.kargo.promotions);
        let stage_id = format!("{}/{}", stage.project, stage.name);
        let runs = stage_run(&links, &stage_id);
        let holds = stage.current_freight.contains(&freight.name);
        let at = sides.stage(place, evidence, &runs);
        let cluster = deploys_to(place, evidence, holds, &runs);
        let reach = reach(place, evidence, holds, &runs);
        groups.push(Group {
            phase: Phase::Stage(ix),
            detail: cluster.words(),
        });
        for (gate, row) in [
            ("Eligible", &gates.eligible_row),
            ("Promotion", &gates.promotion_row),
            ("Verification", &gates.verification_row),
        ] {
            hops.push(gate_hop(&sides, &stage.name, group, ix, gate, row));
        }
        for (n, link) in runs.iter().skip(1).enumerate() {
            // An Application mapped to another workspace cluster: in place
            // of its links into that cluster, which nothing read, the
            // workloads Argo CD reports there.
            let reported = elsewhere(place, evidence, &link.subject)
                .map(|away| reported_hops(&sides, &stage.name, group, away))
                .unwrap_or_default();
            if link.from == Joined::Application && !reported.is_empty() {
                continue;
            }
            let key = format!("{}-{}-{n}", stage.name, kind_key(link.to));
            hops.push(link_hop(
                &at, evidence, freight, pages, key, group, link, &runs,
            ));
            if link.to == Joined::Application {
                hops.extend(reported);
            }
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
            reach,
            object: object(
                &sides.kargo,
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
        kargo_cluster: sides.kargo.label.clone(),
        observed_at: evidence.observed_at,
        page: links::freight_page(pages, freight),
        origin: origin(evidence, freight),
        missing: missing(place, evidence, &sides, reached),
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
    let fed = |stage: &&KargoStage| {
        stage.warehouses.is_empty()
            || freight
                .warehouse
                .as_ref()
                .is_none_or(|warehouse| stage.warehouses.contains(warehouse))
    };
    kargo::promotion_order(stages.iter().filter(fed))
}

/// What the change is: the pull request a build of its commit names, its
/// commit, and the commit's title from that build, else from the Freight.
fn origin(evidence: &Evidence, freight: &Freight) -> Origin {
    let commit = super::commit_of(freight).map(str::to_owned);
    let runs = || {
        evidence
            .builds
            .read()
            .into_iter()
            .flatten()
            .map(|build| &build.run)
            .filter(|run| commit.is_some() && run.sha == commit)
    };
    Origin {
        pull_request: runs().find_map(|run| run.pull_request),
        title: runs().find_map(|run| run.title.clone()).or_else(|| {
            freight
                .commits
                .first()
                .and_then(|commit| commit.subject())
                .map(str::to_owned)
        }),
        commit,
    }
}

/// How sure a link is, to find the weakest.
fn sureness(confidence: Confidence) -> u8 {
    match confidence {
        Confidence::Unknown => 0,
        Confidence::Claimed => 1,
        Confidence::Confirmed => 2,
    }
}

/// How sure it is that a Stage's pods run the Freight: the weakest link of
/// its run, and Unknown when no link reaches its pods. An Application on
/// another workspace entry, whose workloads Argo CD reports, stands for its
/// links into that cluster, as the page shows it: Claimed at most, since
/// nothing there was read. Unknown because a source the run needed was
/// refused is Refused, naming what.
pub(super) fn reach(place: &Place, evidence: &Evidence, holds: bool, run: &[&Link]) -> Reach {
    if !holds {
        return Reach::NotReached;
    }
    let at = Sides::of(place).stage(place, evidence, run);
    let reported = |link: &&Link| {
        link.from == Joined::Application
            && elsewhere(place, evidence, &link.subject).is_some_and(|away| reports(away.app))
    };
    let kept: Vec<&Link> = run.iter().copied().filter(|link| !reported(link)).collect();
    let away = kept.len() < run.len();
    let pods = kept.iter().any(|link| link.to == Joined::Pod);
    // An Application read here still needs its pods, beside one elsewhere.
    let local = kept.iter().any(|link| {
        (link.from == Joined::Application || link.to == Joined::Application)
            && elsewhere(place, evidence, &link.subject).is_none()
    });
    let weakest = kept
        .iter()
        .map(|link| link.confidence)
        .min_by_key(|confidence| sureness(*confidence));
    let confidence = match weakest {
        Some(weakest) if away && (pods || !local) => [weakest, Confidence::Claimed]
            .into_iter()
            .min_by_key(|confidence| sureness(*confidence))
            .unwrap_or(Confidence::Unknown),
        Some(weakest) if pods => weakest,
        _ => Confidence::Unknown,
    };
    if confidence == Confidence::Unknown {
        let kinds = refused(evidence, at.workloads, &kept);
        if !kinds.is_empty() {
            return Reach::Refused(kinds.join(", "));
        }
    }
    Reach::Linked(confidence)
}

/// Whether Argo CD reports a workload for the Application, which the page
/// lists in place of its links ([`reported_hops`]).
fn reports(app: &Application) -> bool {
    app.managed.iter().any(|object| {
        WORKLOADS
            .iter()
            .any(|(group, kind)| object.group == *group && object.kind == *kind)
    })
}

/// The kinds a Stage's run needed that the cluster refused: Argo CD's
/// Applications, which every run goes through, and the workloads and pods
/// its own links reach.
fn refused(evidence: &Evidence, workloads: Option<&Workloads>, run: &[&Link]) -> Vec<&'static str> {
    fn no<T>(source: &Source<T>) -> bool {
        matches!(source, Source::Refused(_))
    }
    let pods = |subject: &str| {
        workloads.is_some_and(|read| {
            let rollout = read.pods.get(subject).is_some_and(|pods| no(&pods.pods));
            let deployment = read
                .deployment_pods
                .get(subject)
                .is_some_and(|pods| no(&pods.pods));
            let namespace = read.namespace_pods.get(subject).is_some_and(no);
            rollout || deployment || namespace
        })
    };
    let touches = |kind: Joined| run.iter().any(|link| link.from == kind || link.to == kind);
    let read = |refused: fn(&Workloads) -> bool| workloads.is_some_and(refused);
    let checks = [
        ("Argo CD Applications", no(&evidence.applications)),
        (
            "Rollouts",
            touches(Joined::Rollout) && read(|w| no(&w.rollouts) || no(&w.replica_sets)),
        ),
        (
            "Deployments",
            touches(Joined::Deployment) && read(|w| no(&w.deployments) || no(&w.deployment_sets)),
        ),
        ("Pods", run.iter().any(|link| pods(&link.subject))),
    ];
    checks
        .into_iter()
        .filter(|(_, refused)| *refused)
        .map(|(kind, _)| kind)
        .collect()
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
/// cluster's name when it is this one, or the workspace cluster the person
/// mapped it to. Unknown, with why, when its Application wasn't found or
/// its destination isn't mapped; kubeconfig servers are never compared, so
/// a server other than Argo CD's own is known only through the mapping.
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
            None => format!(
                "no Argo CD Application in {namespace} was found for it{}",
                cap_note(evidence.applications.capped())
            ),
        });
    };
    let shown = |context: &str| {
        if place.is_here(context) {
            Destination::Cluster(place.label.clone())
        } else if matches!(evidence.environments.get(context), Some(Ok(_))) {
            // Read there, through a client of its own.
            Destination::Cluster(context.to_owned())
        } else if !place.mapping.lists(context) {
            Not(format!(
                "mapped to {context}, which the workspace no longer lists"
            ))
        } else {
            Destination::Entry {
                entry: context.to_owned(),
                via: match (&app.destination_server, &app.destination_name) {
                    (Some(server), _) => format!("server {server}"),
                    (None, Some(name)) => format!("name {name}"),
                    (None, None) => String::new(),
                },
            }
        }
    };
    match evidence.destinations.get(&link.subject) {
        Some(DestinationMatch::One(context) | DestinationMatch::ArgoCd(context)) => shown(context),
        Some(DestinationMatch::Named { context, .. }) => shown(context),
        Some(DestinationMatch::ByName(name)) => Not(format!(
            "Argo CD's cluster {name} isn't mapped to a workspace cluster"
        )),
        Some(DestinationMatch::None) => Not(format!(
            "server {} isn't mapped to a workspace cluster",
            app.destination_server.as_deref().unwrap_or("?")
        )),
        Some(DestinationMatch::Ambiguous(entries)) => {
            let entries: Vec<&str> = entries
                .iter()
                .map(|entry| {
                    if place.is_here(entry) {
                        place.label.as_str()
                    } else {
                        entry.as_str()
                    }
                })
                .collect();
            Not(format!(
                "its destination is mapped to more than one workspace cluster ({})",
                entries.join(", ")
            ))
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

/// An Application mapped to another workspace cluster that wasn't read.
struct Away<'a> {
    app: &'a Application,
    entry: &'a str,
    /// Why it wasn't read, when its entry was asked; else it isn't the open
    /// cluster, and only the open cluster is read.
    why: Option<&'a str>,
}

/// The Application `app_id` names and the workspace cluster it is mapped
/// to, when that is a cluster the workspace lists other than this one, and
/// nothing was read there.
fn elsewhere<'a>(place: &Place, evidence: &'a Evidence, app_id: &str) -> Option<Away<'a>> {
    let entry = evidence.destination_of(app_id)?;
    if place.is_here(entry) || !place.mapping.lists(entry) {
        return None;
    }
    let why = match evidence.environments.get(entry) {
        Some(Ok(_)) => return None,
        Some(Err(why)) => Some(why.as_str()),
        None => None,
    };
    let app = evidence
        .applications
        .read()?
        .iter()
        .find(|app| format!("{}/{}", app.namespace, app.name) == app_id)?;
    Some(Away { app, entry, why })
}

/// The workload kinds Argo CD may report for an Application, by group.
const WORKLOADS: [(&str, &str); 4] = [
    ("argoproj.io", "Rollout"),
    ("apps", "Deployment"),
    ("apps", "StatefulSet"),
    ("apps", "DaemonSet"),
];

/// The workloads Argo CD reports for an Application on another workspace
/// cluster, from its `status.resources`: what Argo CD says, since that
/// cluster wasn't read, so each link is Argo CD's claim. Each opens on that
/// cluster.
fn reported_hops(sides: &Sides, stage: &str, group: usize, away: Away) -> Vec<Hop> {
    let app = away.app;
    app.managed
        .iter()
        .filter(|object| {
            WORKLOADS
                .iter()
                .any(|(g, kind)| object.group == *g && object.kind == *kind)
        })
        .enumerate()
        .map(|(n, object)| {
            reported_hop(
                sides,
                format!("{stage}-reported-{}-{n}", app.name),
                group,
                &away,
                object,
            )
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
        key,
        group,
        state,
        name: format!("{} {}", object.kind, object.name),
        detail: format!("Argo CD reports it on {entry}; not read there"),
        from: sides.argocd.label.clone(),
        at: None,
        link: Some(Confidence::Claimed),
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
                        "{entry} isn't the open cluster, so this is what Argo CD reports, not what was read there."
                    ),
                },
            }),
            fields,
            link: Some(LinkDetail {
                confidence: Confidence::Claimed,
                by: "name".into(),
                before: "Application".into(),
                before_says: format!("status.resources lists {} {shown}", object.kind),
                here_says: "not read".into(),
                why: format!(
                    "Only Argo CD's status says so: {entry} isn't read, so nothing there confirms it."
                ),
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

/// An object to open, on the cluster it was read on, in the version the
/// app reads it.
fn object(at: &At, object: &ObjectRef) -> Object {
    let version = match object.group.as_str() {
        "" | "apps" => "v1",
        "tekton.dev" => "v1",
        _ => "v1alpha1",
    };
    Object {
        cluster: at.cluster.clone(),
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
fn state_of(
    evidence: &Evidence,
    workloads: Option<&Workloads>,
    freight: &Freight,
    link: &Link,
) -> (HealthIndicator, String) {
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
            let rollout = workloads
                .and_then(|read| read.rollouts.read())
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
            let deployment = workloads
                .and_then(|read| read.deployments.read())
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
        Joined::Pod => pods_state(workloads, freight, link),
        _ => {
            let mut word = link.confidence.word().to_owned();
            word[..1].make_ascii_uppercase();
            (Healthy, word)
        }
    }
}

/// The pods' state: how many of the containers that run the Freight's
/// digest are ready, of those the link names.
fn pods_state(
    workloads: Option<&Workloads>,
    freight: &Freight,
    link: &Link,
) -> (HealthIndicator, String) {
    let digests: BTreeSet<&str> = freight
        .images
        .iter()
        .filter_map(|image| image.digest.as_ref().map(|digest| digest.as_str()))
        .collect();
    if digests.is_empty() {
        return (Unknown, "The Freight names no image digest".into());
    }
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
    let read = workloads.map_or([None; 3], |workloads| {
        [
            workloads.pods.get(&link.subject).map(|read| &read.pods),
            workloads
                .deployment_pods
                .get(&link.subject)
                .map(|read| &read.pods),
            workloads.namespace_pods.get(&link.subject),
        ]
    });
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

/// The row of `link`, one of `run`: the links of its Stage, or none for a
/// build's or the Warehouse's.
#[allow(clippy::too_many_arguments)]
fn link_hop(
    sides: &Sides,
    evidence: &Evidence,
    freight: &Freight,
    pages: &Pages,
    key: String,
    group: usize,
    link: &Link,
    run: &[&Link],
) -> Hop {
    let at = sides.at(link.to);
    let (state, words) = state_of(evidence, sides.workloads, freight, link);
    let to_kind = kind_of(link.to);
    // An Unknown link to an object it never found names the side it came
    // from: the row says what wasn't found, or wasn't read when its list was
    // refused or failed, and shows that side as context.
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
    let listed = match link.to {
        Joined::Application => evidence.applications.read().is_some(),
        Joined::Rollout => sides
            .workloads
            .is_some_and(|read| read.rollouts.read().is_some()),
        Joined::Deployment => sides
            .workloads
            .is_some_and(|read| read.deployments.read().is_some()),
        _ => true,
    };
    let missing_words = format!(
        "{} {}",
        link.to.word(),
        if listed { "not found" } else { "not read" }
    );
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
    let revision = (!missing)
        .then(|| revision(at, sides.workloads, link, run))
        .flatten();
    if let Some(Action {
        target: Target::Revision {
            hash: Some(hash), ..
        },
        ..
    }) = &revision
    {
        fields.push(Field {
            label: "Pod template hash".into(),
            value: Value::Mono(hash.clone()),
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
        value: Value::Mono(at.label.clone()),
    });
    let mut actions: Vec<Action> = links::actions(&sides.builds, evidence, pages, link);
    actions.extend(
        link.evidence
            .iter()
            .find(|seen| Some(seen.object.kind.as_str()) == to_kind)
            .map(|seen| {
                Action::new(Target::Resource {
                    what: format!("the {}", seen.object.kind),
                    object: object(at, &seen.object),
                })
            }),
    );
    actions.extend(revision);
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
            missing_words.clone()
        } else {
            row_name(link)
        },
        detail: link.reason.clone(),
        from: at.label.clone(),
        at: time_of(evidence, link),
        link: Some(link.confidence),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: caption(link.to).into(),
            title: if missing {
                missing_words
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

/// Coroot's revision of the workload a link leads to: a Deployment's is
/// the one its current ReplicaSet's pod-template hash names. It is offered
/// only when the pods link in `run` confirms that revision runs the
/// Freight, since otherwise it may be another change. Coroot keeps
/// revisions of Deployments only, so a Rollout's says so.
fn revision(at: &At, workloads: Option<&Workloads>, link: &Link, run: &[&Link]) -> Option<Action> {
    let seen = link
        .evidence
        .iter()
        .find(|seen| Some(seen.object.kind.as_str()) == kind_of(link.to))?;
    let workload = object(at, &seen.object);
    match link.to {
        Joined::Deployment => {
            let hash = current_hash(workloads, link)
                .map_err(|why| sentence(&format!("{why}, so Coroot's revision of it isn't known")));
            let runs_it = hash.is_err()
                || run.iter().any(|pods| {
                    pods.from == Joined::Deployment
                        && pods.to == Joined::Pod
                        && pods.subject == link.subject
                        && pods.confidence == Confidence::Confirmed
                });
            let action = Action::revision(workload, hash);
            Some(if runs_it {
                action
            } else {
                action.disabled(
                    "Its current revision isn't confirmed to run this Freight: see its Pods row. \
                     Coroot's revision of it may be another change.",
                )
            })
        }
        Joined::Rollout => Some(
            Action::new(Target::Revision {
                deployment: workload,
                hash: None,
            })
            .disabled("Coroot keeps revisions of Deployments only, and this is an Argo Rollout."),
        ),
        _ => None,
    }
}

/// The pod-template hash of the current ReplicaSet of the Deployment a
/// link leads to, or why it isn't known.
fn current_hash(workloads: Option<&Workloads>, link: &Link) -> Result<String, String> {
    let deployment = workloads
        .and_then(|read| read.deployments.read())
        .and_then(|all| {
            all.iter()
                .find(|d| format!("{}/{}", d.namespace, d.name) == link.subject)
        })
        .ok_or("the Deployment wasn't read")?;
    let sets = workloads
        .and_then(|read| read.deployment_sets.read())
        .ok_or("the Deployment's ReplicaSets couldn't be read")?;
    let set = current_set(deployment, sets)?;
    set.pod_hash
        .clone()
        .ok_or_else(|| "its current ReplicaSet carries no pod-template-hash label".into())
}

/// The builds of the Freight's commit: every link the join made before it
/// reached a Freight, one hop each.
fn build_hops(
    sides: &Sides,
    evidence: &Evidence,
    freight: &Freight,
    pages: &Pages,
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
            from: sides.builds.label.clone(),
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
                sides,
                evidence,
                freight,
                pages,
                format!("build-{}-{n}", kind_key(link.to)),
                0,
                link,
                &[],
            )
        })
        .collect()
}

/// The Freight itself, joined to its build by the link the join made into
/// it, when it made one.
fn freight_hop(
    sides: &Sides,
    evidence: &Evidence,
    freight: &Freight,
    pages: &Pages,
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
        from: sides.kargo.label.clone(),
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
            actions: vec![
                Action::new(Target::Resource {
                    what: "the Freight".into(),
                    object: object(
                        &sides.kargo,
                        &ObjectRef::new(
                            kargo::GROUP,
                            "Freight",
                            Some(&freight.project),
                            &freight.name,
                            &freight.meta,
                        ),
                    ),
                }),
                Action::browser("Open in Kargo", links::freight_page(pages, freight)),
            ],
        })),
    }
}

/// The row that says the project's Stages weren't read, so none is shown.
fn unread_stages(sides: &Sides, why: &str) -> Hop {
    let body = sentence(&format!("The project's Stages weren't read: {why}"));
    Hop {
        key: "stages".into(),
        group: 1,
        state: Unknown,
        name: "Stages".into(),
        detail: format!("Not read: {why}"),
        from: sides.kargo.label.clone(),
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

fn gate_hop(sides: &Sides, stage: &str, group: usize, ix: usize, gate: &str, row: &Row) -> Hop {
    Hop {
        key: format!("{stage}-{}", gate.to_lowercase()),
        group,
        state: row.state,
        name: gate.into(),
        detail: row.detail.clone(),
        from: sides.kargo.label.clone(),
        at: row.at,
        link: None,
        shows: Shows::Stage(ix),
    }
}

/// What may be missing from the change, one sentence each, in the words
/// that name the entry: a part whose entry couldn't be read at all, a
/// destination that couldn't be, and a kind refused or not served where it
/// was asked.
fn missing(place: &Place, evidence: &Evidence, sides: &Sides, reached: &Reached) -> Vec<String> {
    fn kind<T>(source: &Source<T>, what: &str, at: &At) -> Option<String> {
        let entry = &at.label;
        match source {
            Source::Refused(why) => Some(format!("{what} were refused on {entry}: {why}")),
            Source::NotInstalled(_) => Some(format!("{what} aren't served on {entry}")),
            Source::Unreadable(why) => Some(format!("{what} couldn't be read on {entry}: {why}")),
            Source::Read(_) | Source::Capped(..) => None,
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for (unread, kinds) in [
        (
            &reached.builds,
            // No build is looked for without a commit.
            vec![
                (!evidence.sha.is_empty())
                    .then(|| kind(&evidence.builds, "PipelineRuns", &sides.builds))
                    .flatten(),
            ],
        ),
        (
            &reached.kargo,
            vec![
                kind(&evidence.kargo.stages, "Kargo Stages", &sides.kargo),
                kind(&evidence.kargo.promotions, "Kargo Promotions", &sides.kargo),
            ],
        ),
        (
            &reached.argocd,
            vec![kind(
                &evidence.applications,
                "Argo CD Applications",
                &sides.argocd,
            )],
        ),
    ] {
        match unread {
            Some(unread) => lines.push(unread.words()),
            None => lines.extend(kinds.into_iter().flatten()),
        }
    }
    for (context, read) in &evidence.environments {
        let at = place.at_context(context);
        match read {
            Err(why) => lines.push(why.clone()),
            Ok(workloads) => {
                let pods = workloads
                    .pods
                    .values()
                    .chain(workloads.deployment_pods.values())
                    .map(|read| &read.pods)
                    .chain(workloads.namespace_pods.values())
                    .find(|pods| matches!(pods, Source::Refused(_) | Source::NotInstalled(_)));
                lines.extend(
                    [
                        kind(&workloads.rollouts, "Rollouts", &at),
                        kind(&workloads.deployments, "Deployments", &at),
                        pods.and_then(|pods| kind(pods, "Pods", &at)),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|line| !line.starts_with("Rollouts aren't served")),
                );
            }
        }
    }
    lines.dedup();
    lines
}
