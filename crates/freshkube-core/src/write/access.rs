//! The access review: ask the API server whether this person may do a thing
//! before it is offered (`SelfSubjectAccessReview`). A write asks afresh each
//! time: this module's [`review`] is the gate. [`AccessQuestion`] is what an
//! action declares it needs, so a page can ask before offering it
//! (`access::offer` caches the answers, #854); it only guides.

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
                // RBAC cannot name an object that does not exist yet.
                name: (verb != "create").then(|| target.name.clone()),
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

/// What an action needs, as the one `SelfSubjectAccessReview` that would
/// answer it: a verb on a resource (with its group and subresource) in a
/// namespace, on a named object. The action declares it in core, beside its
/// tier, and a page never builds the review ([`AccessQuestion::of`],
/// `ActionDescription::access_questions`, `Everyday::needs`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccessQuestion {
    pub verb: &'static str,
    /// The API group; empty for the core group.
    pub group: String,
    pub resource: String,
    pub subresource: Option<String>,
    pub namespace: Option<String>,
    /// The object. `None` for `create`: RBAC cannot name an object that does
    /// not exist yet, so the gate does not either.
    pub name: Option<String>,
}

impl AccessQuestion {
    /// The question the gate asks of `verb` on `target`.
    pub fn of(target: &Target, verb: &'static str) -> Self {
        Self {
            verb,
            group: target.group.clone(),
            resource: target.resource.clone(),
            subresource: target.subresource.clone(),
            namespace: target.namespace.clone(),
            name: (verb != "create").then(|| target.name.clone()),
        }
    }

    /// The short form of a no, as Your access words it: "Needs patch on
    /// deployments in shop".
    pub fn reason(&self) -> String {
        let resource = match &self.subresource {
            Some(sub) => format!("{}/{sub}", self.resource),
            None => self.resource.clone(),
        };
        match &self.namespace {
            Some(namespace) => format!("Needs {} on {resource} in {namespace}", self.verb),
            None => format!("Needs {} on {resource} cluster-wide", self.verb),
        }
    }
}

/// Asks `question` of the API server. The same review as [`review`], from the
/// declared question; the answer guides, and the gate still asks afresh.
pub async fn ask(client: &kube::Client, question: &AccessQuestion) -> Access {
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: Some(question.group.clone()),
                resource: Some(question.resource.clone()),
                verb: Some(question.verb.to_owned()),
                namespace: question.namespace.clone(),
                name: question.name.clone(),
                subresource: question.subresource.clone(),
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
