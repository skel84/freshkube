//! The access review: ask the API server whether this person may do a thing
//! before it is offered (`SelfSubjectAccessReview`). The cache keyed by the
//! access identity belongs to #854; a write asks afresh each time.

use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use kube::{Api, api::PostParams};

use super::action::Target;

/// What the review answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    /// The server said no, with its reason if it gave one.
    Denied {
        reason: String,
    },
    /// The review itself failed, so nothing is known.
    Unknown {
        message: String,
    },
}

/// Asks whether `verb` is allowed on `target`.
pub async fn review(client: &kube::Client, target: &Target, verb: &str) -> Access {
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: Some(target.group.clone()),
                resource: Some(target.resource.clone()),
                verb: Some(verb.to_owned()),
                namespace: target.namespace.clone(),
                name: Some(target.name.clone()),
                subresource: target.subresource.clone(),
                version: Some(target.version.clone()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    match api.create(&PostParams::default(), &review).await {
        Ok(answer) => match answer.status {
            Some(status) if status.allowed => Access::Allowed,
            Some(status) => Access::Denied {
                reason: status
                    .reason
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "the cluster did not allow it".to_owned()),
            },
            None => Access::Unknown {
                message: "the review came back empty".to_owned(),
            },
        },
        Err(error) => Access::Unknown {
            message: super::audit::scrub(&error.to_string()),
        },
    }
}
