use super::*;

#[test]
fn the_example_reader_supplies_absolute_paths() {
    let setup = example().expect("the bundled example parses and validates on every platform");
    assert!(setup.workspace.kubeconfig.as_ref().unwrap().is_absolute());
    assert!(
        setup.workspace.clusters[0]
            .talosconfig
            .as_ref()
            .unwrap()
            .is_absolute()
    );
    assert!(
        setup
            .supply_chain
            .as_ref()
            .unwrap()
            .public_key_file
            .as_ref()
            .unwrap()
            .is_absolute()
    );
}

#[test]
fn the_example_reads_as_the_design_describes() {
    let setup = example().expect("the bundled example is valid");
    let workspace = &setup.workspace;
    assert_eq!(workspace.kubeconfig, Some(home("/home/acme/.kube/config")));
    let ids: Vec<_> = workspace.clusters.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["cluster-a", "cluster-b", "cluster-c"]);
    let [a, b, c] = &workspace.clusters[..] else {
        panic!("three clusters");
    };
    assert_eq!(a.talos_context.as_deref(), Some("cluster-a"));
    assert_eq!(a.coroot.as_deref(), Some(&EntryCoroot::new("cluster-a")));
    assert_eq!(a.results, None);
    assert_eq!(a.metrics, None);
    assert_eq!(
        b.results.as_ref().unwrap().address.as_deref(),
        Some("https://results.example.test:8080")
    );
    assert_eq!(b.metrics.as_ref().unwrap().value.as_deref(), Some("ci-b"));
    assert_eq!(
        c.results.as_ref().unwrap().forward,
        Some(Forward::new(
            "tekton-pipelines",
            "tekton-results-api-service",
            8080
        ))
    );
    assert_eq!(
        c.coroot.as_ref().unwrap().project.as_deref(),
        Some("shop-api")
    );
    let source = c.metrics.as_ref().unwrap().source.as_ref().unwrap();
    assert_eq!(
        source.service,
        Some(MetricsService::new(
            "monitoring",
            "prometheus-operated",
            9090
        ))
    );
    assert_eq!(workspace.destinations.len(), 2);
    assert_eq!(setup.metrics.mode, Mode::Central);
    assert_eq!(setup.metrics.label_name(), "cluster");
    let store = setup.metrics.store.as_ref().unwrap();
    assert_eq!(store.entry.as_deref(), Some("cluster-a"));
    let service = store.service.as_ref().unwrap();
    assert_eq!(
        (service.name.as_str(), service.port, service.https),
        ("vmselect", 8481, false)
    );
    assert_eq!(service.path, "select/0/prometheus");
    let coroot = setup.coroot.as_ref().unwrap();
    assert_eq!(coroot.sign_in, SignIn::ApiKey);
    assert_eq!(coroot.key, Some(KeysJson));
    assert_eq!(coroot.project.as_deref(), Some("shop-api"));
    let chain = setup.supply_chain.as_ref().unwrap();
    assert_eq!(
        chain.public_key_file,
        Some(home("/home/acme/keys/cosign.pub"))
    );
    assert!(!chain.transparency_log);
    assert_eq!(
        chain.evidence,
        Some(Evidence::new("build-evidence", "/commit", "/image/digest"))
    );
    assert!(setup.unknown_keys().is_empty());
}

#[test]
fn the_example_survives_a_round_trip_and_the_text_settles() {
    let setup = example().expect("the bundled example is valid");
    let text = to_text(&setup);
    assert_eq!(ok(&text), setup);
    // Written again, the same bytes: a save over the app's own output
    // changes nothing.
    assert_eq!(to_text(&ok(&text)), text);
}

#[test]
fn the_text_starts_with_the_header_and_follows_the_fixed_order() {
    let text = to_text(&example().expect("the bundled example is valid"));
    let header: String = EXAMPLE.lines().take(4).map(|l| format!("{l}\n")).collect();
    assert!(text.starts_with(&header), "{text}");
    let order = [
        "\nversion: ",
        "\nkubeconfig: ",
        "\nclusters:",
        "\ndestinations:",
        "\nsources:",
        "\nmetrics:",
        "\ncoroot:",
        "\nsupply_chain:",
    ];
    let places: Vec<usize> = order
        .iter()
        .map(|key| text.find(key).unwrap_or_else(|| panic!("{key} in\n{text}")))
        .collect();
    assert!(places.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    // An entry's keys follow the design's order too.
    let entry = text.split("- id: cluster-c").nth(1).unwrap();
    let keys = ["role:", "context:", "results:", "coroot:", "metrics:"];
    let found: Vec<usize> = keys.iter().map(|k| entry.find(k).unwrap()).collect();
    assert!(found.windows(2).all(|pair| pair[0] < pair[1]), "{entry}");
    // No key and no secret is written: `keys.json` is a name.
    assert!(text.contains("key: keys.json"));
}

#[test]
fn a_minimal_file_writes_only_what_it_says() {
    let setup = ok(&with_cluster(""));
    let text = to_text(&setup);
    assert!(!text.contains("metrics"), "{text}");
    assert!(!text.contains("coroot"), "{text}");
    assert!(!text.contains("supply_chain"), "{text}");
    assert_eq!(ok(&text), setup);
    assert_eq!(setup.metrics, Metrics::default());
    assert_eq!(setup.metrics.mode, Mode::Automatic);
}

#[test]
fn an_empty_workspace_is_a_setup() {
    let setup = ok("version: 1\n");
    assert!(setup.workspace.clusters.is_empty());
    assert_eq!(ok(&to_text(&setup)), setup);
}

#[test]
fn in_cluster_aliases_in_yaml_keep_the_setup_and_round_trip() {
    let text = "\
version: 1
clusters:
  - {id: core-a, role: core, context: core-a}
  - {id: env-b, role: environment, context: env-b}
sources: {argocd: env-b}
metrics: {mode: per-entry}
destinations:
  - {server: 'https://Kubernetes.Default.Svc:443/', entry: env-b, note: kept}
  - {name: in-cluster, entry: env-b}
  - {name: remote, entry: core-a}
";
    let setup = ok(text);
    assert_eq!(setup.workspace.clusters.len(), 2);
    assert_eq!(setup.workspace.destinations.len(), 3);
    assert_eq!(setup.metrics.mode, Mode::PerEntry);
    assert_eq!(setup.unknown_keys(), vec!["destinations[0].note"]);
    assert_eq!(ok(&to_text(&setup)), setup);
    assert!(matches!(
        refused(&text.replace("sources: {argocd: env-b}", "sources: {argocd: missing}")),
        Invalid::Workspace(workspace::Invalid::Destination {
            fault: workspace::Fault::ArgoCdUnknown,
            ..
        })
    ));
    assert_eq!(
        refused(&text.replace("sources: {argocd: env-b}", "sources: {argocd: core-a}")).to_string(),
        "destinations[0] (server https://Kubernetes.Default.Svc:443/) names Argo CD's own cluster but maps it to another entry; use core-a, the selected Argo CD source entry"
    );
    assert_eq!(
        refused(&text.replace("entry: env-b, note: kept", "entry: core-a, note: kept")).to_string(),
        "destinations[0] (server https://Kubernetes.Default.Svc:443/) names Argo CD's own cluster but maps it to another entry; use env-b, the selected Argo CD source entry"
    );
    assert!(
        matches!(
            refused(&text.replace("role: core", "role: unknown")),
            Invalid::WrongShape(_)
        ),
        "unrelated refusals remain file-fatal"
    );
}

#[test]
fn unknown_keys_are_kept_at_every_level() {
    let text = "\
version: 1
top_note: {a: 1}
clusters:
  - id: cluster-a
    role: core
    context: cluster-a
    entry_note: hello
    results:
      address: https://results.example.test
      results_note: 1
    coroot:
      project: shop-api
      cluster: cluster-a
      coroot_note: [1, 2]
    metrics:
      value: ci-a
      metrics_note: true
      source:
        source_note: 1
        service: {namespace: m, name: p, port: 9090, service_note: 2}
  - id: cluster-b
    role: cicd
    context: cluster-b
    results:
      forward: {namespace: t, service: r, port: 8080, forward_note: 3}
sources:
  kargo: cluster-b
  sources_note: x
metrics:
  mode: central
  metrics_note: 1
  store:
    store_note: 2
    entry: cluster-a
    service: {namespace: vm, name: vmselect, port: 8481, store_service_note: 3}
coroot:
  server: https://coroot.example.test
  sign_in: session
  server_note: 4
supply_chain:
  chain_note: 5
  evidence: {result: r, commit: /c, digest: /d, evidence_note: 6}
";
    let setup = ok(text);
    let expected = [
        "chain_note",
        "cluster-a.coroot.coroot_note",
        "cluster-a.entry_note",
        "cluster-a.metrics.metrics_note",
        "cluster-a.metrics.source.service.service_note",
        "cluster-a.metrics.source.source_note",
        "cluster-a.results.results_note",
        "cluster-b.results.forward.forward_note",
        "coroot.server_note",
        "metrics.metrics_note",
        "metrics.store.store_note",
        "metrics.store.service.store_service_note",
        "sources.sources_note",
        "supply_chain.chain_note",
        "supply_chain.evidence.evidence_note",
        "top_note",
    ];
    let mut expected: Vec<String> = expected.map(String::from).to_vec();
    // `chain_note` above is under `supply_chain`; the bare one is a typo.
    expected.retain(|key| key != "chain_note");
    expected.sort();
    assert_eq!(setup.unknown_keys(), expected);
    let written = to_text(&setup);
    let again = ok(&written);
    assert_eq!(again, setup);
    assert_eq!(again.unknown_keys(), expected);
    for key in [
        "top_note",
        "entry_note",
        "forward_note",
        "store_service_note",
        "evidence_note",
    ] {
        assert!(written.contains(key), "{key} lost in\n{written}");
    }
}
