//! The page's example data: core's acme workspace, six clusters with Argo
//! CD and Kargo in `core-fra`, read in `argocd` only as a live read is.
//! Debug builds reshape it for captures with `FRESHKUBE_APPLICATIONS`.
use freshkube_core::applications::{ArgoScope, Inputs, SessionInputs, example as acme};
use freshkube_core::delivery::source::{Source, Truncation};

use super::display::{ARGOCD_NAMESPACE, Labels};

/// A shape of the example, for captures and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Variant {
    /// All six clusters.
    #[default]
    Acme,
    /// `core-fra` alone, as with one connection open.
    Single,
    /// Kargo refused in `core-fra`: its applications may be missing.
    Partial,
    /// Every source refused everywhere.
    Refused,
    /// Neither Kargo nor Argo CD served, and no labelled workloads.
    Unserved,
    /// No cluster answered.
    Failed,
    /// Argo CD's Applications stopped at the page cap in `core-fra`.
    Capped,
}

impl Variant {
    /// `single`, `partial`, `refused`, `unserved`, `failed` or `capped`
    /// from `FRESHKUBE_APPLICATIONS`; debug and stress builds only.
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
            _ => Self::Acme,
        }
    }
}

fn refused<T>(what: &str) -> Source<T> {
    Source::Refused(format!("{what} is forbidden for this identity"))
}

fn core(inputs: &mut Inputs) -> &mut SessionInputs {
    inputs
        .sessions
        .iter_mut()
        .find(|s| s.key.0 == acme::CORE)
        .expect("acme has core-fra")
}

pub(crate) fn inputs(variant: Variant) -> Inputs {
    let mut inputs = acme::acme();
    for session in &mut inputs.sessions {
        if session.key.0 == acme::CORE {
            session.argo_scope = ArgoScope::Namespace(ARGOCD_NAMESPACE.into());
        }
    }
    match variant {
        Variant::Acme => {}
        Variant::Single => inputs.sessions.retain(|s| s.key.0 == acme::CORE),
        Variant::Partial => core(&mut inputs).kargo = refused("projects.kargo.akuity.io"),
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
                *session = SessionInputs::unread(session.key.clone(), &why);
            }
        }
        Variant::Capped => {
            let core = core(&mut inputs);
            if let Source::Read(apps) = &core.argo_applications {
                core.argo_applications = Source::Capped(apps.clone(), Truncation { read: 500 });
            }
        }
    }
    inputs
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
