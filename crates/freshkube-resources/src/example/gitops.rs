//! A Flux Kustomization for the example cluster (#857): the object Reconcile
//! acts on. Example data sends nothing, so nothing here changes.

use super::objects::{metadata, timestamp};
use super::*;

/// (namespace, name, path, suspended) of the example Kustomizations.
const KUSTOMIZATIONS: [(&str, &str, &str, bool); 2] = [
    ("flux-system", "apps", "./apps/production", false),
    ("flux-system", "infrastructure", "./infrastructure", true),
];

pub(super) fn columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Ready", ColumnKind::Text, false),
        ResourceColumn::new("Suspended", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

pub(super) fn rows(connection: &str, now: i64) -> Vec<ResourceRow> {
    KUSTOMIZATIONS
        .iter()
        .enumerate()
        .map(|(ix, (namespace, name, _, suspended))| ResourceRow {
            identity: identity(
                connection,
                "kustomizations.kustomize.toolkit.fluxcd.io",
                namespace,
                name,
                ix,
            ),
            cells: vec![
                (*name).into(),
                "True".into(),
                if *suspended { "True" } else { "False" }.into(),
                String::new(),
            ],
            created: Some(now - 45 * 86_400 + ix as i64 * 86_400),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        })
        .collect()
}

pub(super) fn yaml(row: &ResourceRow, ix: usize, created: i64) -> String {
    let (_, _, path, suspended) = KUSTOMIZATIONS[ix % KUSTOMIZATIONS.len()];
    let mut yaml = metadata(
        "Kustomization",
        "kustomize.toolkit.fluxcd.io/v1",
        row,
        "  generation: 1\n",
    );
    yaml.push_str(&format!(
        "spec:\n  interval: 10m\n  path: {path}\n  prune: true\n  suspend: {suspended}\n  sourceRef:\n    kind: GitRepository\n    name: fleet\nstatus:\n  lastHandledReconcileAt: '{}'\n  conditions:\n  - type: Ready\n    status: 'True'\n    reason: ReconciliationSucceeded\n    message: Applied revision main@sha1:3f9c2ab\n    lastTransitionTime: '{}'\n",
        timestamp(created + 3_600),
        timestamp(created + 3_600),
    ));
    yaml
}
