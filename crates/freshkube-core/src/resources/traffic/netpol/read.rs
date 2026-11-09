//! The reads behind the NetworkPolicy views: lists and gets only, each
//! bounded. None writes and none reads a Secret.
use super::applies::{Applying, applying};
use super::facts::PolicyFacts;
use super::select::{Match, selector_matches};
use crate::failure::{Failure, FailureKind};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::{Api, Client, api::ListParams};
use std::collections::BTreeMap;

pub const POLICY_LIMIT: u32 = 500;
pub const POD_LIMIT: u32 = 500;
pub const NAMESPACE_LIMIT: u32 = 500;

fn is_cut<T>(list: &kube::core::ObjectList<T>) -> bool
where
    T: Clone + serde::de::DeserializeOwned + kube::Resource,
{
    list.metadata
        .continue_
        .as_deref()
        .is_some_and(|token| !token.is_empty())
}

/// The policies of one namespace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PoliciesRead {
    pub policies: Vec<PolicyFacts>,
    /// The list stopped at its limit: a pod may be governed by one not read.
    pub cut: bool,
}

pub async fn read_policies(client: &Client, namespace: &str) -> Result<PoliciesRead, Failure> {
    let list = Api::<NetworkPolicy>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().limit(POLICY_LIMIT))
        .await
        .map_err(Failure::from_kube)?;
    Ok(PoliciesRead {
        cut: is_cut(&list),
        policies: list.items.iter().map(PolicyFacts::from_policy).collect(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceLabels {
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NamespacesRead {
    pub namespaces: Vec<NamespaceLabels>,
    pub cut: bool,
}

/// Namespace labels, for a namespaceSelector. A cut list can't answer for a
/// namespace that is missing from it, so callers treat `cut` as unread.
pub async fn read_namespaces(client: &Client) -> Result<NamespacesRead, Failure> {
    let list = Api::<Namespace>::all(client.clone())
        .list(&ListParams::default().limit(NAMESPACE_LIMIT))
        .await
        .map_err(Failure::from_kube)?;
    Ok(NamespacesRead {
        cut: is_cut(&list),
        namespaces: list
            .items
            .iter()
            .map(|n| NamespaceLabels {
                name: n.metadata.name.clone().unwrap_or_default(),
                labels: n.metadata.labels.clone().unwrap_or_default(),
            })
            .collect(),
    })
}

/// How many pods in the policy's namespace a policy selects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPods {
    pub selected: usize,
    /// Pods whose labels the selector could not be judged on.
    pub unknown: usize,
    pub total: usize,
    /// The pod list stopped at its limit, so the count is at least this.
    pub cut: bool,
}

pub fn count_selected(policy: &PolicyFacts, pods: &[Pod], cut: bool) -> SelectedPods {
    let mut selected = 0;
    let mut unknown = 0;
    for pod in pods {
        let labels = pod.metadata.labels.clone().unwrap_or_default();
        match selector_matches(&policy.pod_selector, &labels) {
            Match::Yes => selected += 1,
            Match::Unknown => unknown += 1,
            Match::No => {}
        }
    }
    SelectedPods {
        selected,
        unknown,
        total: pods.len(),
        cut,
    }
}

pub async fn read_selected_pods(
    client: &Client,
    policy: &PolicyFacts,
) -> Result<SelectedPods, Failure> {
    let list = Api::<Pod>::namespaced(client.clone(), &policy.namespace)
        .list(&ListParams::default().limit(POD_LIMIT))
        .await
        .map_err(Failure::from_kube)?;
    Ok(count_selected(policy, &list.items, is_cut(&list)))
}

/// One policy as read: itself and the pods it selects. A refused pod list
/// leaves the count unknown and leaves the policy as read.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PolicyReading {
    pub policy: PolicyFacts,
    pub pods: Result<SelectedPods, Failure>,
}

impl PolicyReading {
    pub fn new(policy: PolicyFacts, pods: Result<SelectedPods, Failure>) -> Self {
        Self { policy, pods }
    }
}

pub async fn read_policy_by_name(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<PolicyReading, Failure> {
    let policy = Api::<NetworkPolicy>::namespaced(client.clone(), namespace)
        .get(name)
        .await
        .map_err(Failure::from_kube)?;
    let policy = PolicyFacts::from_policy(&policy);
    let pods = read_selected_pods(client, &policy).await;
    Ok(PolicyReading { policy, pods })
}

/// The policies that apply to a pod, or to the pods a workload's template
/// makes, with each direction's combined verdict. A refused or cut policy
/// list is an error: "no policy" is never inferred from it.
pub async fn read_applying(
    client: &Client,
    namespace: &str,
    labels: &BTreeMap<String, String>,
) -> Result<Applying, Failure> {
    let read = read_policies(client, namespace).await?;
    if read.cut {
        return Err(Failure::new(
            FailureKind::Other,
            format!("NetworkPolicy list cut at {POLICY_LIMIT}"),
        ));
    }
    Ok(applying(&read.policies, namespace, labels))
}
