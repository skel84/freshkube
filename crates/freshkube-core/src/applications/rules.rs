//! The three rules, applied in precedence order to one shared set of
//! applications: an object is claimed by the highest rule that finds it, and
//! a lower claim is noted on the member, never dropped.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{
    Application, ApplicationId, Basis, Coverage, CoverageState, Destination, Evidence, Inputs,
    Member, MemberKind, MemberRef, Note, Rule, SessionInputs, SessionKey, SourceKind,
};
use crate::delivery::argocd::{
    Application as ArgoApplication, IN_CLUSTER_NAME, IN_CLUSTER_SERVER, normalize_server,
};
use crate::delivery::source::Source;
use crate::workloads::WorkloadKind;

#[derive(Default)]
pub(super) struct Builder {
    pub apps: BTreeMap<ApplicationId, Application>,
}

impl Builder {
    fn app(
        &mut self,
        id: ApplicationId,
        rule: Rule,
        name: &str,
        evidence: Evidence,
    ) -> &mut Application {
        let app = self.apps.entry(id.clone()).or_insert_with(|| Application {
            id,
            name: name.to_owned(),
            rule,
            evidence: Vec::new(),
            members: Vec::new(),
            notes: Vec::new(),
        });
        if !app.evidence.contains(&evidence) {
            app.evidence.push(evidence);
        }
        app
    }

    fn join(&mut self, id: &ApplicationId, member: Member) {
        if let Some(app) = self.apps.get_mut(id)
            && !app.members.iter().any(|m| m.at == member.at)
        {
            app.members.push(member);
        }
    }

    fn note(&mut self, id: &ApplicationId, note: Note) {
        if let Some(app) = self.apps.get_mut(id)
            && !app.notes.contains(&note)
        {
            app.notes.push(note);
        }
    }
}

pub(super) fn coverage(inputs: &Inputs) -> Vec<Coverage> {
    let mut out = Vec::new();
    for session in &inputs.sessions {
        let mut push = |source, project: Option<&str>, state| {
            out.push(Coverage {
                session: session.key.clone(),
                source,
                project: project.map(str::to_owned),
                state,
            });
        };
        push(
            SourceKind::KargoProjects,
            None,
            CoverageState::of(&session.kargo),
        );
        for project in session.kargo.read().into_iter().flatten() {
            let name = Some(project.name.as_str());
            push(
                SourceKind::KargoStages,
                name,
                CoverageState::of(&project.stages),
            );
            push(
                SourceKind::KargoWarehouses,
                name,
                CoverageState::of(&project.warehouses),
            );
        }
        push(
            SourceKind::ArgoApplications,
            None,
            CoverageState::of(&session.argo_applications),
        );
        push(
            SourceKind::ArgoApplicationSets,
            None,
            CoverageState::of(&session.argo_application_sets),
        );
        push(
            SourceKind::Workloads,
            None,
            CoverageState::of(&session.workloads),
        );
    }
    out
}

fn member(
    session: &SessionKey,
    kind: MemberKind,
    namespace: Option<&str>,
    name: &str,
    basis: Basis,
) -> Member {
    Member {
        at: MemberRef {
            session: session.clone(),
            kind,
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        },
        basis,
        destination: None,
        also_claimed_by: Vec::new(),
    }
}

/// Rule 1: each Kargo Project, with its Stages and Warehouses.
pub(super) fn kargo(builder: &mut Builder, inputs: &Inputs) -> BTreeSet<String> {
    let mut projects = BTreeSet::new();
    for session in &inputs.sessions {
        for project in session.kargo.read().into_iter().flatten() {
            projects.insert(project.name.clone());
            let evidence = Evidence::KargoProject {
                session: session.key.clone(),
                project: project.name.clone(),
            };
            let id = builder
                .app(
                    ApplicationId::new(Rule::Kargo, &project.name),
                    Rule::Kargo,
                    &project.name,
                    evidence,
                )
                .id
                .clone();
            let ns = Some(project.name.as_str());
            match &project.stages {
                Source::Read(stages) | Source::Capped(stages, _) => {
                    capped_members(builder, &id, "Stages", &project.stages);
                    for stage in stages {
                        let m = member(
                            &session.key,
                            MemberKind::KargoStage,
                            ns,
                            &stage.name,
                            Basis::Direct,
                        );
                        builder.join(&id, m);
                    }
                }
                other => unknown_members(builder, &id, "Stages", other),
            }
            match &project.warehouses {
                Source::Read(items) | Source::Capped(items, _) => {
                    capped_members(builder, &id, "Warehouses", &project.warehouses);
                    for warehouse in items {
                        let m = member(
                            &session.key,
                            MemberKind::KargoWarehouse,
                            ns,
                            &warehouse.name,
                            Basis::Direct,
                        );
                        builder.join(&id, m);
                    }
                }
                other => unknown_members(builder, &id, "Warehouses", other),
            }
        }
    }
    projects
}

fn unknown_members<T>(builder: &mut Builder, id: &ApplicationId, what: &str, source: &Source<T>) {
    let why = source.why_not_read().unwrap_or_default();
    builder.note(
        id,
        Note::MembersUnknown {
            why: format!("{what}: {why}"),
        },
    );
}

/// A listing that stopped at the page cap read real items, but may not have
/// read them all.
fn capped_members<T>(builder: &mut Builder, id: &ApplicationId, what: &str, source: &Source<T>) {
    if let Some(truncation) = source.capped() {
        builder.note(
            id,
            Note::MembersUnknown {
                why: format!(
                    "{what}: the listing stopped at the page cap after {} items",
                    truncation.read
                ),
            },
        );
    }
}

fn destination(application: &ArgoApplication) -> Destination {
    let own_server = |server: &str| {
        let server = normalize_server(server);
        server == normalize_server(IN_CLUSTER_SERVER)
    };
    match (
        &application.destination_server,
        &application.destination_name,
    ) {
        (None, None) => Destination::Missing,
        (_, Some(name)) if name == IN_CLUSTER_NAME => Destination::Local,
        (Some(server), None) if own_server(server) => Destination::Local,
        (server, name) => Destination::Other {
            server: server.clone(),
            name: name.clone(),
        },
    }
}

/// Rule 2: ApplicationSets and Applications. Returns where each Application
/// went, for the workload pass.
pub(super) fn argo(
    builder: &mut Builder,
    inputs: &Inputs,
    projects: &BTreeSet<String>,
) -> HashMap<(SessionKey, String, String), ApplicationId> {
    let mut home = HashMap::new();
    for session in &inputs.sessions {
        let sets = session
            .argo_application_sets
            .read()
            .map_or(&[][..], Vec::as_slice);
        for application in session.argo_applications.read().into_iter().flatten() {
            let naming = inputs.stage_naming.as_ref();
            let project = projects
                .iter()
                .find(|project| application.is_of_project(project, naming));
            let own_name = application
                .owner_application_set
                .as_deref()
                .unwrap_or(&application.name);
            let (id, basis) = match project {
                Some(project) => (
                    ApplicationId::new(Rule::Kargo, project),
                    Basis::NamesProject,
                ),
                None => match &application.owner_application_set {
                    Some(set) => {
                        // An owner reference stays in the owner's namespace.
                        let known = sets
                            .iter()
                            .find(|s| &s.name == set && s.namespace == application.namespace);
                        let evidence = Evidence::ArgoApplicationSet {
                            session: session.key.clone(),
                            namespace: application.namespace.clone(),
                            name: set.clone(),
                            read: known.is_some(),
                        };
                        let id = ApplicationId::argo_application_set(&application.namespace, set);
                        (
                            builder.app(id, Rule::ArgoCd, set, evidence).id.clone(),
                            Basis::Direct,
                        )
                    }
                    None => {
                        let evidence = Evidence::ArgoApplication {
                            session: session.key.clone(),
                            namespace: application.namespace.clone(),
                            name: application.name.clone(),
                        };
                        let id = ApplicationId::argo_application(
                            &application.namespace,
                            &application.name,
                        );
                        (
                            builder
                                .app(id, Rule::ArgoCd, &application.name, evidence)
                                .id
                                .clone(),
                            Basis::Direct,
                        )
                    }
                },
            };
            let mut m = member(
                &session.key,
                MemberKind::ArgoApplication,
                Some(&application.namespace),
                &application.name,
                basis,
            );
            let place = destination(application);
            if matches!(place, Destination::Other { .. }) {
                builder.note(
                    &id,
                    Note::UnmappedDestination {
                        member: m.at.clone(),
                    },
                );
            }
            m.destination = Some(place);
            if project.is_some() {
                m.also_claimed_by.push(Rule::ArgoCd);
                builder.note(
                    &id,
                    Note::LowerClaim {
                        member: m.at.clone(),
                        rule: Rule::ArgoCd,
                        name: own_name.to_owned(),
                    },
                );
            }
            home.insert(
                (
                    session.key.clone(),
                    application.namespace.clone(),
                    application.name.clone(),
                ),
                id.clone(),
            );
            builder.join(&id, m);
        }
        // A set that generated nothing is still an application.
        let applications = session
            .argo_applications
            .read()
            .map_or(&[][..], Vec::as_slice);
        for set in sets {
            if !applications.iter().any(|a| {
                a.owner_application_set.as_deref() == Some(&set.name)
                    && a.namespace == set.namespace
            }) {
                let evidence = Evidence::ArgoApplicationSet {
                    session: session.key.clone(),
                    namespace: set.namespace.clone(),
                    name: set.name.clone(),
                    read: true,
                };
                let id = ApplicationId::argo_application_set(&set.namespace, &set.name);
                builder.app(id, Rule::ArgoCd, &set.name, evidence);
            }
        }
    }
    home
}

fn kind_name(kind: WorkloadKind) -> &'static str {
    match kind {
        WorkloadKind::Deployment => "Deployment",
        WorkloadKind::StatefulSet => "StatefulSet",
        WorkloadKind::DaemonSet => "DaemonSet",
    }
}

fn managing_application<'a>(
    session: &'a SessionInputs,
    workload: &super::LabelledWorkload,
) -> Option<&'a ArgoApplication> {
    session
        .argo_applications
        .read()?
        .iter()
        .filter(|application| {
            application
                .managed_object(
                    "apps",
                    kind_name(workload.kind),
                    Some(&workload.namespace),
                    &workload.name,
                )
                .is_some()
        })
        .min_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)))
}

/// Rule 3: the `part-of` label, for the workloads no higher rule has.
pub(super) fn part_of(
    builder: &mut Builder,
    inputs: &Inputs,
    home: &HashMap<(SessionKey, String, String), ApplicationId>,
) {
    for session in &inputs.sessions {
        for workload in session.workloads.read().into_iter().flatten() {
            let value = workload.part_of.trim();
            if value.is_empty() {
                continue;
            }
            let kind = MemberKind::Workload(workload.kind);
            let ns = Some(workload.namespace.as_str());
            let managed = managing_application(session, workload).and_then(|application| {
                home.get(&(
                    session.key.clone(),
                    application.namespace.clone(),
                    application.name.clone(),
                ))
            });
            let same_name = same_name_target(builder, value);
            let (id, basis) = match (managed, same_name) {
                (Some(id), _) => (id.clone(), Basis::ManagedBy),
                (None, Some(id)) => (id, Basis::SameName),
                (None, None) => {
                    let evidence = Evidence::PartOfLabel {
                        value: value.to_owned(),
                    };
                    let id = ApplicationId::new(Rule::PartOf, value);
                    (
                        builder.app(id, Rule::PartOf, value, evidence).id.clone(),
                        Basis::Direct,
                    )
                }
            };
            let mut m = member(&session.key, kind, ns, &workload.name, basis);
            if basis != Basis::Direct {
                m.also_claimed_by.push(Rule::PartOf);
                builder.note(
                    &id,
                    Note::LowerClaim {
                        member: m.at.clone(),
                        rule: Rule::PartOf,
                        name: value.to_owned(),
                    },
                );
            }
            // A workload that stays with its label, in a cluster whose
            // Applications were not all read, may be managed by one.
            if basis != Basis::ManagedBy && !fully_read(&session.argo_applications) {
                builder.note(
                    &id,
                    Note::ManagerUnknown {
                        member: m.at.clone(),
                    },
                );
            }
            builder.join(&id, m);
        }
    }
}

/// Whether a source answered completely, or is not served at all (then there
/// is nothing it could have held).
fn fully_read<T>(source: &Source<T>) -> bool {
    matches!(source, Source::Read(_) | Source::NotInstalled(_))
}

/// The one Kargo, else the one Argo CD, application a `part-of` value is the
/// name of. Two of a kind is not guessed between.
fn same_name_target(builder: &Builder, value: &str) -> Option<ApplicationId> {
    for rule in [Rule::Kargo, Rule::ArgoCd] {
        let mut found = builder
            .apps
            .values()
            .filter(|app| app.rule == rule && app.name == value);
        match (found.next(), found.next()) {
            (None, _) => continue,
            (Some(app), None) => return Some(app.id.clone()),
            (Some(_), Some(_)) => return None,
        }
    }
    None
}

/// An id found in more than one cluster is one application only because the
/// name is the same, which is an inference. Within one cluster joining stays
/// direct; where the rule's own objects sit in several clusters, or a member
/// sits in a cluster the rule's own objects do not, the member is inferred
/// and a note names the clusters.
pub(super) fn infer_across_sessions(builder: &mut Builder) {
    for app in builder.apps.values_mut() {
        let own: BTreeSet<SessionKey> = match app.rule {
            Rule::PartOf => app
                .members
                .iter()
                .filter(|m| m.basis == Basis::Direct)
                .map(|m| m.at.session.clone())
                .collect(),
            _ => app
                .evidence
                .iter()
                .filter_map(|evidence| match evidence {
                    Evidence::KargoProject { session, .. }
                    | Evidence::ArgoApplicationSet { session, .. }
                    | Evidence::ArgoApplication { session, .. } => Some(session.clone()),
                    _ => None,
                })
                .collect(),
        };
        let mut all = own.clone();
        all.extend(app.members.iter().map(|m| m.at.session.clone()));
        if all.len() < 2 {
            continue;
        }
        let several = own.len() > 1;
        for member in &mut app.members {
            if matches!(member.basis, Basis::Direct | Basis::NamesProject)
                && (several || !own.contains(&member.at.session))
            {
                member.basis = Basis::Inferred;
            }
        }
        app.notes.push(Note::JoinedAcrossSessions {
            name: app.name.clone(),
            sessions: all.into_iter().collect(),
        });
    }
}
