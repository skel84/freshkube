//! How sure each part's link to its application is (#521): the delivery
//! chain's [`Confidence`], with what was read on each side and why.
//!
//! A link both sides state, or that a controller reports from objects that
//! were read, is confirmed. One a label, an annotation, a name or the user
//! states is claimed. One whose other side couldn't be read is unknown. A
//! lower rule's claim stays listed on its part, and a read that may have
//! left parts out is a [`Gap`], never an empty group. The words name no
//! cluster: a side and a gap carry its key, for a screen to name it as it
//! labels clusters.

use super::{
    Application, Basis, CoverageState, Derived, Evidence, Member, MemberKind, MemberRef, Note,
    Rule, SessionKey, SourceKind,
};
use crate::delivery::join::Confidence;
use crate::delivery::observation::Fact;
use crate::workloads::WorkloadKind;

/// One side of a link: what was read there, in which cluster when it is
/// one cluster's, and what kind of statement it is. No fact when that side
/// wasn't read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub text: String,
    pub session: Option<SessionKey>,
    pub fact: Option<Fact>,
}

impl Side {
    fn read(fact: Fact, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            session: None,
            fact: Some(fact),
        }
    }

    fn unread(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            session: None,
            fact: None,
        }
    }

    fn at(mut self, session: &SessionKey) -> Self {
        self.session = Some(session.clone());
        self
    }
}

/// One part's link to the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub member: MemberRef,
    pub confidence: Confidence,
    /// The application's side: its own object, or what names the part.
    pub app_side: Side,
    /// The part's side: what on it, or about it, was read.
    pub member_side: Side,
    /// Why the link has its confidence.
    pub why: String,
    /// What wasn't checked that could still say otherwise: Argo CD's
    /// Applications in the part's cluster, for a labelled part there.
    pub unchecked: Option<Unchecked>,
    /// Lower rules that also found the part, with what each would have
    /// called its application.
    pub lower: Vec<(Rule, String)>,
}

/// How Argo CD's Applications in a labelled part's cluster were read,
/// when not in full: the cluster is the part's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// Refused or failed: none were read.
    NotRead,
    /// Read in these namespaces only.
    Namespaces(Vec<String>),
    /// Nothing marks Argo CD, and its default namespace holds none.
    NotFound(String),
    /// Stopped at the page cap after this many.
    Capped(usize),
}

/// A read in one cluster that may have left parts of this application out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub session: SessionKey,
    /// The kinds that may be missing, in [`MemberKind::rank`] order.
    pub kinds: Vec<MemberKind>,
    /// What may be missing and why, such as "Labelled workloads may be
    /// missing: refused: forbidden".
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    /// In kind order, then cluster, namespace and name.
    pub links: Vec<Claim>,
    /// In kind order, then cluster.
    pub gaps: Vec<Gap>,
}

/// Every part's link to `app`, and what may be missing from it.
pub fn claims(derived: &Derived, app: &Application) -> Claims {
    let mut links: Vec<Claim> = app
        .members
        .iter()
        .map(|member| link(derived, app, member))
        .collect();
    links.sort_by(|a, b| {
        let order = |m: &MemberRef| {
            (
                m.kind.rank(),
                m.session.clone(),
                m.namespace.clone(),
                m.name.clone(),
            )
        };
        order(&a.member).cmp(&order(&b.member))
    });
    Claims {
        links,
        gaps: gaps(derived, app),
    }
}

fn kind_word(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::KargoStage => "Stage",
        MemberKind::KargoWarehouse => "Warehouse",
        MemberKind::ArgoApplication => "Application",
        MemberKind::Workload(kind) => super::rules::kind_name(kind),
    }
}

fn object(member: &MemberRef) -> String {
    match &member.namespace {
        Some(ns) => format!("{} {ns}/{}", kind_word(member.kind), member.name),
        None => format!("{} {}", kind_word(member.kind), member.name),
    }
}

/// The application's own object, as read, or what made it: its first
/// evidence, which is the rule's own.
fn own(app: &Application) -> Side {
    let Some(evidence) = app.evidence.first() else {
        return Side::unread("Nothing was read for it");
    };
    match evidence {
        Evidence::KargoProject { session, project } => {
            Side::read(Fact::Reported, format!("Project {project}, read")).at(session)
        }
        Evidence::ArgoApplicationSet {
            session,
            namespace,
            name,
            read,
        } => {
            let set = format!("ApplicationSet {namespace}/{name}");
            if *read {
                Side::read(Fact::Reported, format!("{set}, read")).at(session)
            } else {
                Side::unread(format!("{set}, not found")).at(session)
            }
        }
        Evidence::ArgoApplication {
            session,
            namespace,
            name,
        } => Side::read(
            Fact::Reported,
            format!("Application {namespace}/{name}, read"),
        )
        .at(session),
        Evidence::PartOfLabel { value } => Side::read(
            Fact::Declared,
            format!("The label app.kubernetes.io/part-of={value}"),
        ),
        Evidence::Override(what) => Side::read(Fact::Declared, format!("Your override: {what}")),
    }
}

fn link(derived: &Derived, app: &Application, member: &Member) -> Claim {
    let at = &member.at;
    let lower = app
        .notes
        .iter()
        .filter_map(|note| match note {
            Note::LowerClaim { member, rule, name } if member == at => Some((*rule, name.clone())),
            _ => None,
        })
        .collect();
    let unchecked = app.notes.iter().find_map(|note| match note {
        Note::ManagerUnknown { member } if member == at => Some(argo_unchecked(derived, at)),
        _ => None,
    });
    let claim = |confidence, app_side, member_side, why: &str| {
        let mut why = why.to_owned();
        if unchecked.is_some() {
            why.push_str(
                ". Argo CD's Applications there weren't all read, so one may manage it and its \
                 label may not be the last word",
            );
        }
        Claim {
            member: at.clone(),
            confidence,
            app_side,
            member_side,
            why,
            unchecked: unchecked.clone(),
            lower,
        }
    };
    if let Some((member_side, why)) = unknown(app, at) {
        return claim(Confidence::Unknown, own(app), member_side, why);
    }
    let listed = Side::read(Fact::Reported, format!("{}, listed", object(at))).at(&at.session);
    // The Application whose inventory lists the part, named with its
    // cluster, and its own link to this application.
    let via = member
        .via
        .as_ref()
        .and_then(|via| app.members.iter().find(|m| &m.at == via));
    let inventory = || match via {
        Some(via) => Side::read(
            Fact::Reported,
            format!("Argo CD {}'s inventory lists it", object(&via.at)),
        )
        .at(&via.at.session),
        None => Side::read(
            Fact::Reported,
            "An Argo CD Application's inventory lists it",
        ),
    };
    // A part found through an Application is no surer than the
    // Application's own link.
    let through = |confidence: Confidence, why: &str| -> (Confidence, String) {
        match via.map(|via| link(derived, app, via)) {
            Some(via_link) if via_link.confidence > confidence => (
                via_link.confidence,
                format!(
                    "{why}, but that Application's own link is {}: {}",
                    via_link.confidence.word(),
                    via_link.why
                ),
            ),
            _ => (confidence, why.to_owned()),
        }
    };
    match member.basis {
        Basis::Direct => match (at.kind, app.evidence.first()) {
            (MemberKind::KargoStage | MemberKind::KargoWarehouse, _) => claim(
                Confidence::Confirmed,
                own(app),
                listed,
                "Kargo keeps a Project's Stages and Warehouses in its namespace, and both were \
                 read",
            ),
            (
                MemberKind::ArgoApplication,
                Some(Evidence::ArgoApplicationSet {
                    session,
                    name,
                    read,
                    ..
                }),
            ) => {
                let owner = Side::read(
                    Fact::Reported,
                    format!("Its owner reference names ApplicationSet {name}"),
                )
                .at(&at.session);
                let (confidence, why) = if *read {
                    (
                        Confidence::Confirmed,
                        "The ApplicationSet was read, and the Application's owner reference \
                         names it",
                    )
                } else if blind_to_sets(derived, session) {
                    (
                        Confidence::Unknown,
                        "Argo CD's ApplicationSets couldn't all be read, so whether this one \
                         exists is unknown",
                    )
                } else {
                    (
                        Confidence::Claimed,
                        "Only the owner reference says so: no such ApplicationSet was found",
                    )
                };
                claim(confidence, own(app), owner, why)
            }
            (MemberKind::ArgoApplication, _) => claim(
                Confidence::Confirmed,
                own(app),
                listed,
                "It is the Application itself",
            ),
            (MemberKind::Workload(_), _) => claim(
                Confidence::Claimed,
                own(app),
                part_of(at, &app.name),
                "Only a label says so",
            ),
        },
        Basis::Tracked => {
            let (confidence, why) = through(
                Confidence::Confirmed,
                "Argo CD lists it, and it names the Application back",
            );
            claim(
                confidence,
                inventory(),
                Side::read(
                    Fact::Reported,
                    "Its tracking annotation or instance label names that Application",
                )
                .at(&at.session),
                &why,
            )
        }
        Basis::ManagedBy => {
            let (confidence, why) = through(
                Confidence::Claimed,
                "Only Argo CD's inventory says so: nothing on it names the Application back",
            );
            claim(
                confidence,
                inventory(),
                // What was read shows no such name: derived from the read,
                // not stated by anyone.
                Side::read(
                    Fact::Derived,
                    "No tracking annotation or instance label names that Application",
                )
                .at(&at.session),
                &why,
            )
        }
        Basis::NamesProject => claim(
            Confidence::Claimed,
            own(app),
            Side::read(
                Fact::Declared,
                format!(
                    "Its authorized-stage annotation or the configured naming names Project {}",
                    app.name
                ),
            )
            .at(&at.session),
            "Only an annotation or a name says so",
        ),
        Basis::SameName => claim(
            Confidence::Claimed,
            own(app),
            part_of(at, &app.name),
            "Only its label, which is this application's name, says so",
        ),
        Basis::Inferred => {
            let clusters = app
                .notes
                .iter()
                .find_map(|note| match note {
                    Note::JoinedAcrossSessions { sessions, .. } => Some(sessions.len()),
                    _ => None,
                })
                .unwrap_or_default();
            let joined = match clusters {
                0 | 1 => format!("Joined by the name {} in another cluster", app.name),
                n => format!("Joined by the name {} across {n} clusters", app.name),
            };
            claim(
                Confidence::Claimed,
                own(app),
                Side::read(Fact::Derived, joined).at(&at.session),
                "Only the same name in another cluster says so",
            )
        }
        Basis::Override => claim(
            Confidence::Claimed,
            own(app),
            Side::read(Fact::Declared, "Placed here by your override"),
            "Placed by your override",
        ),
    }
}

/// The part's side and why, when the read that would settle the link
/// didn't happen.
fn unknown(app: &Application, at: &MemberRef) -> Option<(Side, &'static str)> {
    app.notes.iter().find_map(|note| match note {
        Note::ProjectNotRead {
            member,
            project,
            session,
            why,
        } if member == at => Some((
            Side::unread(format!("Kargo Project {project} wasn't read: {why}")).at(session),
            "It names a Kargo Project that wasn't read, so which application it belongs to is \
             unknown",
        )),
        _ => None,
    })
}

fn part_of(at: &MemberRef, name: &str) -> Side {
    Side::read(
        Fact::Declared,
        format!("{} carries app.kubernetes.io/part-of={name}", object(at)),
    )
    .at(&at.session)
}

/// What wasn't checked in the part's cluster: Argo CD's Applications,
/// none of them or only some.
fn argo_unchecked(derived: &Derived, at: &MemberRef) -> Unchecked {
    let state = derived
        .coverage
        .iter()
        .find(|c| c.session == at.session && c.source == SourceKind::ArgoApplications);
    match state.map(|c| &c.state) {
        Some(CoverageState::NamespaceOnly { namespaces, .. }) => {
            Unchecked::Namespaces(namespaces.clone())
        }
        Some(CoverageState::NamespaceNotFound(namespace)) => Unchecked::NotFound(namespace.clone()),
        Some(CoverageState::Capped(read)) => Unchecked::Capped(*read),
        _ => Unchecked::NotRead,
    }
}

/// Why Argo CD's Applications in a cluster may not all have been read:
/// [`blind`], or read in some namespaces only.
fn partial(state: &CoverageState) -> Option<String> {
    match state {
        CoverageState::NamespaceOnly { namespaces, .. } => {
            Some(format!("read in {} only", namespaces.join(", ")))
        }
        _ => blind(state),
    }
}

/// Whether the ApplicationSets read in `session` may have missed one.
fn blind_to_sets(derived: &Derived, session: &SessionKey) -> bool {
    derived.coverage.iter().any(|c| {
        &c.session == session
            && c.source == SourceKind::ArgoApplicationSets
            && blind(&c.state).is_some()
    })
}

/// Why a read may have missed something: capped, refused or failed. A read
/// of one namespace is the normal live case, shown as a fact, and a source
/// that isn't served holds nothing.
fn blind(state: &CoverageState) -> Option<String> {
    match state {
        CoverageState::Capped(read) => Some(format!("stopped at the page cap after {read}")),
        CoverageState::Refused(why) => Some(format!("refused: {why}")),
        CoverageState::Unreadable(why) => Some(format!("couldn't be read: {why}")),
        CoverageState::NamespaceNotFound(namespace) => Some(format!(
            "Argo CD's namespace wasn't found, and {namespace} holds no Applications"
        )),
        CoverageState::Read
        | CoverageState::NamespaceOnly { .. }
        | CoverageState::NotInstalled(_) => None,
    }
}

const WORKLOADS: [MemberKind; 3] = [
    MemberKind::Workload(WorkloadKind::Deployment),
    MemberKind::Workload(WorkloadKind::StatefulSet),
    MemberKind::Workload(WorkloadKind::DaemonSet),
];

/// What may be missing: the Project's Stage and Warehouse reads that didn't
/// answer, Argo CD's Applications where the application's own objects sit,
/// and labelled workloads anywhere, since a label joins any cluster.
fn gaps(derived: &Derived, app: &Application) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = app
        .notes
        .iter()
        .filter_map(|note| match note {
            Note::MembersUnknown { session, why } => Some(Gap {
                session: session.clone(),
                kinds: vec![MemberKind::KargoStage, MemberKind::KargoWarehouse],
                why: format!("Kargo Stages and Warehouses may be missing: {why}"),
            }),
            _ => None,
        })
        .collect();
    let own: Vec<&SessionKey> = app
        .evidence
        .iter()
        .filter_map(|evidence| match evidence {
            Evidence::KargoProject { session, .. }
            | Evidence::ArgoApplicationSet { session, .. }
            | Evidence::ArgoApplication { session, .. } => Some(session),
            _ => None,
        })
        .collect();
    for coverage in &derived.coverage {
        let session = &coverage.session;
        let ((kinds, what), why) = match coverage.source {
            SourceKind::ArgoApplications if app.rule == Rule::ArgoCd && own.contains(&session) => (
                (vec![MemberKind::ArgoApplication], "Argo CD Applications"),
                blind(&coverage.state),
            ),
            // A Project's Stages promote to Argo CD Applications, in any
            // cluster, since an Application names its Project by name. Where
            // the Project is, a read of only some namespaces may have left
            // them out; anywhere else, a refused, failed or capped read.
            SourceKind::ArgoApplications if app.rule == Rule::Kargo => (
                (
                    vec![MemberKind::ArgoApplication],
                    "Argo CD Applications its Stages promote to",
                ),
                if own.contains(&session) {
                    partial(&coverage.state)
                } else {
                    blind(&coverage.state)
                },
            ),
            SourceKind::Workloads => (
                (WORKLOADS.to_vec(), "Labelled workloads"),
                blind(&coverage.state),
            ),
            _ => continue,
        };
        let Some(why) = why else {
            continue;
        };
        gaps.push(Gap {
            session: session.clone(),
            kinds,
            why: format!("{what} may be missing: {why}"),
        });
    }
    gaps.sort_by(|a, b| {
        let order = |g: &Gap| (g.kinds.first().map(|k| k.rank()), g.session.clone());
        order(a).cmp(&order(b))
    });
    gaps
}

#[cfg(test)]
mod tests;
