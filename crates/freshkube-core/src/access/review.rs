//! The reads: who you are, a namespace's rules, the Nodes row's reviews and
//! the namespace names. Each is one GET or one review `create`, which the
//! API server answers without storing anything.

use k8s_openapi::api::{
    authentication::v1::SelfSubjectReview,
    authorization::v1::{
        NonResourceAttributes, ResourceAttributes, SelfSubjectAccessReview,
        SelfSubjectAccessReviewSpec, SelfSubjectRulesReview, SelfSubjectRulesReviewSpec,
    },
    core::v1::Namespace,
};
use kube::{
    Api,
    api::{ListParams, PostParams},
};

use super::{COLUMNS, Cell, Cells, NAMESPACE_CAP, Need, Rule, RulesAnswer};
use crate::{failure::Failure, write::scrub};

/// Who the cluster says you are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    pub username: String,
    pub groups: Vec<String>,
}

fn failed(error: kube::Error) -> String {
    scrub(&Failure::from_kube(error).message)
}

/// One `SelfSubjectReview`. A server older than 1.28 does not serve it, and
/// that reads as an error the page shows as "unknown", never as no access.
pub async fn who(client: &kube::Client) -> Result<Who, String> {
    let api: Api<SelfSubjectReview> = Api::all(client.clone());
    let answer = api
        .create(&PostParams::default(), &SelfSubjectReview::default())
        .await
        .map_err(failed)?;
    let user = answer
        .status
        .and_then(|status| status.user_info)
        .ok_or_else(|| "the review came back empty".to_owned())?;
    Ok(Who {
        username: user.username.unwrap_or_default(),
        groups: user.groups.unwrap_or_default(),
    })
}

/// One `SelfSubjectRulesReview` of `namespace`.
pub async fn rules(client: &kube::Client, namespace: &str) -> Result<Cells, String> {
    let review = SelfSubjectRulesReview {
        spec: SelfSubjectRulesReviewSpec {
            namespace: Some(namespace.to_owned()),
        },
        ..Default::default()
    };
    let api: Api<SelfSubjectRulesReview> = Api::all(client.clone());
    let answer = api
        .create(&PostParams::default(), &review)
        .await
        .map_err(failed)?;
    let status = answer
        .status
        .ok_or_else(|| "the review came back empty".to_owned())?;
    Ok(RulesAnswer {
        rules: status
            .resource_rules
            .iter()
            .map(Rule::from_resource_rule)
            .collect(),
        incomplete: status.incomplete,
        evaluation_error: status
            .evaluation_error
            .filter(|words| !words.is_empty())
            .map(|words| scrub(&words)),
    }
    .cells())
}

/// The Nodes row. A rules review cannot tell a cluster-wide grant from a
/// RoleBinding to a ClusterRole, so each need is asked of the server as the
/// exact review, with no namespace. Any review that does not answer makes
/// the row "couldn't check" as a whole.
pub async fn nodes(client: &kube::Client) -> Result<Cells, String> {
    let mut cells = Vec::new();
    for column in COLUMNS {
        let Some(needs) = column.node_needs() else {
            cells.push(Cell::NotApplicable);
            continue;
        };
        let mut denied = Vec::new();
        for need in needs {
            if !allowed(client, need).await? {
                denied.push(*need);
            }
        }
        cells.push(if denied.is_empty() {
            Cell::Yes
        } else {
            Cell::No(denied)
        });
    }
    Ok(Cells {
        cells,
        incomplete: false,
        note: None,
    })
}

async fn allowed(client: &kube::Client, need: &Need) -> Result<bool, String> {
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: Some(need.group.to_owned()),
                resource: Some(need.resource.to_owned()),
                verb: Some(need.verb.to_owned()),
                subresource: need.subresource.map(str::to_owned),
                ..Default::default()
            }),
            non_resource_attributes: None::<NonResourceAttributes>,
        },
        ..Default::default()
    };
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    let answer = api
        .create(&PostParams::default(), &review)
        .await
        .map_err(failed)?;
    answer
        .status
        .map(|status| status.allowed)
        .ok_or_else(|| "the review came back empty".to_owned())
}

/// The namespace names, one list of metadata, at most [`NAMESPACE_CAP`].
/// Returns whether the cluster has more.
pub async fn list_namespaces(client: &kube::Client) -> Result<(Vec<String>, bool), String> {
    let api: Api<Namespace> = Api::all(client.clone());
    let list = api
        .list_metadata(&ListParams::default().limit(NAMESPACE_CAP as u32 + 1))
        .await
        .map_err(failed)?;
    let mut names: Vec<String> = list
        .items
        .into_iter()
        .filter_map(|item| item.metadata.name)
        .collect();
    let more = names.len() > NAMESPACE_CAP;
    names.sort();
    names.truncate(NAMESPACE_CAP);
    Ok((names, more))
}
