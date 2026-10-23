use super::*;

mod reading {
    use serde_json::json;

    use super::*;
    use crate::applications::read::{MAX_ARGO_NAMESPACES, MAX_PROJECTS, read_session};
    use crate::fixtures::FixtureReader;

    fn world() -> FixtureReader {
        let namespaced = |ns: &str, name: &str, labels: Value| json!({"metadata": {"namespace": ns, "name": name, "labels": labels}});
        FixtureReader::default()
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "stages", "warehouses"],
            )
            .serves(
                "argoproj.io",
                "v1alpha1",
                &["applications", "applicationsets"],
            )
            .with("projects", vec![json!({"metadata": {"name": "checkout"}})])
            .with("stages", vec![namespaced("checkout", "dev", json!({}))])
            .with(
                "warehouses",
                vec![namespaced("checkout", "images", json!({}))],
            )
            .with(
                "applications",
                vec![json!({"metadata": {"namespace": "argocd", "name": "storefront"}})],
            )
            .with(
                "applicationsets",
                vec![json!({"metadata": {"namespace": "argocd", "name": "catalog"}})],
            )
            .with(
                "deployments",
                vec![
                    namespaced(
                        "shop",
                        "web",
                        json!({"app.kubernetes.io/part-of": "storefront"}),
                    ),
                    namespaced("shop", "plain", json!({"app.kubernetes.io/name": "plain"})),
                ],
            )
            .with(
                "statefulsets",
                vec![namespaced(
                    "shop",
                    "db",
                    json!({"app.kubernetes.io/part-of": "storefront"}),
                )],
            )
    }

    #[tokio::test]
    async fn a_session_reads_every_source_and_derives_from_them() {
        let reader = world();
        let read = read_session(&reader, key("core-fra")).await;
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
                destinations: Vec::new(),
            },
            &Override::default(),
        );
        let found_names: Vec<_> = found.applications.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(found_names, ["catalog", "checkout", "storefront"]);
        let storefront = found.find(&app_id("storefront")).unwrap();
        assert_eq!(
            names(storefront),
            ["storefront", "web", "db"],
            "the label matches the name, kinds in order"
        );
        assert_eq!(
            found.unknown().keys().copied().collect::<Vec<_>>(),
            [Rule::ArgoCd],
            "only the Argo CD namespace was listed"
        );
    }

    #[tokio::test]
    async fn reads_are_only_gets_and_scoped_lists() {
        let reader = world();
        read_session(&reader, key("core-fra")).await;
        let requests = reader.requests.borrow();
        assert!(
            requests
                .iter()
                .all(|r| r.starts_with("GET /apis") || r.starts_with("LIST "))
        );
        assert!(
            requests
                .iter()
                .filter(|r| r.contains("/deployments"))
                .all(|r| r.contains("selector=Some(")),
            "cluster-wide lists carry a selector: {requests:?}"
        );
        assert!(
            requests
                .iter()
                .any(|r| r.contains("applications ns=Some(\"argocd\")"))
        );
    }

    #[tokio::test]
    async fn sources_that_are_not_served_or_are_refused_are_told_apart() {
        let bare = FixtureReader::default().refusing("deployments");
        let read = read_session(&bare, key("core-fra")).await;
        assert!(matches!(read.kargo, Source::NotInstalled(_)));
        assert!(matches!(read.argo_applications, Source::NotInstalled(_)));
        assert!(matches!(read.workloads, Source::Refused(_)));
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
                destinations: Vec::new(),
            },
            &Override::default(),
        );
        assert!(found.applications.is_empty());
        assert_eq!(
            found.unknown().keys().copied().collect::<Vec<_>>(),
            [Rule::PartOf]
        );
    }

    #[tokio::test]
    async fn projects_past_the_cap_are_not_read_and_say_so() {
        let projects = (0..MAX_PROJECTS + 2)
            .map(|n| json!({"metadata": {"name": format!("p{n:03}")}}))
            .collect();
        let reader = FixtureReader::default()
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "stages", "warehouses"],
            )
            .with("projects", projects);
        let read = read_session(&reader, key("core-fra")).await;
        let Source::Capped(projects, truncation) = read.kargo else {
            panic!("expected a capped read");
        };
        assert_eq!(
            (projects.len(), truncation.read),
            (MAX_PROJECTS, MAX_PROJECTS)
        );
        let stage_lists = reader
            .requests
            .borrow()
            .iter()
            .filter(|r| r.contains("/stages"))
            .count();
        assert_eq!(stage_lists, MAX_PROJECTS);
    }

    /// Argo CD's own workloads, labelled as its install labels them, in
    /// each namespace given, with an Application in each.
    fn argo_in(namespaces: &[&str]) -> FixtureReader {
        let workload = |ns: &str| {
            json!({"metadata": {"namespace": ns, "name": "argocd-server",
                "labels": {"app.kubernetes.io/part-of": "argocd"}}})
        };
        let application =
            |ns: &str| json!({"metadata": {"namespace": ns, "name": format!("shop-{ns}")}});
        FixtureReader::default()
            .serves(
                "argoproj.io",
                "v1alpha1",
                &["applications", "applicationsets"],
            )
            .with(
                "deployments",
                namespaces.iter().map(|ns| workload(ns)).collect(),
            )
            .with(
                "applications",
                namespaces.iter().map(|ns| application(ns)).collect(),
            )
    }

    fn application_names(read: &SessionInputs) -> Vec<String> {
        let mut names: Vec<String> = read
            .argo_applications
            .read()
            .into_iter()
            .flatten()
            .map(|a| a.name.clone())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn argo_cd_is_read_where_its_workloads_run() {
        let reader = argo_in(&["gitops"]);
        let read = read_session(&reader, key("core-fra")).await;
        assert_eq!(
            read.argo_scope,
            ArgoScope::Namespace {
                namespaces: vec!["gitops".into()],
                found: ArgoFound::Labelled { skipped: vec![] },
            }
        );
        assert_eq!(application_names(&read), ["shop-gitops"]);
        let requests = reader.requests.borrow();
        assert!(
            !requests.iter().any(|r| r.contains("ns=Some(\"argocd\")")),
            "the default namespace isn't read when Argo CD was found: {requests:?}"
        );
    }

    #[tokio::test]
    async fn argo_cd_in_two_namespaces_is_read_in_both() {
        let read = read_session(&argo_in(&["platform", "gitops"]), key("core-fra")).await;
        let ArgoScope::Namespace { namespaces, found } = &read.argo_scope else {
            panic!("expected a namespaced read");
        };
        assert_eq!(namespaces, &["gitops", "platform"], "by name");
        assert_eq!(found, &ArgoFound::Labelled { skipped: vec![] });
        assert_eq!(application_names(&read), ["shop-gitops", "shop-platform"]);
    }

    #[tokio::test]
    async fn with_nothing_labelled_the_default_namespace_is_read() {
        let read = read_session(&world(), key("core-fra")).await;
        assert_eq!(
            read.argo_scope,
            ArgoScope::Namespace {
                namespaces: vec!["argocd".into()],
                found: ArgoFound::Default,
            }
        );
        assert_eq!(application_names(&read), ["storefront"]);
    }

    #[tokio::test]
    async fn namespaces_past_the_cap_are_named_and_not_read() {
        let reader = argo_in(&["e-ops", "a-ops", "d-ops", "b-ops", "c-ops"]);
        let read = read_session(&reader, key("core-fra")).await;
        let ArgoScope::Namespace { namespaces, found } = &read.argo_scope else {
            panic!("expected a namespaced read");
        };
        assert_eq!(namespaces.len(), MAX_ARGO_NAMESPACES);
        assert_eq!(namespaces, &["a-ops", "b-ops", "c-ops"]);
        assert_eq!(
            found,
            &ArgoFound::Labelled {
                skipped: vec!["d-ops".into(), "e-ops".into()]
            }
        );
        assert!(
            !reader
                .requests
                .borrow()
                .iter()
                .any(|r| r.contains("d-ops") || r.contains("e-ops"))
        );
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
                destinations: Vec::new(),
            },
            &Override::default(),
        );
        assert!(found.coverage.iter().any(|c| matches!(
            &c.state,
            CoverageState::NamespaceOnly { found: ArgoFound::Labelled { skipped }, .. }
                if skipped.len() == 2
        )));
    }

    #[tokio::test]
    async fn an_empty_default_namespace_is_not_found_never_none() {
        // Kargo and workloads answer; Argo CD is served but nothing marks
        // it, and argocd holds no Applications.
        let reader = FixtureReader::default().serves(
            "argoproj.io",
            "v1alpha1",
            &["applications", "applicationsets"],
        );
        let read = read_session(&reader, key("core-fra")).await;
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
                destinations: Vec::new(),
            },
            &Override::default(),
        );
        let argo: Vec<_> = found
            .coverage
            .iter()
            .filter(|c| c.source.rule() == Rule::ArgoCd)
            .map(|c| &c.state)
            .collect();
        assert_eq!(
            argo,
            [
                &CoverageState::NamespaceNotFound("argocd".into()),
                &CoverageState::NamespaceNotFound("argocd".into()),
            ]
        );
        assert!(found.unknown().contains_key(&Rule::ArgoCd));
    }

    #[tokio::test]
    async fn a_refusal_in_one_namespace_stands_for_the_whole_read() {
        let reader = argo_in(&["gitops", "platform"]).refusing("applications");
        let read = read_session(&reader, key("core-fra")).await;
        assert!(matches!(read.argo_applications, Source::Refused(_)));
    }

    #[tokio::test]
    async fn a_listing_cut_at_the_page_cap_marks_the_source_capped() {
        let reader = world().capped("deployments");
        let read = read_session(&reader, key("core-fra")).await;
        assert!(matches!(read.workloads, Source::Capped(_, _)));
    }
}
