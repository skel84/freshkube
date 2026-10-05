//! The Kubernetes part of the sidebar, grouped as Kubeli groups it.

pub(crate) struct NavGroup {
    pub(crate) label: &'static str,
    /// Element id suffix: `nav-k8s-group-<slug>`.
    pub(crate) slug: &'static str,
    /// (label, kubectl resource key).
    pub(crate) items: &'static [(&'static str, &'static str)],
}

pub(crate) const NAVIGATION: [NavGroup; 6] = [
    NavGroup {
        label: "Workloads",
        slug: "workloads",
        items: &[
            ("Pods", "pods"),
            ("Deployments", "deployments.apps"),
            ("ReplicaSets", "replicasets.apps"),
            ("DaemonSets", "daemonsets.apps"),
            ("StatefulSets", "statefulsets.apps"),
            ("Jobs", "jobs.batch"),
            ("CronJobs", "cronjobs.batch"),
        ],
    },
    NavGroup {
        label: "Networking",
        slug: "networking",
        items: &[
            ("Services", "services"),
            ("Ingresses", "ingresses.networking.k8s.io"),
            ("EndpointSlices", "endpointslices.discovery.k8s.io"),
            ("Network Policies", "networkpolicies.networking.k8s.io"),
            ("Ingress Classes", "ingressclasses.networking.k8s.io"),
        ],
    },
    NavGroup {
        label: "Configuration",
        slug: "configuration",
        items: &[
            ("ConfigMaps", "configmaps"),
            ("Secrets", "secrets"),
            (
                "Horizontal Pod Autoscalers",
                "horizontalpodautoscalers.autoscaling",
            ),
            ("Limit Ranges", "limitranges"),
            ("Resource Quotas", "resourcequotas"),
            ("Pod Disruption Budgets", "poddisruptionbudgets.policy"),
        ],
    },
    NavGroup {
        label: "Storage",
        slug: "storage",
        items: &[
            ("Persistent Volume Claims", "persistentvolumeclaims"),
            ("Persistent Volumes", "persistentvolumes"),
            ("Storage Classes", "storageclasses.storage.k8s.io"),
            ("Volume Attachments", "volumeattachments.storage.k8s.io"),
            ("CSI Drivers", "csidrivers.storage.k8s.io"),
            ("CSI Nodes", "csinodes.storage.k8s.io"),
        ],
    },
    NavGroup {
        label: "Access Control",
        slug: "access-control",
        items: &[
            ("Service Accounts", "serviceaccounts"),
            ("Roles", "roles.rbac.authorization.k8s.io"),
            ("Role Bindings", "rolebindings.rbac.authorization.k8s.io"),
            ("Cluster Roles", "clusterroles.rbac.authorization.k8s.io"),
            (
                "Cluster Role Bindings",
                "clusterrolebindings.rbac.authorization.k8s.io",
            ),
        ],
    },
    NavGroup {
        label: "Administration",
        slug: "administration",
        items: &[
            (
                "Custom Resource Definitions",
                "customresourcedefinitions.apiextensions.k8s.io",
            ),
            ("Priority Classes", "priorityclasses.scheduling.k8s.io"),
            ("Runtime Classes", "runtimeclasses.node.k8s.io"),
            (
                "Mutating Webhooks",
                "mutatingwebhookconfigurations.admissionregistration.k8s.io",
            ),
            (
                "Validating Webhooks",
                "validatingwebhookconfigurations.admissionregistration.k8s.io",
            ),
            (
                "Validating Admission Policies",
                "validatingadmissionpolicies.admissionregistration.k8s.io",
            ),
            (
                "Validating Admission Policy Bindings",
                "validatingadmissionpolicybindings.admissionregistration.k8s.io",
            ),
            ("Leases", "leases.coordination.k8s.io"),
        ],
    },
];

/// The page a fresh window shows for Kubernetes.
pub(crate) const CLUSTER: [(&str, &str); 3] = [
    ("Nodes", "nodes"),
    ("Namespaces", "namespaces"),
    ("Events", "events"),
];

pub(crate) const DEFAULT_KIND: &str = "pods";

/// The navigation's own copy of a resource key, if it offers that kind.
#[cfg(any(debug_assertions, feature = "stress", test))]
pub(crate) fn known(key: &str) -> Option<&'static str> {
    if let Some((_, key)) = CLUSTER.iter().find(|(_, item)| *item == key) {
        return Some(*key);
    }
    NAVIGATION
        .iter()
        .flat_map(|group| group.items.iter())
        .find(|(_, item)| *item == key)
        .map(|(_, item)| *item)
}

/// The navigation label for a resource key.
pub(crate) fn label(key: &str) -> Option<&'static str> {
    if let Some((label, _)) = CLUSTER.iter().find(|(_, item)| *item == key) {
        return Some(*label);
    }
    NAVIGATION
        .iter()
        .flat_map(|group| group.items.iter())
        .find(|(_, item)| *item == key)
        .map(|(label, _)| *label)
}

/// The group a resource key sits in.
pub(crate) fn group_of(key: &str) -> Option<&'static str> {
    NAVIGATION
        .iter()
        .find(|group| group.items.iter().any(|(_, item)| *item == key))
        .map(|group| group.slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_core::resources::builtin;

    #[test]
    fn every_navigation_item_is_a_known_kind() {
        let mut keys = std::collections::HashSet::new();
        for group in &NAVIGATION {
            for (label, key) in group.items {
                assert!(builtin(key).is_some(), "{label}: {key}");
                assert!(keys.insert(*key), "duplicate {key}");
            }
        }
        for (_, key) in CLUSTER {
            assert!(builtin(key).is_some());
            assert!(keys.insert(key));
        }
        assert_eq!(keys.len(), 40);
        assert_eq!(label(DEFAULT_KIND), Some("Pods"));
        assert_eq!(known("pods"), Some("pods"));
        assert_eq!(known("pod"), None);
        assert_eq!(group_of("csinodes.storage.k8s.io"), Some("storage"));
        assert_eq!(group_of("nope"), None);
    }
}
