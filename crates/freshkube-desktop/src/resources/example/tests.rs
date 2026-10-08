use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::HashSet;

#[test]
fn example_kinds_read_with_unique_identities_and_matching_cells() {
    for context in fixture::CONTEXTS {
        for key in KINDS {
            let (columns, rows) = read(context, key, None, TEST_NOW).unwrap();
            assert!(!rows.is_empty(), "{context} {key}");
            let unique: HashSet<_> = rows.iter().map(|row| &row.identity).collect();
            assert_eq!(unique.len(), rows.len(), "{context} {key}");
            for row in &rows {
                assert_eq!(row.cells.len(), columns.len(), "{key}");
                assert_eq!(row.cells[0], row.identity.name);
                assert_eq!(row.identity.connection, connection(context));
            }
        }
    }
    assert!(read("prod-fra", "configmaps", None, TEST_NOW).is_none());
}

#[test]
fn every_example_object_reads_in_full_with_its_events() {
    for key in KINDS {
        let (_, rows) = read("homelab", key, None, TEST_NOW).unwrap();
        for row in &rows {
            let identity = &row.identity;
            let document = document(identity, TEST_NOW)
                .unwrap_or_else(|| panic!("{key} {}", identity.address()));
            assert_eq!(document.uid, identity.uid);
            assert_eq!(document.name, identity.name);
            assert_eq!(document.namespace.unwrap_or_default(), identity.namespace);
            assert_eq!(document.resource_version, row.resource_version);
            assert!(document.overview.created.is_some(), "{key}");
            for event in events(identity, TEST_NOW) {
                assert!(event.uid.starts_with(&identity.uid));
                assert!(event.last_seen >= event.first_seen);
            }
        }
    }
    // Not an example object, so nothing to read.
    let mut stranger = pod_rows(1).remove(0).identity;
    stranger.connection = connection("homelab");
    assert!(document(&stranger, TEST_NOW).is_none());
    assert!(events(&stranger, TEST_NOW).is_empty());

    let (_, pods) = read("prod-fra", "pods", None, TEST_NOW).unwrap();
    let crashing = pods
        .iter()
        .find(|row| row.cells[2] == "CrashLoopBackOff")
        .unwrap();
    assert!(
        events(&crashing.identity, TEST_NOW)
            .iter()
            .any(ObjectEvent::is_warning)
    );
}

#[test]
fn example_secrets_hide_values_until_one_is_revealed() {
    let (_, secrets) = read("homelab", "secrets", Some("payments"), TEST_NOW).unwrap();
    let identity = &secrets[0].identity;
    let document = document(identity, TEST_NOW).unwrap();
    assert!(!document.yaml.contains("example-only-password"));
    assert!(
        !document
            .yaml
            .contains(&STANDARD.encode(b"example-only-password"))
    );
    let keys = &document.overview.secret.unwrap().keys;
    assert_eq!(keys.len(), 3);
    assert_eq!(
        secret_value(identity, "password", TEST_NOW),
        Some(SecretValue::Text("example-only-password".into()))
    );
    assert_eq!(
        secret_value(identity, "keystore.p12", TEST_NOW),
        Some(SecretValue::Binary(6))
    );
    assert_eq!(secret_value(identity, "missing", TEST_NOW), None);
}

#[test]
fn example_discovery_covers_every_group_state() {
    let groups = custom_groups();
    assert_eq!(groups.len(), CUSTOM_GROUPS.len() + acme::GROUPS.len());
    let names: Vec<_> = groups.iter().map(|group| group.name.as_str()).collect();
    assert!(names.is_sorted(), "{names:?}");
    let cilium = groups
        .iter()
        .find(|group| group.name == "cilium.io")
        .unwrap();
    assert_eq!(cilium.versions, ["v2", "v2alpha1"]);
    let certs = group_kinds("cert-manager.io").unwrap();
    assert_eq!(certs.kinds.len(), 4);
    assert!(certs.failures.is_empty());
    assert_eq!(
        kind("certificates.cert-manager.io"),
        Some(certs.kinds[0].clone())
    );
    assert_eq!(kind("pods"), builtin("pods"));
    assert_eq!(kind("widgets.example.com"), None);
    assert_eq!(
        group_kinds("monitoring.coreos.com").unwrap().failures.len(),
        2
    );
    assert_eq!(
        group_kinds("traefik.containo.us").unwrap_err().kind,
        FailureKind::NotFound
    );
    assert!(group_kinds("metrics.k8s.io").unwrap().kinds.is_empty());
    assert_eq!(
        group_kinds("velero.io").unwrap_err().kind,
        FailureKind::Forbidden
    );
    assert_eq!(
        group_kinds("external.metrics.k8s.io").unwrap_err().kind,
        FailureKind::Other
    );
    assert_eq!(
        group_kinds("gone.example.com").unwrap_err().kind,
        FailureKind::NotFound
    );
    // A namespace narrows a namespaced custom kind too.
    let (_, payments) = read(
        "homelab",
        "certificates.cert-manager.io",
        Some("payments"),
        TEST_NOW,
    )
    .unwrap();
    assert_eq!(payments.len(), 2);
}

#[test]
fn a_namespace_reads_only_its_objects() {
    let (_, all) = read("prod-fra", "pods", None, TEST_NOW).unwrap();
    let (_, payments) = read("prod-fra", "pods", Some("payments"), TEST_NOW).unwrap();
    assert!(!payments.is_empty() && payments.len() < all.len());
    assert!(
        payments
            .iter()
            .all(|row| row.identity.namespace == "payments")
    );
    let statuses: HashSet<_> = all.iter().map(|row| row.cells[2].as_str()).collect();
    for status in ["Running", "CrashLoopBackOff", "Pending", "Completed"] {
        assert!(statuses.contains(status), "{status}");
    }
    // A namespace doesn't narrow cluster-scoped kinds.
    let (_, nodes) = read("prod-fra", "nodes", Some("payments"), TEST_NOW).unwrap();
    assert_eq!(nodes.len(), fixture::kubernetes_nodes("prod-fra").len());
}

#[test]
fn summary_matches_the_example_lists_and_owner_chain() {
    for context in fixture::CONTEXTS {
        let summary = summary(context, TEST_NOW);
        assert_eq!(
            summary.pods.loaded().unwrap().total,
            read(context, "pods", None, TEST_NOW).unwrap().1.len()
        );
        assert_eq!(summary.namespaces.loaded(), Some(&namespaces().len()));
        assert_eq!(
            summary.claims.loaded().unwrap().pending[0].name,
            "report-data"
        );
        let (_, rows) = read(context, "replicasets.apps", None, TEST_NOW).unwrap();
        let (_, deployments) = read(context, "deployments.apps", None, TEST_NOW).unwrap();
        for row in rows {
            let object = document(&row.identity, TEST_NOW).unwrap();
            let owner = &object.overview.owners[0];
            assert_eq!(owner.kind, "Deployment");
            assert!(
                deployments
                    .iter()
                    .any(
                        |deployment| deployment.identity.namespace == row.identity.namespace
                            && deployment.identity.name == owner.name
                            && deployment.identity.uid == owner.uid
                    )
            );
        }
    }
}

#[test]
fn long_context_keeps_the_homelab_shape() {
    let long = "talos-production-frankfurt-equinix-fr5-baremetal-b7";
    assert_eq!(
        read(long, "pods", None, TEST_NOW).unwrap().1.len(),
        read("homelab", "pods", None, TEST_NOW).unwrap().1.len()
    );
    assert_eq!(
        read(long, "nodes", None, TEST_NOW).unwrap().1.len(),
        read("homelab", "nodes", None, TEST_NOW).unwrap().1.len()
    );
}

#[test]
fn acme_core_objects_list_and_read_in_full_on_every_context() {
    let keys = acme::KINDS
        .iter()
        .map(|(group, _, _, plural, _)| format!("{plural}.{group}"));
    for context in fixture::CONTEXTS {
        for key in keys.clone() {
            let (_, rows) = read(context, &key, None, TEST_NOW).unwrap();
            assert!(!rows.is_empty(), "{key}");
            let identities: std::collections::BTreeSet<_> =
                rows.iter().map(|row| row.identity.uid.clone()).collect();
            assert_eq!(identities.len(), rows.len(), "{key}");
            for row in &rows {
                assert_eq!(row.cells[0], row.identity.name);
                let document = document(&row.identity, TEST_NOW)
                    .unwrap_or_else(|| panic!("{key} {}", row.identity.address()));
                assert_eq!(document.uid, row.identity.uid);
                assert_eq!(
                    document.namespace.unwrap_or_default(),
                    row.identity.namespace
                );
                assert!(events(&row.identity, TEST_NOW).is_empty());
            }
        }
    }
    let stage = read(
        "prod-fra",
        "stages.kargo.akuity.io",
        Some("checkout"),
        TEST_NOW,
    )
    .unwrap()
    .1;
    let names: Vec<_> = stage.iter().map(|row| row.identity.name.as_str()).collect();
    assert_eq!(names, ["dev", "stage", "prod-ams", "prod-lon"]);
    let projects = read(
        "prod-fra",
        "projects.kargo.akuity.io",
        Some("checkout"),
        TEST_NOW,
    )
    .unwrap()
    .1;
    assert_eq!(projects.len(), 2, "Projects are cluster-scoped");
}

#[test]
fn acme_status_page_is_a_deployment_of_its_own_beside_the_examples() {
    let (_, rows) = read("homelab", "deployments.apps", None, TEST_NOW).unwrap();
    assert_eq!(rows.len(), WORKLOADS.len() + 1);
    assert_eq!(
        rows[..WORKLOADS.len()]
            .iter()
            .map(|row| row.identity.clone())
            .collect::<Vec<_>>(),
        (0..WORKLOADS.len())
            .map(|ix| deployment(&connection("homelab"), ix, TEST_NOW).identity)
            .collect::<Vec<_>>(),
        "the example's own Deployments keep their identities"
    );
    let status = rows.last().unwrap();
    assert_eq!(status.identity.address(), "status/status-page");
    let document = document(&status.identity, TEST_NOW).unwrap();
    assert_eq!(document.name, "status-page");
    assert!(events(&status.identity, TEST_NOW).is_empty());
    for namespace in ["argocd", "cart", "checkout", "status"] {
        assert!(namespaces().iter().any(|name| name == namespace));
    }
}

/// The fixture's Kubernetes summary reads status-page as Resources does, not
/// as one of the example's own Deployments.
#[test]
fn the_summary_reads_status_page_as_its_own_deployment() {
    use k8s_openapi::api::apps::v1::Deployment;
    let deployments: Vec<Deployment> =
        super::summary::summary_objects("homelab", "deployments.apps", TEST_NOW);
    assert_eq!(deployments.len(), WORKLOADS.len() + 1);
    let status = deployments.last().unwrap();
    assert_eq!(status.metadata.name.as_deref(), Some("status-page"));
    assert_eq!(status.metadata.namespace.as_deref(), Some("status"));
    let labels = status.metadata.labels.clone().unwrap_or_default();
    assert_eq!(
        labels.get("app.kubernetes.io/part-of").map(String::as_str),
        Some("public-status")
    );
    let containers = &status
        .spec
        .as_ref()
        .unwrap()
        .template
        .spec
        .as_ref()
        .unwrap()
        .containers;
    assert_eq!(
        containers[0].image.as_deref(),
        Some("ghcr.io/example/status-page:1.4.0")
    );
    // The example's first Deployment is a different object.
    assert_ne!(deployments[0].metadata.name, status.metadata.name);
}
