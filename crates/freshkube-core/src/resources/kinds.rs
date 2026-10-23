//! Built-in resource kinds and the API paths they live at.

/// A listable resource: its API group, version, kind, plural and scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceKind {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub namespaced: bool,
}

impl ResourceKind {
    pub fn new(group: &str, version: &str, kind: &str, plural: &str, namespaced: bool) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            kind: kind.into(),
            plural: plural.into(),
            namespaced,
        }
    }

    /// The kubectl-style resource name: `pods`, `deployments.apps`.
    pub fn key(&self) -> String {
        if self.group.is_empty() {
            self.plural.clone()
        } else {
            format!("{}.{}", self.plural, self.group)
        }
    }

    pub fn api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }

    fn prefix(&self) -> String {
        if self.group.is_empty() {
            format!("/api/{}", self.version)
        } else {
            format!("/apis/{}/{}", self.group, self.version)
        }
    }

    /// The collection URL, across all namespaces when `namespace` is `None`.
    pub fn collection_path(&self, namespace: Option<&str>) -> String {
        match namespace.filter(|_| self.namespaced) {
            Some(namespace) => format!("{}/namespaces/{namespace}/{}", self.prefix(), self.plural),
            None => format!("{}/{}", self.prefix(), self.plural),
        }
    }

    pub fn object_path(&self, namespace: Option<&str>, name: &str) -> String {
        format!("{}/{name}", self.collection_path(namespace))
    }

    /// Core v1 Secrets, whose values are hidden until revealed one by one.
    pub fn is_secret(&self) -> bool {
        self.group.is_empty() && self.plural == "secrets"
    }

    /// Core v1 Pods, which have containers with logs.
    pub fn is_pod(&self) -> bool {
        self.group.is_empty() && self.plural == "pods"
    }
}

/// Built-in kinds that Freshkube's navigation offers, by kubectl key.
/// Custom resources come from discovery instead.
pub fn builtin(key: &str) -> Option<ResourceKind> {
    BUILTIN
        .iter()
        .find(|(group, _, _, plural, _)| {
            if group.is_empty() {
                *plural == key
            } else {
                key.strip_prefix(plural)
                    .and_then(|rest| rest.strip_prefix('.'))
                    == Some(group)
            }
        })
        .map(|(group, version, kind, plural, namespaced)| {
            ResourceKind::new(group, version, kind, plural, *namespaced)
        })
}

/// A built-in kind by what a person types: the kubectl key (`deployments.apps`),
/// a plural (`deployments`) or a kind in any case (`Deployment`). A name
/// that two groups share, such as `events`, is not guessed: `None`.
pub fn find(name: &str) -> Option<ResourceKind> {
    if let Some(kind) = builtin(name) {
        return Some(kind);
    }
    let wanted = name.to_ascii_lowercase();
    let mut found = BUILTIN
        .iter()
        .filter(|(_, _, kind, plural, _)| *plural == wanted || kind.to_ascii_lowercase() == wanted);
    let first = found.next()?;
    found.next().is_none().then(|| {
        let (group, version, kind, plural, namespaced) = first;
        ResourceKind::new(group, version, kind, plural, *namespaced)
    })
}

/// kubectl's short names for the built-in kinds, by plural.
const SHORT_NAMES: [(&str, &str); 21] = [
    ("po", "pods"),
    ("deploy", "deployments"),
    ("rs", "replicasets"),
    ("ds", "daemonsets"),
    ("sts", "statefulsets"),
    ("cj", "cronjobs"),
    ("svc", "services"),
    ("ing", "ingresses"),
    ("ns", "namespaces"),
    ("no", "nodes"),
    ("cm", "configmaps"),
    ("pv", "persistentvolumes"),
    ("pvc", "persistentvolumeclaims"),
    ("sa", "serviceaccounts"),
    ("ev", "events"),
    ("hpa", "horizontalpodautoscalers"),
    ("pdb", "poddisruptionbudgets"),
    ("netpol", "networkpolicies"),
    ("sc", "storageclasses"),
    ("quota", "resourcequotas"),
    ("limits", "limitranges"),
];

/// Splits what kubectl takes (`pod`, `Deployment.apps`, `po`) into the name
/// and the group it names, if any. A name has no dot, so the first one
/// separates them.
pub(crate) fn split_name(asked: &str) -> (String, Option<String>) {
    let asked = asked.to_ascii_lowercase();
    match asked.split_once('.') {
        Some((name, group)) => (name.to_owned(), Some(group.to_owned())),
        None => (asked, None),
    }
}

/// Whether `name` (lower case, no group) is a plural, the kind, or a short
/// name of `kind`, whose short names are known only for built-ins.
pub(crate) fn names_kind(kind: &ResourceKind, name: &str, short: bool) -> bool {
    kind.plural == name
        || kind.kind.to_ascii_lowercase() == name
        || (short
            && SHORT_NAMES
                .iter()
                .any(|(alias, plural)| *alias == name && *plural == kind.plural))
}

/// A built-in kind by anything kubectl takes: the key, a plural, a singular
/// (the kind in any case) or a short name, with an optional `.group`. A name
/// that two kinds share, such as `events` in two groups, is not guessed.
pub fn find_named(asked: &str) -> Option<ResourceKind> {
    if let Some(kind) = builtin(asked) {
        return Some(kind);
    }
    let (name, group) = split_name(asked);
    let mut found = BUILTIN
        .iter()
        .map(|(group, version, kind, plural, namespaced)| {
            ResourceKind::new(group, version, kind, plural, *namespaced)
        })
        .filter(|kind| group.as_deref().is_none_or(|group| kind.group == group))
        .filter(|kind| names_kind(kind, &name, true));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Resolve a built-in owner reference by group and kind, keeping its named version.
pub fn builtin_by_gvk(api_version: &str, kind: &str) -> Option<ResourceKind> {
    let (group, version) = api_version.split_once('/').unwrap_or(("", api_version));
    BUILTIN
        .iter()
        .find(|(known, _, name, _, _)| *known == group && *name == kind)
        .map(|(_, _, kind, plural, namespaced)| {
            ResourceKind::new(group, version, kind, plural, *namespaced)
        })
}

const BUILTIN: [(&str, &str, &str, &str, bool); 47] = [
    // Cluster
    ("", "v1", "Node", "nodes", false),
    ("", "v1", "Event", "events", true),
    ("", "v1", "Namespace", "namespaces", false),
    ("coordination.k8s.io", "v1", "Lease", "leases", true),
    // Workloads
    ("apps", "v1", "Deployment", "deployments", true),
    ("", "v1", "Pod", "pods", true),
    ("apps", "v1", "ReplicaSet", "replicasets", true),
    ("apps", "v1", "DaemonSet", "daemonsets", true),
    ("apps", "v1", "StatefulSet", "statefulsets", true),
    ("batch", "v1", "Job", "jobs", true),
    ("batch", "v1", "CronJob", "cronjobs", true),
    // Networking
    ("", "v1", "Service", "services", true),
    ("networking.k8s.io", "v1", "Ingress", "ingresses", true),
    (
        "discovery.k8s.io",
        "v1",
        "EndpointSlice",
        "endpointslices",
        true,
    ),
    (
        "networking.k8s.io",
        "v1",
        "NetworkPolicy",
        "networkpolicies",
        true,
    ),
    (
        "networking.k8s.io",
        "v1",
        "IngressClass",
        "ingressclasses",
        false,
    ),
    // Configuration
    ("", "v1", "Secret", "secrets", true),
    ("", "v1", "ConfigMap", "configmaps", true),
    (
        "autoscaling",
        "v2",
        "HorizontalPodAutoscaler",
        "horizontalpodautoscalers",
        true,
    ),
    ("", "v1", "LimitRange", "limitranges", true),
    ("", "v1", "ResourceQuota", "resourcequotas", true),
    (
        "policy",
        "v1",
        "PodDisruptionBudget",
        "poddisruptionbudgets",
        true,
    ),
    // Storage
    ("", "v1", "PersistentVolume", "persistentvolumes", false),
    (
        "",
        "v1",
        "PersistentVolumeClaim",
        "persistentvolumeclaims",
        true,
    ),
    (
        "storage.k8s.io",
        "v1",
        "VolumeAttachment",
        "volumeattachments",
        false,
    ),
    (
        "storage.k8s.io",
        "v1",
        "StorageClass",
        "storageclasses",
        false,
    ),
    ("storage.k8s.io", "v1", "CSIDriver", "csidrivers", false),
    ("storage.k8s.io", "v1", "CSINode", "csinodes", false),
    // Access control
    ("", "v1", "ServiceAccount", "serviceaccounts", true),
    ("rbac.authorization.k8s.io", "v1", "Role", "roles", true),
    (
        "rbac.authorization.k8s.io",
        "v1",
        "RoleBinding",
        "rolebindings",
        true,
    ),
    (
        "rbac.authorization.k8s.io",
        "v1",
        "ClusterRole",
        "clusterroles",
        false,
    ),
    (
        "rbac.authorization.k8s.io",
        "v1",
        "ClusterRoleBinding",
        "clusterrolebindings",
        false,
    ),
    // Administration
    (
        "apiextensions.k8s.io",
        "v1",
        "CustomResourceDefinition",
        "customresourcedefinitions",
        false,
    ),
    (
        "scheduling.k8s.io",
        "v1",
        "PriorityClass",
        "priorityclasses",
        false,
    ),
    ("node.k8s.io", "v1", "RuntimeClass", "runtimeclasses", false),
    (
        "admissionregistration.k8s.io",
        "v1",
        "MutatingWebhookConfiguration",
        "mutatingwebhookconfigurations",
        false,
    ),
    (
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingWebhookConfiguration",
        "validatingwebhookconfigurations",
        false,
    ),
    (
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingAdmissionPolicy",
        "validatingadmissionpolicies",
        false,
    ),
    (
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingAdmissionPolicyBinding",
        "validatingadmissionpolicybindings",
        false,
    ),
    // Further kinds reachable by key, not shown in navigation by default.
    ("", "v1", "Endpoints", "endpoints", true),
    (
        "",
        "v1",
        "ReplicationController",
        "replicationcontrollers",
        true,
    ),
    ("", "v1", "PodTemplate", "podtemplates", true),
    (
        "apps",
        "v1",
        "ControllerRevision",
        "controllerrevisions",
        true,
    ),
    ("events.k8s.io", "v1", "Event", "events", true),
    (
        "certificates.k8s.io",
        "v1",
        "CertificateSigningRequest",
        "certificatesigningrequests",
        false,
    ),
    (
        "flowcontrol.apiserver.k8s.io",
        "v1",
        "FlowSchema",
        "flowschemas",
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_kubectl_takes_finds_its_built_in_kind_with_or_without_a_group() {
        for asked in ["po", "pod", "Pod", "pods", "PODS"] {
            assert_eq!(find_named(asked).unwrap().key(), "pods", "{asked}");
        }
        for asked in [
            "deploy",
            "Deployment.apps",
            "deployment.apps",
            "deploy.apps",
        ] {
            assert_eq!(
                find_named(asked).unwrap().key(),
                "deployments.apps",
                "{asked}"
            );
        }
        assert_eq!(find_named("svc").unwrap().key(), "services");
        // The wrong group, an unknown name and a group alone are not kinds.
        assert!(find_named("deployment.batch").is_none());
        assert!(find_named("widgets").is_none());
        assert!(find_named(".apps").is_none());
    }

    #[test]
    fn a_kind_is_found_by_key_plural_or_kind_and_never_guessed() {
        assert_eq!(find("deployments.apps").unwrap().key(), "deployments.apps");
        assert_eq!(find("deployments").unwrap().key(), "deployments.apps");
        assert_eq!(find("Deployment").unwrap().key(), "deployments.apps");
        assert_eq!(find("pods").unwrap().key(), "pods");
        assert!(find("nonsense").is_none());
    }

    #[test]
    fn keys_and_paths_follow_kubectl_and_api_conventions() {
        let pods = builtin("pods").unwrap();
        assert_eq!(pods.key(), "pods");
        assert_eq!(pods.collection_path(None), "/api/v1/pods");
        assert_eq!(
            pods.object_path(Some("kube-system"), "coredns-1"),
            "/api/v1/namespaces/kube-system/pods/coredns-1"
        );
        let deployments = builtin("deployments.apps").unwrap();
        assert_eq!(deployments.api_version(), "apps/v1");
        assert_eq!(
            deployments.collection_path(Some("default")),
            "/apis/apps/v1/namespaces/default/deployments"
        );
        // Cluster-scoped kinds ignore a namespace.
        let nodes = builtin("nodes").unwrap();
        assert_eq!(nodes.collection_path(Some("default")), "/api/v1/nodes");
        // Same plural in two groups resolves by group.
        assert_eq!(builtin("events").unwrap().group, "");
        assert_eq!(
            builtin("events.events.k8s.io").unwrap().group,
            "events.k8s.io"
        );
        assert!(builtin("unknown").is_none());
    }
}
