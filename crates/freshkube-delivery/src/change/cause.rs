//! Why a hop or a link can't be confirmed (docs/DESIGN.md, "The change
//! page"): one cause for every Unknown, the evidence that would confirm it,
//! and the one next step it offers. A cause is decided where the join or the
//! derive knows it, from the [`Source`](crate::source::Source) variant or the
//! setup, never from a sentence; a site that can't tell leaves its hop
//! without one.

/// The verbs of a read-only Role: what an access request asks for.
pub(crate) const READ_VERBS: [&str; 3] = ["get", "list", "watch"];

/// A kind of object a read-only Role names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    /// The API group; empty for the core group.
    pub group: String,
    /// The kind's plural, as the API serves it.
    pub resource: String,
    /// The namespace it was read in; `None` for a read across all of them.
    pub namespace: Option<String>,
}

impl Role {
    pub fn new(group: &str, resource: &str, namespace: Option<&str>) -> Self {
        Self {
            group: group.into(),
            resource: resource.into(),
            namespace: namespace.map(str::to_owned),
        }
    }

    /// The Role (or ClusterRole, for all namespaces) a person would ask an
    /// administrator for: read-only, and only this kind.
    // The Copy access request button of the Why / Needs / Fix block copies it.
    #[allow(dead_code)]
    pub(crate) fn manifest(&self) -> String {
        let (kind, scope) = match &self.namespace {
            Some(namespace) => ("Role", format!("\n  namespace: {namespace}")),
            None => ("ClusterRole", String::new()),
        };
        format!(
            "apiVersion: rbac.authorization.k8s.io/v1\nkind: {kind}\nmetadata:\n  name: freshkube-read-{}{scope}\nrules:\n  - apiGroups: [\"{}\"]\n    resources: [\"{}\"]\n    verbs: [{}]\n",
            self.resource.replace('/', "-"),
            self.group,
            self.resource,
            READ_VERBS.map(|verb| format!("\"{verb}\"")).join(", ")
        )
    }
}

/// A field of Settings: the section to open and the setup file's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsField {
    /// The Settings page's slug, such as `settings-sources`.
    pub section: &'static str,
    /// The key in `freshkube.yaml`, such as `delivery.build_namespace`.
    pub field: &'static str,
}

impl SettingsField {
    pub const BUILD_NAMESPACE: Self = Self {
        section: "settings-sources",
        field: "delivery.build_namespace",
    };
    pub const STAGE_NAMING: Self = Self {
        section: "settings-sources",
        field: "delivery.stage_naming",
    };
    pub const DESTINATIONS: Self = Self {
        section: "settings-destinations",
        field: "destinations",
    };
}

/// The one next step a cause offers. None changes anything in a cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// The Freight's page in Kargo; the page itself never promotes.
    OpenInKargo,
    /// Copy the read-only Role that would let the source answer.
    CopyAccessRequest(Role),
    /// Read it again; the failure may pass.
    Retry,
    /// Open Settings at the field that tells Freshkube where to look.
    OpenSettings(SettingsField),
    /// Look at the sources the read went through.
    CheckSources,
}

/// Why a hop or a link is Unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Not reached yet, and nothing is wrong.
    Waiting { needs: String },
    /// A source answered no: refused, unreachable or timed out.
    CantRead {
        needs: String,
        /// What an access request asks for, when the source named the kind
        /// that was refused; `None` when the whole cluster didn't answer.
        role: Option<Role>,
        /// Whether reading again may help: not after a refusal.
        retry: bool,
    },
    /// Freshkube wasn't told where to look.
    NotSetUp { needs: String, field: SettingsField },
    /// It looked, and the piece isn't there.
    NotFound { needs: String },
}

impl Cause {
    pub fn waiting(needs: impl Into<String>) -> Self {
        Self::Waiting {
            needs: needs.into(),
        }
    }

    pub fn not_found(needs: impl Into<String>) -> Self {
        Self::NotFound {
            needs: needs.into(),
        }
    }

    pub fn not_set_up(needs: impl Into<String>, field: SettingsField) -> Self {
        Self::NotSetUp {
            needs: needs.into(),
            field,
        }
    }

    /// A refused read can't be tried again until access is granted; any
    /// other failure may pass.
    pub fn cant_read(needs: impl Into<String>, role: Option<Role>, refused: bool) -> Self {
        Self::CantRead {
            needs: needs.into(),
            role,
            retry: !refused,
        }
    }

    /// The cause in the words the page shows.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Waiting { .. } => "Waiting",
            Self::CantRead { .. } => "Can't read",
            Self::NotSetUp { .. } => "Not set up",
            Self::NotFound { .. } => "Not found",
        }
    }

    /// The evidence that would confirm it.
    pub fn needs(&self) -> &str {
        match self {
            Self::Waiting { needs }
            | Self::CantRead { needs, .. }
            | Self::NotSetUp { needs, .. }
            | Self::NotFound { needs } => needs,
        }
    }

    /// Waiting is no problem: it counts with neither warnings nor unknowns.
    pub fn is_problem(&self) -> bool {
        !matches!(self, Self::Waiting { .. })
    }

    /// The steps it offers, in the order the page shows them.
    pub fn next(&self) -> Vec<Next> {
        match self {
            Self::Waiting { .. } => vec![Next::OpenInKargo],
            Self::CantRead { role, retry, .. } => role
                .iter()
                .cloned()
                .map(Next::CopyAccessRequest)
                .chain(retry.then_some(Next::Retry))
                .collect(),
            Self::NotSetUp { field, .. } => vec![Next::OpenSettings(*field)],
            Self::NotFound { .. } => vec![Next::CheckSources],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cause_offers_its_own_next_step() {
        let role = Role::new("kargo.akuity.io", "stages", Some("checkout"));
        assert_eq!(
            Cause::waiting("a Promotion").next(),
            vec![Next::OpenInKargo]
        );
        assert_eq!(
            Cause::cant_read("the list", Some(role.clone()), false).next(),
            vec![Next::CopyAccessRequest(role.clone()), Next::Retry]
        );
        // A refusal can't pass by reading again.
        assert_eq!(
            Cause::cant_read("the list", Some(role.clone()), true).next(),
            vec![Next::CopyAccessRequest(role)]
        );
        assert_eq!(
            Cause::cant_read("the list", None, false).next(),
            vec![Next::Retry]
        );
        assert_eq!(
            Cause::cant_read("the list", None, true).next(),
            Vec::<Next>::new()
        );
        assert_eq!(
            Cause::not_set_up("a build", SettingsField::BUILD_NAMESPACE).next(),
            vec![Next::OpenSettings(SettingsField::BUILD_NAMESPACE)]
        );
        assert_eq!(Cause::not_found("a run").next(), vec![Next::CheckSources]);
    }

    #[test]
    fn only_waiting_is_no_problem() {
        assert!(!Cause::waiting("x").is_problem());
        assert!(Cause::not_found("x").is_problem());
        assert!(Cause::cant_read("x", None, false).is_problem());
        assert!(Cause::not_set_up("x", SettingsField::STAGE_NAMING).is_problem());
    }

    #[test]
    fn an_access_request_is_read_only_and_names_its_scope() {
        let namespaced = Role::new("", "pods", Some("shop")).manifest();
        assert!(namespaced.contains("kind: Role\n"), "{namespaced}");
        assert!(namespaced.contains("namespace: shop"));
        assert!(namespaced.contains("verbs: [\"get\", \"list\", \"watch\"]"));
        assert!(!namespaced.contains("create"));
        let wide = Role::new("argoproj.io", "applications", None).manifest();
        assert!(wide.contains("kind: ClusterRole"), "{wide}");
        assert!(wide.contains("apiGroups: [\"argoproj.io\"]"));
    }
}
