//! What the everyday actions (#857) need the example cluster to hold to show
//! themselves: a CronJob to suspend or resume, an autoscaler that targets one
//! Deployment (Scale's notice), and a Deployment Argo CD tracks (the GitOps
//! warning). Example data sends nothing, so none of it changes anything.

use super::objects::{metadata, timestamp};
use super::*;

/// (namespace, name, schedule, suspended) of the example CronJobs.
const CRONJOBS: [(&str, &str, &str, bool); 1] = [("batch", "nightly-report", "0 2 * * *", false)];

/// The Deployment an example autoscaler targets, and the autoscaler's name.
const AUTOSCALED: (&str, &str, &str) = ("payments", "api", "api");

/// The Deployment the example Argo CD tracks.
const GITOPS: (&str, &str) = ("payments", "ledger");

pub(super) fn cronjob_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Schedule", ColumnKind::Text, false),
        ResourceColumn::new("Suspend", ColumnKind::Text, false),
        ResourceColumn::new("Active", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

pub(super) fn cronjobs(connection: &str, now: i64) -> Vec<ResourceRow> {
    CRONJOBS
        .iter()
        .enumerate()
        .map(|(ix, (namespace, name, schedule, suspended))| ResourceRow {
            identity: identity(connection, "cronjobs.batch", namespace, name, ix),
            cells: vec![
                (*name).into(),
                (*schedule).into(),
                if *suspended { "True" } else { "False" }.into(),
                "0".into(),
                String::new(),
            ],
            created: Some(now - 30 * 86_400 + ix as i64 * 86_400),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        })
        .collect()
}

pub(super) fn cronjob_yaml(row: &ResourceRow, ix: usize, created: i64) -> String {
    let (_, _, schedule, suspended) = CRONJOBS[ix % CRONJOBS.len()];
    let mut yaml = metadata("CronJob", "batch/v1", row, "  generation: 1\n");
    yaml.push_str(&format!(
        "spec:\n  schedule: '{schedule}'\n  suspend: {suspended}\n  concurrencyPolicy: Forbid\n  jobTemplate:\n    spec:\n      template:\n        spec:\n          restartPolicy: Never\n          containers:\n          - name: report\n            image: batch/report:0.4.1\nstatus:\n  lastScheduleTime: '{}'\n",
        timestamp(created + 86_400),
    ));
    yaml
}

/// The annotation that marks the example Deployment Argo CD tracks, for its
/// document; empty for any other.
pub(super) fn gitops_annotation(namespace: &str, app: &str) -> &'static str {
    if (namespace, app) == GITOPS {
        "    argocd.argoproj.io/tracking-id: payments:apps/Deployment:payments/ledger\n"
    } else {
        ""
    }
}

/// The name of the autoscaler that targets this example object, if one does.
pub fn autoscaler_of(identity: &ResourceIdentity) -> Option<&'static str> {
    (identity.resource == "deployments.apps"
        && (identity.namespace.as_str(), identity.name.as_str()) == (AUTOSCALED.0, AUTOSCALED.1))
        .then_some(AUTOSCALED.2)
}

/// The example object behind an identity as JSON, the way the cluster would
/// return it to a write, so an action can be described from it.
pub fn object_json(identity: &ResourceIdentity, now: i64) -> Option<serde_json::Value> {
    let document = super::document(identity, now)?;
    serde_yaml::from_str(&document.yaml).ok()
}
