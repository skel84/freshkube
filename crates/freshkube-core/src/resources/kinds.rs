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
