//! The page's example data: core's acme workspace, six clusters each read
//! for what the example workspace reads there: Kargo in `cicd-fra`, Argo
//! CD in `core-fra`, read where a live read would look (nothing is
//! labelled as Argo CD's, so in `argocd`), and the environments' labelled
//! workloads.
//! Debug builds reshape it for captures with `FRESHKUBE_APPLICATIONS`.
use freshkube_core::applications::read::argo_namespaces;
use freshkube_core::applications::{
    Absent, ArgoFound, ArgoScope, Asked, Inputs, SessionInputs, SessionKey, example as acme,
};
use freshkube_core::delivery::source::{Source, Truncation};

use super::display::Labels;
use super::links::Connections;

/// A shape of the example, for captures and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Variant {
    /// All six clusters.
    #[default]
    Acme,
    /// One cluster read for everything, as the workspace of one is:
    /// `core-fra` holding Kargo too.
    Single,
    /// Kargo refused in `cicd-fra`: its applications may be missing.
    Partial,
    /// Every source refused everywhere.
    Refused,
    /// Neither Kargo nor Argo CD served, and no labelled workloads.
    Unserved,
    /// No cluster answered.
    Failed,
    /// Argo CD's Applications stopped at the page cap in `core-fra`.
    Capped,
    /// Argo CD found by its workloads in five namespaces of `core-fra`, the
    /// last two past the cap.
    Spread,
    /// Nothing marks Argo CD in `core-fra`, and `argocd` holds no
    /// Applications.
    NotFound,
    /// `checkout`'s Stages didn't answer in `cicd-fra`: its page shows
    /// what may be missing as a group row.
    Stages,
    /// Deployments refused in `prod-lon`: labelled parts there may be
    /// missing, as `checkout`'s page says in a group row.
    Workloads,
    /// `cicd-fra` couldn't be reached: Kargo's applications may be missing.
    Unreachable,
    /// Argo CD refused in `core-fra`: its applications may be missing.
    ArgoRefused,
    /// `dev-fra`, an Argo CD destination, couldn't be reached: labelled
    /// parts there may be missing.
    Destination,
}

impl Variant {
    /// `single`, `partial`, `refused`, `unserved`, `failed`, `capped`,
    /// `spread`, `not-found`, `stages`, `workloads`, `unreachable`,
    /// `argo-refused` or `destination` from
    /// `FRESHKUBE_APPLICATIONS`;
    /// debug and stress builds only.
    pub(crate) fn from_env() -> Self {
        if !cfg!(any(debug_assertions, feature = "stress")) {
            return Self::default();
        }
        match std::env::var("FRESHKUBE_APPLICATIONS").as_deref() {
            Ok("single") => Self::Single,
            Ok("partial") => Self::Partial,
            Ok("refused") => Self::Refused,
            Ok("unserved") => Self::Unserved,
            Ok("failed") => Self::Failed,
            Ok("capped") => Self::Capped,
            Ok("spread") => Self::Spread,
            Ok("not-found") => Self::NotFound,
            Ok("stages") => Self::Stages,
            Ok("workloads") => Self::Workloads,
            Ok("unreachable") => Self::Unreachable,
            Ok("argo-refused") => Self::ArgoRefused,
            Ok("destination") => Self::Destination,
            _ => Self::Acme,
        }
    }
}

fn refused<T>(what: &str) -> Source<T> {
    Source::Refused(format!("{what} is forbidden for this identity"))
}

fn session<'a>(inputs: &'a mut Inputs, cluster: &str) -> &'a mut SessionInputs {
    inputs
        .sessions
        .iter_mut()
        .find(|s| s.key.0 == cluster)
        .expect("an acme cluster")
}

/// Argo CD's cluster.
fn core(inputs: &mut Inputs) -> &mut SessionInputs {
    session(inputs, acme::CORE)
}

/// Kargo's cluster.
fn cicd(inputs: &mut Inputs) -> &mut SessionInputs {
    session(inputs, acme::CICD)
}

pub(crate) fn inputs(variant: Variant) -> Inputs {
    let mut inputs = acme::acme();
    for session in &mut inputs.sessions {
        if session.key.0 == acme::CORE {
            let (namespaces, found) = argo_namespaces(&session.workloads);
            session.argo_scope = ArgoScope::Namespace { namespaces, found };
        }
    }
    match variant {
        Variant::Acme => {}
        Variant::Single => {
            let kargo = cicd(&mut inputs).kargo.clone();
            inputs.sessions.retain(|s| s.key.0 == acme::CORE);
            let core = core(&mut inputs);
            core.kargo = kargo;
            core.asked = Asked::ALL;
        }
        Variant::Partial => cicd(&mut inputs).kargo = refused("projects.kargo.akuity.io"),
        Variant::Refused => {
            for session in &mut inputs.sessions {
                session.kargo = refused("projects.kargo.akuity.io");
                session.argo_applications = refused("applications.argoproj.io");
                session.argo_application_sets = refused("applicationsets.argoproj.io");
                session.workloads = refused("deployments.apps");
            }
        }
        Variant::Unserved => {
            for session in &mut inputs.sessions {
                session.kargo = Source::NotInstalled("kargo.akuity.io is not served".into());
                session.argo_applications =
                    Source::NotInstalled("argoproj.io is not served".into());
                session.argo_application_sets =
                    Source::NotInstalled("argoproj.io is not served".into());
                session.workloads = Source::Read(Vec::new());
            }
        }
        Variant::Failed => {
            for session in &mut inputs.sessions {
                let why = format!("connection refused by {}.example.test:6443", session.key.0);
                unreachable(session, &why);
            }
        }
        Variant::Capped => {
            let core = core(&mut inputs);
            if let Source::Read(apps) = &core.argo_applications {
                core.argo_applications = Source::Capped(apps.clone(), Truncation { read: 500 });
            }
        }
        Variant::Spread => {
            core(&mut inputs).argo_scope = ArgoScope::Namespace {
                namespaces: ["delivery", "gitops", "platform"].map(Into::into).to_vec(),
                found: ArgoFound::Labelled {
                    skipped: vec!["sandbox".into(), "tenants".into()],
                },
            };
        }
        Variant::NotFound => {
            let core = core(&mut inputs);
            core.argo_applications = Source::Read(Vec::new());
            core.argo_application_sets = Source::Read(Vec::new());
        }
        Variant::Stages => {
            if let Source::Read(projects) = &mut cicd(&mut inputs).kargo
                && let Some(checkout) = projects.iter_mut().find(|p| p.name == "checkout")
            {
                checkout.stages = Source::Unreadable(
                    "stages.kargo.akuity.io: the read timed out after 10s".into(),
                );
            }
        }
        Variant::Workloads => {
            if let Some(prod) = inputs.sessions.iter_mut().find(|s| s.key.0 == "prod-lon") {
                prod.workloads = refused("deployments.apps");
            }
        }
        Variant::Unreachable => unreachable(
            cicd(&mut inputs),
            "no answer within 5 s; a credential plugin may be waiting for a sign-in",
        ),
        Variant::ArgoRefused => {
            let core = core(&mut inputs);
            core.argo_applications = refused("applications.argoproj.io");
            core.argo_application_sets = refused("applicationsets.argoproj.io");
        }
        Variant::Destination => unreachable(
            session(&mut inputs, "dev-fra"),
            "connection refused by dev-fra.example.test:6443",
        ),
    }
    inputs
}

/// A cluster whose entry couldn't be reached, for `why`.
fn unreachable(session: &mut SessionInputs, why: &str) {
    *session = SessionInputs::absent(
        session.key.clone(),
        session.asked,
        Absent::Unreachable(why.into()),
    );
}

/// The acme clusters are named as the workspace names them.
pub(crate) fn labels(inputs: &Inputs) -> Labels {
    Labels::new(
        inputs
            .sessions
            .iter()
            .map(|s| (s.key.clone(), s.key.0.clone())),
    )
}

/// The acme cluster the open workspace entry is, else `core-fra`, is the
/// example cluster that is open, whose Resources list its objects; acme's
/// other clusters are no connection, so their parts don't open here.
pub(crate) fn connections(open: &str, active: Option<&str>) -> Connections {
    let cluster = active
        .filter(|active| acme::acme().sessions.iter().any(|s| s.key.0 == *active))
        .unwrap_or(acme::CORE);
    Connections::new(open, [(SessionKey::new(cluster), open.to_owned())])
}

#[cfg(test)]
mod tests {
    /// Only `core-fra` is mapped to the open example cluster, so no other
    /// acme cluster may share an example context's name: one would read as
    /// the open cluster while its parts are refused.
    #[test]
    fn no_acme_cluster_is_named_as_an_example_context() {
        let inputs = freshkube_core::applications::example::acme();
        for session in &inputs.sessions {
            assert!(
                !crate::fixture::CONTEXTS.contains(&session.key.0.as_str()),
                "{} is also an example context",
                session.key.0
            );
        }
    }
}
