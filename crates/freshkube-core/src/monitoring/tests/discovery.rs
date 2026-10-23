use super::*;

#[tokio::test]
async fn discovery_confirms_the_operator_service_and_reads_its_scrape_interval() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "default",
                    "web",
                    json!({}),
                    json!([{"name": "http", "port": 80}]),
                ),
                operated(),
            ]),
        ),
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/query" => {
            (200, query_one())
        }
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/status/buildinfo" => {
            (200, build_info())
        }
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/targets" => {
            (200, targets())
        }
        _ => (404, status(404, "not found")),
    });
    let Discovery::Found {
        prometheus,
        version,
        tried,
    } = discover(&client, None).await.unwrap()
    else {
        panic!("Prometheus should be found");
    };
    assert_eq!(
        prometheus.service().unwrap().label(),
        "monitoring/prometheus-operated:9090"
    );
    assert_eq!(version.as_deref(), Some("2.53.0"));
    assert_eq!(prometheus.scrape_interval(), 15.);
    assert!(tried.is_empty());
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().all(|request| request.starts_with("GET ")),
        "{seen:?}"
    );
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(seen[1].ends_with("/api/v1/query?query=1"), "{seen:?}");
    assert!(
        seen[3].ends_with("/api/v1/targets?state=active"),
        "{seen:?}"
    );
}

#[tokio::test]
async fn discovery_moves_past_candidates_that_are_not_prometheus() {
    let (client, _) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "monitoring",
                    "alertmanager-operated",
                    json!({}),
                    json!([{"name": "web", "port": 9093}]),
                ),
                operated(),
                service(
                    "monitoring",
                    "prometheus-k8s",
                    json!({"app.kubernetes.io/name": "prometheus"}),
                    json!([{"name": "web", "port": 9090}]),
                ),
            ]),
        ),
        // The operator's Service has no endpoints yet.
        p if p.contains("prometheus-operated") => (
            503,
            status(
                503,
                "no endpoints available for service \"prometheus-operated\"",
            ),
        ),
        p if p.contains("prometheus-k8s:9090/proxy/api/v1/query") => (200, query_one()),
        p if p.contains("prometheus-k8s:9090/proxy/api/v1/status/buildinfo") => (200, build_info()),
        p if p.contains("prometheus-k8s") => (500, "oops".into()),
        _ => (404, "404 page not found".into()),
    });
    let Discovery::Found {
        prometheus, tried, ..
    } = discover(&client, None).await.unwrap()
    else {
        panic!("Prometheus should be found");
    };
    assert_eq!(prometheus.service().unwrap().name, "prometheus-k8s");
    // A failed targets read keeps the default interval.
    assert_eq!(prometheus.scrape_interval(), DEFAULT_SCRAPE_INTERVAL);
    assert_eq!(tried.len(), 1);
    assert_eq!(tried[0].error.kind, ErrorKind::Unavailable);
    assert!(
        tried[0].error.message.contains("no endpoints"),
        "{}",
        tried[0].error.message
    );
}

#[tokio::test]
async fn discovery_moves_past_a_silent_vmsingle_to_vmselect_under_its_prefix() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "storefront",
                    "storefront-web",
                    json!({"app.kubernetes.io/name": "storefront"}),
                    json!([{"name": "http", "port": 8080}]),
                ),
                service(
                    "vm",
                    "vmselect-acme",
                    json!({"app.kubernetes.io/name": "vmselect"}),
                    json!([{"name": "http", "port": 8481}]),
                ),
                service(
                    "metrics",
                    "vmsingle-acme",
                    json!({"app.kubernetes.io/name": "vmsingle"}),
                    json!([{"name": "http", "port": 8429}]),
                ),
            ]),
        ),
        // The single-node server ranks first and has no endpoints.
        p if p.contains("vmsingle-acme") => (
            503,
            status(503, "no endpoints available for service \"vmsingle-acme\""),
        ),
        // vmselect answers only under its tenant's prefix, and has no
        // buildinfo there.
        "/api/v1/namespaces/vm/services/vmselect-acme:8481/proxy/select/0/prometheus/api/v1/query" => {
            (200, query_one())
        }
        _ => (404, "404 page not found".into()),
    });
    let Discovery::Found {
        prometheus,
        version,
        tried,
    } = discover(&client, None).await.unwrap()
    else {
        panic!("vmselect should be found");
    };
    let found = prometheus.service().unwrap();
    assert_eq!(found.label(), "vm/vmselect-acme:8481/select/0/prometheus");
    assert_eq!(prometheus.endpoint().backend(), Backend::VictoriaMetrics);
    assert_eq!(version, None);
    assert_eq!(tried.len(), 1);
    assert_eq!(tried[0].service.name, "vmsingle-acme");
    assert_eq!(tried[0].error.kind, ErrorKind::Unavailable);
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().all(|request| request.starts_with("GET ")),
        "{seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|request| request.contains("storefront-web")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn discovery_without_prometheus_lists_what_it_tried() {
    let (client, _) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "monitoring",
                    "alertmanager-operated",
                    json!({}),
                    json!([{"name": "web", "port": 9093}]),
                ),
                service(
                    "default",
                    "kubernetes",
                    json!({}),
                    json!([{"name": "https", "port": 443}]),
                ),
            ]),
        ),
        _ => (404, "404 page not found".into()),
    });
    let Discovery::Missing { candidates, tried } = discover(&client, None).await.unwrap() else {
        panic!("nothing should be found");
    };
    assert_eq!(candidates.len(), 1);
    assert_eq!(tried.len(), 1);
    assert_eq!(tried[0].error.kind, ErrorKind::NotFound);
    assert_eq!(tried[0].error.message, "Not found: 404 page not found");

    let (client, _) = fake(|_, _, _| (200, list(Vec::new())));
    assert!(matches!(
        discover(&client, None).await.unwrap(),
        Discovery::Missing { candidates, tried } if candidates.is_empty() && tried.is_empty()
    ));
}

#[tokio::test]
async fn a_refused_service_list_falls_back_to_the_monitoring_namespaces() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (403, status(403, "forbidden")),
        "/api/v1/namespaces/monitoring/services" => (200, list(vec![operated()])),
        p if p.ends_with("/services") => (403, status(403, "forbidden")),
        p if p.ends_with("/api/v1/query") => (200, query_one()),
        p if p.ends_with("/status/buildinfo") => (200, build_info()),
        _ => (404, status(404, "not found")),
    });
    assert!(matches!(
        discover(&client, None).await.unwrap(),
        Discovery::Found { .. }
    ));
    let listed = seen.lock().unwrap().clone();
    assert!(
        listed
            .iter()
            .any(|r| r == "GET /api/v1/namespaces/kube-system/services?"
                || r == "GET /api/v1/namespaces/kube-system/services")
    );

    let (client, _) = fake(|_, _, _| (403, status(403, "forbidden")));
    let error = discover(&client, None).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "Not allowed to list Services to find Prometheus"
    );
}

#[tokio::test]
async fn the_remembered_service_is_tried_before_listing() {
    let (client, seen) = fake(|_, path, _| match path {
        p if p.contains("/services/prom:9090/proxy/api/v1/query") => (200, query_one()),
        p if p.contains("/services/prom:9090/proxy/api/v1/status/buildinfo") => (200, build_info()),
        _ => (404, status(404, "not found")),
    });
    let remembered = PrometheusService::new("obs", "prom", 9090);
    let Discovery::Found { prometheus, .. } =
        discover(&client, Some(remembered.clone())).await.unwrap()
    else {
        panic!("the remembered Service should answer");
    };
    assert_eq!(prometheus.service(), Some(&remembered));
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("/api/v1/services"))
    );
}
