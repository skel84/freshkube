//! Which custom kinds the API server offers, read from its discovery
//! documents.
//!
//! `/apis` lists every API group in one light request. Leaving out the
//! groups the API server serves itself leaves custom resource definitions'
//! groups and aggregated APIs. A group's kinds are read when it is opened,
//! from every version it serves, so one broken aggregated API fails alone
//! and a kind served only by an older version still shows.

use std::collections::HashSet;

use futures::future::join_all;
use kube::Client;
use serde::Deserialize;

use super::failure::{Failure, FailureKind};
use super::kinds::ResourceKind;
use super::object::{JSON, json_get};
use super::table::nullable;

/// Groups the API server serves itself; the sidebar's other groups show
/// their kinds. Every other group is a custom resource definition's or an
/// aggregated API's.
const SERVER_GROUPS: [&str; 22] = [
    "admissionregistration.k8s.io",
    "apiextensions.k8s.io",
    "apiregistration.k8s.io",
    "apps",
    "authentication.k8s.io",
    "authorization.k8s.io",
    "autoscaling",
    "batch",
    "certificates.k8s.io",
    "coordination.k8s.io",
    "discovery.k8s.io",
    "events.k8s.io",
    "extensions",
    "flowcontrol.apiserver.k8s.io",
    "internal.apiserver.k8s.io",
    "networking.k8s.io",
    "node.k8s.io",
    "policy",
    "rbac.authorization.k8s.io",
    "resource.k8s.io",
    "scheduling.k8s.io",
    "storage.k8s.io",
];

/// An API group the API server doesn't serve itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiGroup {
    pub name: String,
    /// Served versions, the preferred one first.
    pub versions: Vec<String>,
}

/// What a group offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupKinds {
    /// Kinds that can be listed and watched, by kind name, each at the most
    /// preferred version that serves it.
    pub kinds: Vec<ResourceKind>,
    /// Kinds that can't be listed and watched, such as metrics that can only
    /// be read.
    pub unlistable: Vec<String>,
    /// Versions whose discovery failed while another's succeeded.
    pub failures: Vec<(String, Failure)>,
}

#[derive(Deserialize)]
struct GroupList {
    #[serde(default, deserialize_with = "nullable")]
    groups: Vec<GroupEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupEntry {
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    versions: Vec<VersionEntry>,
    #[serde(default)]
    preferred_version: Option<VersionEntry>,
}

#[derive(Deserialize)]
struct VersionEntry {
    version: String,
}

#[derive(Deserialize)]
struct ResourceList {
    #[serde(default, deserialize_with = "nullable")]
    resources: Vec<ResourceEntry>,
}

#[derive(Deserialize)]
struct ResourceEntry {
    name: String,
    kind: String,
    #[serde(default)]
    namespaced: bool,
    #[serde(default, deserialize_with = "nullable")]
    verbs: Vec<String>,
}

/// The groups that aren't the API server's own, by name.
pub async fn list_custom_groups(client: &Client) -> Result<Vec<ApiGroup>, Failure> {
    let list: GroupList = client
        .request(json_get("/apis".into(), JSON)?)
        .await
        .map_err(Failure::from_kube)?;
    Ok(custom_groups(list))
}

/// Reads every version of `group` at once and merges their kinds. Fails only
/// when no version could be read, with the preferred version's failure.
pub async fn list_group_kinds(client: &Client, group: &ApiGroup) -> Result<GroupKinds, Failure> {
    let reads = group.versions.iter().map(|version| async move {
        let path = format!("/apis/{}/{version}", group.name);
        let list = match json_get(path, JSON) {
            Ok(request) => client
                .request::<ResourceList>(request)
                .await
                .map_err(Failure::from_kube),
            Err(failure) => Err(failure),
        };
        (version.clone(), list)
    });
    group_kinds(&group.name, join_all(reads).await)
}

fn custom_groups(list: GroupList) -> Vec<ApiGroup> {
    let mut groups: Vec<ApiGroup> = list
        .groups
        .into_iter()
        .filter(|group| !SERVER_GROUPS.contains(&group.name.as_str()))
        .filter_map(|group| {
            let listed = group.versions.into_iter().map(|entry| entry.version);
            let mut versions: Vec<String> = Vec::new();
            for version in group
                .preferred_version
                .map(|entry| entry.version)
                .into_iter()
                .chain(listed)
            {
                if !versions.contains(&version) {
                    versions.push(version);
                }
            }
            (!versions.is_empty()).then_some(ApiGroup {
                name: group.name,
                versions,
            })
        })
        .collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}

/// Merges the versions' kinds in preference order: a kind is taken from the
/// first version that serves it.
fn group_kinds(
    group: &str,
    reads: Vec<(String, Result<ResourceList, Failure>)>,
) -> Result<GroupKinds, Failure> {
    let mut merged = GroupKinds::default();
    let mut seen = HashSet::new();
    let mut read_any = false;
    for (version, list) in reads {
        let list = match list {
            Ok(list) => list,
            Err(failure) => {
                merged.failures.push((version, failure));
                continue;
            }
        };
        read_any = true;
        for resource in list.resources {
            // Subresources (`pods/log`) and kindless endpoints aren't kinds.
            if resource.name.contains('/')
                || resource.kind.is_empty()
                || !seen.insert(resource.name.clone())
            {
                continue;
            }
            let can = |verb: &str| resource.verbs.iter().any(|item| item == verb);
            if can("list") && can("watch") {
                merged.kinds.push(ResourceKind::new(
                    group,
                    &version,
                    &resource.kind,
                    &resource.name,
                    resource.namespaced,
                ));
            } else {
                merged.unlistable.push(resource.kind);
            }
        }
    }
    if !read_any {
        return Err(match merged.failures.into_iter().next() {
            Some((_, failure)) => failure,
            None => Failure::new(FailureKind::NotFound, format!("{group} serves no versions")),
        });
    }
    merged.kinds.sort_by(|a, b| a.kind.cmp(&b.kind));
    merged.unlistable.sort();
    merged.unlistable.dedup();
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_leave_out_the_servers_own_and_put_the_preferred_version_first() {
        let list: GroupList = serde_json::from_str(
            r#"{"kind":"APIGroupList","apiVersion":"v1","groups":[
                {"name":"apps","versions":[{"groupVersion":"apps/v1","version":"v1"}],
                 "preferredVersion":{"groupVersion":"apps/v1","version":"v1"}},
                {"name":"monitoring.coreos.com","versions":[
                   {"groupVersion":"monitoring.coreos.com/v1","version":"v1"},
                   {"groupVersion":"monitoring.coreos.com/v1alpha1","version":"v1alpha1"}],
                 "preferredVersion":{"groupVersion":"monitoring.coreos.com/v1","version":"v1"}},
                {"name":"cilium.io","versions":[
                   {"groupVersion":"cilium.io/v2alpha1","version":"v2alpha1"},
                   {"groupVersion":"cilium.io/v2","version":"v2"}],
                 "preferredVersion":{"groupVersion":"cilium.io/v2","version":"v2"}},
                {"name":"gateway.networking.k8s.io","versions":[
                   {"groupVersion":"gateway.networking.k8s.io/v1","version":"v1"}]},
                {"name":"empty.example.com","versions":null}]}"#,
        )
        .unwrap();
        let groups = custom_groups(list);
        let names: Vec<_> = groups.iter().map(|group| group.name.as_str()).collect();
        // A CRD group under k8s.io is still custom; only the server's own
        // groups are left out.
        assert_eq!(
            names,
            [
                "cilium.io",
                "gateway.networking.k8s.io",
                "monitoring.coreos.com"
            ]
        );
        assert_eq!(groups[0].versions, ["v2", "v2alpha1"]);
        assert_eq!(groups[1].versions, ["v1"]);
        assert_eq!(groups[2].versions, ["v1", "v1alpha1"]);
    }

    fn resources(json: &str) -> Result<ResourceList, Failure> {
        Ok(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn kinds_merge_across_versions_preferring_the_first() {
        let preferred = resources(
            r#"{"kind":"APIResourceList","groupVersion":"cilium.io/v2","resources":[
                {"name":"ciliumnetworkpolicies","singularName":"ciliumnetworkpolicy","namespaced":true,
                 "kind":"CiliumNetworkPolicy","verbs":["delete","get","list","patch","watch"]},
                {"name":"ciliumnetworkpolicies/status","namespaced":true,"kind":"CiliumNetworkPolicy",
                 "verbs":["get","patch","update"]},
                {"name":"ciliumnodes","namespaced":false,"kind":"CiliumNode",
                 "verbs":["get","list","watch"]},
                {"name":"version","namespaced":false,"kind":"","verbs":[]}]}"#,
        );
        let older = resources(
            r#"{"kind":"APIResourceList","groupVersion":"cilium.io/v2alpha1","resources":[
                {"name":"ciliumnodes","namespaced":false,"kind":"CiliumNode","verbs":["get","list","watch"]},
                {"name":"ciliumloadbalancerippools","namespaced":false,
                 "kind":"CiliumLoadBalancerIPPool","verbs":["get","list","watch"]}]}"#,
        );
        let kinds = group_kinds(
            "cilium.io",
            vec![("v2".into(), preferred), ("v2alpha1".into(), older)],
        )
        .unwrap();
        let found: Vec<_> = kinds
            .kinds
            .iter()
            .map(|kind| (kind.kind.as_str(), kind.version.as_str(), kind.namespaced))
            .collect();
        assert_eq!(
            found,
            [
                ("CiliumLoadBalancerIPPool", "v2alpha1", false),
                ("CiliumNetworkPolicy", "v2", true),
                ("CiliumNode", "v2", false),
            ]
        );
        assert_eq!(kinds.kinds[1].key(), "ciliumnetworkpolicies.cilium.io");
        assert_eq!(
            kinds.kinds[0].collection_path(None),
            "/apis/cilium.io/v2alpha1/ciliumloadbalancerippools"
        );
        assert!(kinds.unlistable.is_empty());
        assert!(kinds.failures.is_empty());
    }

    #[test]
    fn unlistable_kinds_and_failed_versions_are_reported() {
        let metrics = resources(
            r#"{"resources":[
                {"name":"nodes","namespaced":false,"kind":"NodeMetrics","verbs":["get","list"]},
                {"name":"pods","namespaced":true,"kind":"PodMetrics","verbs":["get","list"]}]}"#,
        );
        let kinds = group_kinds("metrics.k8s.io", vec![("v1beta1".into(), metrics)]).unwrap();
        assert!(kinds.kinds.is_empty());
        assert_eq!(kinds.unlistable, ["NodeMetrics", "PodMetrics"]);

        let unavailable = Failure::new(
            FailureKind::Other,
            "the server is currently unable to handle the request",
        );
        let partial = group_kinds(
            "example.com",
            vec![
                (
                    "v1".into(),
                    resources(r#"{"resources":[{"name":"widgets","kind":"Widget","namespaced":true,"verbs":["list","watch"]}]}"#),
                ),
                ("v1beta1".into(), Err(unavailable.clone())),
            ],
        )
        .unwrap();
        assert_eq!(partial.kinds.len(), 1);
        assert_eq!(
            partial.failures,
            [("v1beta1".to_owned(), unavailable.clone())]
        );

        let forbidden = Failure::new(FailureKind::Forbidden, "forbidden");
        let failed = group_kinds(
            "example.com",
            vec![
                ("v1".into(), Err(forbidden.clone())),
                ("v1beta1".into(), Err(unavailable)),
            ],
        );
        assert_eq!(failed, Err(forbidden));
    }
}
