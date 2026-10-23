use super::*;

#[tokio::test]
async fn a_dashboard_resolves_and_queries_over_get() {
    let (client, seen) = fake(|_, path, _| match path {
        p if p.ends_with("/label/node/values") => (200, success(json!(["cp-1", "worker-1"]))),
        p if p.ends_with("/query_range") => (
            200,
            success(json!({"resultType": "matrix", "result": [
                {"metric": {"node": "cp-1"}, "values": [[1_700_000_000, "0.5"], [1_700_000_060, "0.7"]]},
                {"metric": {"node": "worker-1"}, "values": [[1_700_000_000, "0.2"]]},
            ]})),
        ),
        _ => (404, status(404, "not found")),
    });
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let source = Source::Prometheus(Prometheus::new(
        client,
        PrometheusService::new("monitoring", "prometheus-operated", 9090),
    ));
    let (variables, warnings) = source
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(variables.options(0), ["All", "cp-1", "worker-1"]);
    let context = QueryContext {
        window: window(),
        variables: variables.context_values(),
    };
    let result = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert_eq!(result.frame.series.len(), 2);
    assert_eq!(result.frame.series[0].name, "cp-1");
    assert_eq!(
        result.expressions,
        [(
            "A".to_owned(),
            r#"sum by (node) (rate(cpu{node=~"(cp-1|worker-1)"}[5m]))"#.to_owned()
        )]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter().all(|request| request.starts_with(
            "GET /api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/"
        )),
        "{seen:?}"
    );
    assert!(
        seen[0].contains("match%5B%5D=up%7Bjob%3D%22node%22%7D"),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_failed_variable_names_itself() {
    let (client, _) = fake(|_, _, _| (403, status(403, "forbidden")));
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090));
    let error = prometheus
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "variable node: Not allowed to query Prometheus (services/proxy)"
    );
}

#[tokio::test]
async fn the_example_source_answers_through_the_same_plan() {
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let source = Source::Example(
        ExampleSource::new("nodes").with_values("node", vec!["alpha".into(), "beta".into()]),
    );
    let (variables, _) = source
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap();
    assert_eq!(variables.options(0), ["All", "alpha", "beta"]);
    let context = QueryContext {
        window: window(),
        variables: variables.context_values(),
    };
    let first = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert!(!first.frame.series.is_empty());
    assert_eq!(first.frame.times.len(), 90);
    assert!(first.expressions[0].1.contains("(alpha|beta)"));
    // Deterministic: the same panel and window give the same numbers.
    let second = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert_eq!(first.frame.series[0].values, second.frame.series[0].values);
}

#[tokio::test]
async fn a_scoped_prometheus_sends_the_matcher_with_every_dashboard_request() {
    let (client, seen) = fake(|_, path, _| match path {
        p if p.ends_with("/label/node/values") => (200, success(json!(["cp-1"]))),
        p if p.ends_with("/query_range") => (
            200,
            success(json!({"resultType": "matrix", "result": [
                {"metric": {"node": "cp-1"}, "values": [[1_700_000_000, "0.5"]]},
            ]})),
        ),
        _ => (404, status(404, "not found")),
    });
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let source = Source::Prometheus(
        Prometheus::new(
            client,
            PrometheusService::new("monitoring", "central", 9090),
        )
        .with_matcher(Matcher::new("cluster", "cluster-a")),
    );
    let (variables, _) = source
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap();
    let context = QueryContext {
        window: window(),
        variables: variables.context_values(),
    };
    let result = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert_eq!(
        result.expressions,
        [(
            "A".to_owned(),
            r#"sum by (node) (rate(cpu{cluster="cluster-a",node=~"cp-1"}[5m]))"#.to_owned()
        )]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(
        seen[0].contains("match%5B%5D=up%7Bcluster%3D%22cluster-a%22%2Cjob%3D%22node%22%7D"),
        "{seen:?}"
    );
    assert!(seen[1].contains("cluster%3D%22cluster-a%22"), "{seen:?}");
}

#[tokio::test]
async fn a_scoped_prometheus_never_sends_a_query_it_cannot_scope() {
    let (client, seen) = fake(|_, _, _| (200, success(json!([]))));
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090))
        .with_matcher(Matcher::new("cluster", "cluster-a"));
    let error = prometheus
        .get(&ApiRequest::new(
            "query",
            vec![("query".into(), "sum(".into())],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(
        error.message.starts_with("Can't scope the query"),
        "{error}"
    );
    assert!(seen.lock().unwrap().is_empty());
}
