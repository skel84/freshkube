//! Invented ESO objects; no Secret or provider configuration is needed.
use super::*;
use freshkube_core::resources::{Condition, ExternalSecret, StoreRead, StoreStatus};

pub(crate) const KEY: &str = "externalsecrets.external-secrets.io";
const CASES: &[(&str, &str, &str)] = &[
    ("lantern-api", "SecretSynced", "Secret was synced"),
    (
        "old-refresh",
        "SecretSynced",
        "Last reported sync; compare the refresh time and policy",
    ),
    (
        "broken-store",
        "SecretSyncedError",
        "SecretStore meadow-broken is not ready",
    ),
    (
        "missing-entry",
        "SecretSyncedError",
        "Provider key lantern/config was not found",
    ),
    (
        "access-denied",
        "SecretSyncedError",
        "Access denied to provider key lantern/config",
    ),
    (
        "owned-elsewhere",
        "SecretSyncedError",
        "Target Secret is owned by another controller",
    ),
    ("unknown-status", "", ""),
    ("refused-store", "SecretSynced", "Secret was synced"),
    (
        "missing-store",
        "SecretSyncedError",
        "Referenced store is missing",
    ),
    ("cluster-store", "SecretSynced", "Secret was synced"),
    ("on-change", "SecretSynced", "Secret was synced"),
    ("created-once", "SecretSynced", "Secret was synced"),
];

pub(super) fn read(connection: &str, now: i64) -> (Vec<ResourceColumn>, Vec<ResourceRow>) {
    let columns = vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Status", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ];
    let rows = CASES
        .iter()
        .enumerate()
        .map(|(ix, (name, reason, _))| ResourceRow {
            identity: identity(connection, KEY, "paper-kites", name, ix),
            cells: vec![(*name).into(), (*reason).into(), String::new()],
            created: Some(now - 86400),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        })
        .collect();
    (columns, rows)
}

pub(super) fn yaml(row: &ResourceRow, now: i64) -> String {
    let name = row.identity.name.as_str();
    let (_, reason, message) = CASES.iter().find(|(n, _, _)| *n == name).unwrap();
    let store = match name {
        "broken-store" => "meadow-broken",
        "refused-store" => "meadow-refused",
        "missing-store" => "meadow-missing",
        "cluster-store" => "shared-meadow",
        _ => "meadow-store",
    };
    let policy = match name {
        "on-change" => "OnChange",
        "created-once" => "CreatedOnce",
        _ => "Periodic",
    };
    let condition = if reason.is_empty() {
        String::new()
    } else {
        format!(
            "  conditions:\n  - type: Ready\n    status: '{}'\n    reason: {reason}\n    message: {message}\n",
            if *reason == "SecretSynced" {
                "True"
            } else {
                "False"
            }
        )
    };
    let refreshed = if reason.is_empty() {
        String::new()
    } else {
        format!(
            "  refreshTime: '{}'\n",
            objects::timestamp(
                now - if matches!(name, "old-refresh" | "on-change" | "created-once") {
                    86400
                } else {
                    60
                }
            )
        )
    };
    format!(
        "{}spec:\n  refreshInterval: 1h\n  refreshPolicy: {policy}\n  secretStoreRef:\n    kind: {}\n    name: {store}\n  target:\n    name: {name}\nstatus:\n{refreshed}{condition}",
        objects::metadata("ExternalSecret", "external-secrets.io/v1", row, ""),
        if name == "cluster-store" {
            "ClusterSecretStore"
        } else {
            "SecretStore"
        }
    )
}

pub(crate) fn stores(secret: &ExternalSecret) -> Vec<StoreRead> {
    secret
        .stores
        .iter()
        .map(|store| {
            let status = match store.name.as_str() {
                "meadow-refused" => Err(Failure::new(
                    FailureKind::Forbidden,
                    "Invented viewer cannot get secretstores",
                )),
                "meadow-missing" => Err(Failure::new(
                    FailureKind::NotFound,
                    "SecretStore meadow-missing not found",
                )),
                _ => Ok(StoreStatus::new(Some(Condition {
                    kind: "Ready".into(),
                    status: if store.name == "meadow-broken" {
                        "False"
                    } else {
                        "True"
                    }
                    .into(),
                    reason: if store.name == "meadow-broken" {
                        "InvalidProviderConfig"
                    } else {
                        "Valid"
                    }
                    .into(),
                    message: if store.name == "meadow-broken" {
                        "Store is not ready"
                    } else {
                        "Store validated"
                    }
                    .into(),
                    changed: None,
                }))),
            };
            StoreRead::new(store.clone(), status)
        })
        .collect()
}
