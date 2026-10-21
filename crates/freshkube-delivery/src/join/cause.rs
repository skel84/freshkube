//! The causes the join decides from a source's answer or the setup.

use crate::change::{Cause, Role};
use crate::source::Source;

/// A kind of object a join reads: its API group and plural.
#[derive(Clone, Copy)]
pub(crate) struct Kind {
    group: &'static str,
    resource: &'static str,
}

pub(crate) const STAGES: Kind = Kind {
    group: crate::kargo::GROUP,
    resource: "stages",
};
pub(crate) const PROMOTIONS: Kind = Kind {
    group: crate::kargo::GROUP,
    resource: "promotions",
};
pub(crate) const FREIGHT: Kind = Kind {
    group: crate::kargo::GROUP,
    resource: "freights",
};
pub(crate) const WAREHOUSES: Kind = Kind {
    group: crate::kargo::GROUP,
    resource: "warehouses",
};
pub(crate) const APPLICATIONS: Kind = Kind {
    group: crate::argocd::GROUP,
    resource: "applications",
};
pub(crate) const PIPELINE_RUNS: Kind = Kind {
    group: "tekton.dev",
    resource: "pipelineruns",
};
pub(crate) const ROLLOUTS: Kind = Kind {
    group: "argoproj.io",
    resource: "rollouts",
};
pub(crate) const DEPLOYMENTS: Kind = Kind {
    group: "apps",
    resource: "deployments",
};
pub(crate) const PODS: Kind = Kind {
    group: "",
    resource: "pods",
};

/// Why a source gave nothing, or `None` when it was read. A refusal can
/// only be fixed with access to this kind, so it names the Role and offers
/// no retry; a kind not served is not there to find; anything else may
/// pass, and no Role would help. `needs` is what the read would show.
pub(crate) fn unread<T>(
    source: &Source<T>,
    kind: Kind,
    namespace: Option<&str>,
    needs: impl Into<String>,
) -> Option<Cause> {
    match source {
        Source::Read(_) | Source::Capped(..) => None,
        Source::NotInstalled(_) => Some(Cause::not_found(needs)),
        Source::Refused(_) => Some(Cause::cant_read(
            needs,
            Some(Role::new(kind.group, kind.resource, namespace)),
            true,
        )),
        Source::Unreadable(_) => Some(Cause::cant_read(needs, None, false)),
    }
}

/// Like [`unread`] for a source outside the cluster, such as GitHub, where
/// no Role would help: a refusal names no kind and offers no retry.
pub(crate) fn unread_outside<T>(source: &Source<T>, needs: impl Into<String>) -> Option<Cause> {
    match source {
        Source::Read(_) | Source::Capped(..) => None,
        Source::NotInstalled(_) => Some(Cause::not_found(needs)),
        Source::Refused(_) => Some(Cause::cant_read(needs, None, true)),
        Source::Unreadable(_) => Some(Cause::cant_read(needs, None, false)),
    }
}
