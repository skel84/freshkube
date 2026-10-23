use super::*;

/// One change, with Argo CD installed in each namespace and its Stage's
/// Application in only `application_namespace`.
fn argo_in(namespaces: &[&str], application_namespace: &str) -> FixtureReader {
    one_cluster(|world| {
        let mut app = application(Some(IN_CLUSTER_SERVER));
        app["metadata"]["namespace"] = json!(application_namespace);
        world.argocd = std::mem::take(&mut world.argocd)
            .with("applications", vec![app])
            .with(
                "deployments",
                namespaces
                    .iter()
                    .map(|namespace| {
                        json!({
                            "metadata": {
                                "namespace": namespace, "name": "argocd-server",
                                "labels": {"app.kubernetes.io/part-of": "argocd"}
                            }
                        })
                    })
                    .collect(),
            );
        world.kargo = std::mem::take(&mut world.kargo)
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "warehouses", "freights", "stages", "promotions"],
            )
            .with(
                "projects",
                vec![json!({"metadata": {"name": "storefront"}})],
            );
    })
}

fn application_lists(reader: &FixtureReader) -> Vec<String> {
    reader
        .requests
        .borrow()
        .iter()
        .filter(|request| request.contains("/applications ns="))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_change_finds_its_application_in_each_discovered_namespace() {
    for namespace in ["gitops", "platform"] {
        let reader = argo_in(&["platform", "delivery", "gitops"], namespace);
        let change = read(&reader, &place("f-new"), observed_at()).await.unwrap();
        assert_eq!(
            change.stages[0].cluster,
            Destination::Cluster("core".into())
        );
        assert!(
            change.hops.iter().any(|hop| match &hop.shows {
                Shows::Hop(detail) => detail.actions.iter().any(|action| match &action.target {
                    super::super::Target::Resource { object, .. } =>
                        object.kind == "Application" && object.namespace == namespace,
                    _ => false,
                }),
                _ => false,
            }),
            "the Application opens in {namespace}"
        );
        assert_eq!(application_lists(&reader).len(), 3);
    }
}

#[tokio::test]
async fn the_changes_list_finds_an_application_in_the_second_namespace() {
    let reader = argo_in(&["delivery", "gitops"], "gitops");
    let at = crate::changes::live::Where {
        cluster: "cluster-key".into(),
        label: "core".into(),
        argocd_namespace: "delivery".into(),
        mapping: Mapping::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
        github: None,
    };
    let read =
        crate::changes::live::read(&reader, &at, observed_at(), crate::changes::live::BUDGET).await;
    let changes = crate::changes::derive(&read);
    let change = changes
        .rows
        .iter()
        .find_map(|row| row.trail.as_ref().ok())
        .unwrap();
    assert_eq!(
        change.stages[0].cluster,
        Destination::Cluster("core".into())
    );
    assert_eq!(
        application_lists(&reader).len(),
        2,
        "each namespace read once"
    );
}

#[tokio::test]
async fn changes_and_applications_share_the_namespace_cap() {
    let namespaces = ["e-ops", "a-ops", "d-ops", "b-ops", "c-ops"];
    let applications = argo_in(&namespaces, "e-ops");
    let session = crate::applications::read::read_session(
        &applications,
        crate::applications::SessionKey::new("core"),
    )
    .await;
    let crate::applications::ArgoScope::Namespace {
        namespaces: read_namespaces,
        ..
    } = session.argo_scope
    else {
        panic!("a namespaced read");
    };
    assert_eq!(read_namespaces, ["a-ops", "b-ops", "c-ops"]);
    let expected = application_lists(&applications);
    let reader = argo_in(&namespaces, "e-ops");
    let change = read(&reader, &place("f-new"), observed_at()).await.unwrap();
    assert_eq!(application_lists(&reader), expected);
    assert!(matches!(change.stages[0].cluster, Destination::Unknown(_)));
    assert!(change.missing.iter().any(|line| line.text.contains("d-ops")
        && line.text.contains("e-ops")
        && line.text.contains("namespace cap")));

    let reader = argo_in(&namespaces, "e-ops");
    let at = crate::changes::live::Where {
        cluster: "cluster-key".into(),
        label: "core".into(),
        argocd_namespace: "a-ops".into(),
        mapping: Mapping::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
        github: None,
    };
    let read =
        crate::changes::live::read(&reader, &at, observed_at(), crate::changes::live::BUDGET).await;
    assert_eq!(application_lists(&reader), expected);
    assert!(
        crate::changes::derive(&read)
            .notes
            .iter()
            .any(|note| note.text.contains("d-ops") && note.text.contains("e-ops"))
    );
}
