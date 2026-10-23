use super::*;

#[test]
fn proxy_paths_name_scheme_service_and_port() {
    let mut service = PrometheusService::new("monitoring", "prometheus-k8s", 9090);
    assert_eq!(
        service.proxy_base(),
        "/api/v1/namespaces/monitoring/services/prometheus-k8s:9090/proxy/"
    );
    service.https = true;
    assert_eq!(
        service.proxy_base(),
        "/api/v1/namespaces/monitoring/services/https:prometheus-k8s:9090/proxy/"
    );
    assert_eq!(service.label(), "monitoring/prometheus-k8s:9090");
}

#[test]
fn ranking_prefers_the_operator_then_labels_then_names_then_ports() {
    let rank_of = |value| rank(&parse(value)).map(|candidate| candidate.rank);
    assert_eq!(rank_of(operated()), Some(Rank::Operated));
    assert_eq!(
        rank_of(service(
            "obs",
            "metrics",
            json!({"app.kubernetes.io/name": "prometheus"}),
            json!([{"name": "http", "port": 80}]),
        )),
        Some(Rank::Labelled)
    );
    assert_eq!(
        rank_of(service(
            "monitoring",
            "kube-prometheus-stack-prometheus",
            json!({}),
            json!([{"name": "http-web", "port": 9090}, {"name": "reloader-web", "port": 8080}]),
        )),
        Some(Rank::Named)
    );
    // Prometheus's companions only rank by a port, and the operator's own
    // https port isn't one.
    assert_eq!(
        rank_of(service(
            "monitoring",
            "alertmanager-operated",
            json!({}),
            json!([{"name": "web", "port": 9093}]),
        )),
        Some(Rank::Port)
    );
    assert_eq!(
        rank_of(service(
            "monitoring",
            "prometheus-operator",
            json!({}),
            json!([{"name": "https", "port": 8443}]),
        )),
        None
    );
    assert_eq!(
        rank_of(service(
            "default",
            "kubernetes",
            json!({}),
            json!([{"name": "https", "port": 443}]),
        )),
        None
    );
}

#[test]
fn the_chosen_port_and_scheme_follow_its_name() {
    let candidate = rank(&parse(service(
        "monitoring",
        "prometheus",
        json!({"app": "prometheus"}),
        json!([{"name": "grpc", "port": 10901}, {"name": "https-web", "port": 9091}, {"name": "web", "port": 9090}]),
    )))
    .unwrap();
    assert_eq!(candidate.service.port, 9090);
    assert!(!candidate.service.https);
    let candidate = rank(&parse(service(
        "monitoring",
        "prometheus",
        json!({"app": "prometheus"}),
        json!([{"name": "https", "port": 443}]),
    )))
    .unwrap();
    assert_eq!(candidate.service.port, 443);
    assert!(candidate.service.https);
}

#[test]
fn compatible_servers_rank_with_their_port_and_prefix() {
    let candidate = |name: &str, labels: Value, ports: Value| {
        rank(&parse(service("metrics", name, labels, ports)))
    };
    let single = candidate(
        "vmsingle-victoria",
        json!({"app.kubernetes.io/name": "vmsingle"}),
        json!([{"name": "http", "port": 8429}]),
    )
    .unwrap();
    assert_eq!(single.rank, Rank::Compatible);
    assert_eq!(single.service.label(), "metrics/vmsingle-victoria:8429");
    assert_eq!(single.service.backend(), Backend::VictoriaMetrics);

    let select = candidate(
        "vmselect-cluster",
        json!({}),
        json!([{"name": "http", "port": 8481}]),
    )
    .unwrap();
    assert_eq!(
        select.service.proxy_base(),
        "/api/v1/namespaces/metrics/services/vmselect-cluster:8481/proxy/select/0/prometheus/"
    );

    let thanos = candidate(
        "thanos-query",
        json!({}),
        json!([{"name": "grpc", "port": 10901}, {"name": "http", "port": 10902}]),
    )
    .unwrap();
    assert_eq!(thanos.service.port, 10902);
    assert_eq!(thanos.service.backend(), Backend::Thanos);

    let mimir = candidate(
        "mimir-query-frontend",
        json!({}),
        json!([{"name": "http-metrics", "port": 8080}]),
    )
    .unwrap();
    assert_eq!(
        mimir.service.label(),
        "metrics/mimir-query-frontend:8080/prometheus"
    );
    assert_eq!(mimir.service.backend(), Backend::Mimir);

    // The writers have no query API.
    assert!(
        candidate(
            "vminsert-cluster",
            json!({}),
            json!([{"name": "http", "port": 8480}])
        )
        .is_none()
    );
    // Prometheus itself still outranks them.
    assert!(Rank::Operated < Rank::Compatible && Rank::Labelled < Rank::Compatible);
}

#[test]
fn paths_and_urls_are_normalised() {
    assert_eq!(
        normalise_path("/select//0/prometheus/"),
        "select/0/prometheus"
    );
    assert_eq!(normalise_path("../a/./b"), "a/b");
    assert_eq!(
        normalise_url(" https://vm.example.com:8481/select/0/prometheus ").unwrap(),
        "https://vm.example.com:8481/select/0/prometheus/"
    );
    assert_eq!(
        normalise_url("http://prom.lan").unwrap(),
        "http://prom.lan/"
    );
    assert!(normalise_url("ftp://prom.lan").is_err());
    assert!(normalise_url("prom.lan:9090").is_err());
    assert!(normalise_url("https://user:secret@prom.lan").is_err());
    assert!(normalise_url("https://prom.lan/?token=x").is_err());
}
