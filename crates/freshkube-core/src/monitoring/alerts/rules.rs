//! PrometheusRule objects, read as plain objects, and the Rules page's read.
use kube::api::{Api, ApiResource, DynamicObject, ListParams};
use kube::core::GroupVersionKind;

use super::super::{ErrorKind, QueryError};
use super::model::{RuleGroup, RuleHealth};
use super::parse;
use super::read::{ReadFrom, RulesData, Scope, Sources, unknown};
use super::state::Unknown;
use crate::resources::{Failure, FailureKind};

/// PrometheusRule objects read for the page at most.
const MAX_OBJECTS: u32 = 500;

fn api(client: &kube::Client) -> Api<DynamicObject> {
    let resource = ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk("monitoring.coreos.com", "v1", "PrometheusRule"),
        "prometheusrules",
    );
    Api::all_with(client.clone(), &resource)
}

fn failed(error: kube::Error) -> QueryError {
    let failure = Failure::from_kube(error);
    let kind = match failure.kind {
        FailureKind::Forbidden => ErrorKind::Refused,
        FailureKind::Unauthorized => ErrorKind::Unauthorized,
        FailureKind::NotFound => ErrorKind::NotFound,
        FailureKind::Timeout => ErrorKind::TimedOut,
        FailureKind::Config | FailureKind::Unreachable | FailureKind::Other => {
            ErrorKind::Unavailable
        }
    };
    QueryError::new(
        kind,
        format!("Couldn't list PrometheusRules: {}", failure.message),
    )
}

/// Whether the cluster holds any PrometheusRule. A cluster without the
/// definition, or an identity that may not list it, says no.
pub async fn any_objects(client: &kube::Client) -> Result<bool, QueryError> {
    match api(client).list(&ListParams::default().limit(1)).await {
        Ok(list) => Ok(!list.items.is_empty()),
        Err(error) => match failed(error) {
            error if error.kind == ErrorKind::NotFound || error.is_refused() => Ok(false),
            error => Err(error),
        },
    }
}

/// Every PrometheusRule's groups, at most [`MAX_OBJECTS`] objects, and
/// whether more were left.
pub async fn objects(client: &kube::Client) -> Result<(Vec<RuleGroup>, bool), QueryError> {
    let list = api(client)
        .list(&ListParams::default().limit(MAX_OBJECTS))
        .await
        .map_err(failed)?;
    let more = list.metadata.continue_.is_some();
    let mut groups = Vec::new();
    for object in &list.items {
        if let Ok(value) = serde_json::to_value(object) {
            groups.extend(parse::prometheus_rule(&value));
        }
    }
    Ok((groups, more))
}

/// The Rules page's read. A server that evaluates rules (vmalert) gives
/// each rule's health and last evaluation; PrometheusRule objects alone give
/// the definitions with Unknown health, never Ok. In a central store the
/// evaluator's groups aren't scoped to one cluster, so only this cluster's
/// objects are shown.
pub async fn page(
    sources: &Sources,
    scope: &Scope,
    own: Option<&kube::Client>,
) -> Result<RulesData, Unknown> {
    let mut note = None;
    let mut evaluated = false;
    let mut groups = Vec::new();
    let mut read: Vec<ReadFrom> = Vec::new();
    if let Some(vmalert) = &sources.vmalert {
        if scope.matcher().is_some() {
            note = Some(
                "The central vmalert's rules aren't scoped to one cluster, so only this \
                 cluster's PrometheusRule objects are shown"
                    .to_owned(),
            );
        } else {
            groups = vmalert.rules().await.map_err(unknown)?;
            evaluated = true;
            read = sources.read();
        }
    }
    if !evaluated && let Some(client) = own {
        let (defined, more) = match objects(client).await {
            Ok(objects) => objects,
            Err(error) if error.kind == ErrorKind::NotFound => (Vec::new(), false),
            Err(error) => return Err(unknown(error)),
        };
        if more {
            note = Some(format!(
                "Only the first {MAX_OBJECTS} PrometheusRule objects are shown"
            ));
        }
        groups = defined;
    }
    if groups.is_empty() && !evaluated {
        return Err(Unknown::NotFound(
            "No vmalert was found and the cluster holds no PrometheusRule".into(),
        ));
    }
    if !evaluated {
        debug_assert!(
            groups
                .iter()
                .flat_map(|group| &group.rules)
                .all(|rule| rule.health == RuleHealth::Unknown)
        );
        note.get_or_insert_with(|| {
            "No vmalert evaluates these rules here, so their health is Unknown".to_owned()
        });
    }
    Ok(RulesData {
        groups,
        evaluated,
        read,
        note,
    })
}
