use super::*;

/// The example's paths are Unix ones; a Windows path is absolute only with a
/// drive, so the tests read them under one.
fn portable(text: &str) -> String {
    if cfg!(windows) {
        text.replace("C:/home/acme", "/home/acme")
            .replace("/home/acme", "C:/home/acme")
    } else {
        text.to_owned()
    }
}

fn home(path: &str) -> std::path::PathBuf {
    portable(path).into()
}

fn ok(text: &str) -> Setup {
    parse(portable(text).as_bytes()).unwrap_or_else(|invalid| panic!("{invalid}"))
}

fn refused(text: &str) -> Invalid {
    parse(portable(text).as_bytes()).expect_err("the file should be refused")
}

fn at(line: usize, column: usize) -> Option<Place> {
    Some(Place { line, column })
}

/// A file with one cluster and `rest` after it.
fn with_cluster(rest: &str) -> String {
    format!(
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    context: cluster-a\n{rest}"
    )
}

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

#[test]
fn a_file_that_is_not_yaml_says_where() {
    // The second entry is indented under the first one's value.
    let text = "version: 1\nclusters:\n  - id: cluster-a\n   role: core\n";
    let invalid = refused(text);
    assert_eq!(invalid, Invalid::NotYaml(at(4, 4)));
    assert_eq!(invalid.to_string(), "invalid YAML at line 4, column 4");
}

#[test]
fn a_duplicate_key_is_invalid_yaml_at_its_line() {
    let invalid = refused("version: 1\nkubeconfig: /a\nkubeconfig: /b\n");
    assert!(matches!(invalid, Invalid::NotYaml(_)), "{invalid:?}");
    assert!(!invalid.to_string().contains("/b"), "{invalid}");
}

#[test]
fn a_file_that_is_not_text_is_invalid_yaml() {
    let invalid = parse(&[0x76, 0xff, 0xfe, 0x3a]).unwrap_err();
    assert!(matches!(invalid, Invalid::NotYaml(_)), "{invalid:?}");
}

#[test]
fn words_without_a_place_say_which_pass_failed() {
    assert_eq!(Invalid::NotYaml(None).to_string(), "invalid YAML");
    assert_eq!(
        Invalid::WrongShape(None).to_string(),
        "a value of the wrong type or shape"
    );
}

#[test]
fn a_value_of_the_wrong_shape_says_where() {
    let text = with_cluster("    talosconfig: [not, a, path]\n");
    let invalid = refused(&text);
    assert_eq!(invalid, Invalid::WrongShape(at(6, 18)));
    assert_eq!(
        invalid.to_string(),
        "a value of the wrong type or shape at line 6, column 18"
    );
}

#[test]
fn a_wrong_role_and_a_wrong_mode_are_found_at_their_own_lines() {
    let text = "version: 1\nclusters:\n  - id: cluster-a\n    role: wizard\n    context: x\n";
    assert_eq!(refused(text), Invalid::WrongShape(at(4, 11)));
    let text = "version: 1\nmetrics:\n  mode: sideways\n";
    assert_eq!(refused(text), Invalid::WrongShape(at(3, 9)));
    let text = "version: 1\ncoroot:\n  server: https://coroot.example.test\n";
    let Invalid::WrongShape(Some(place)) = refused(text) else {
        panic!("a missing sign_in is a wrong shape");
    };
    // A missing key is found at the mapping that lacks it.
    assert_eq!(place.line, 3);
}

#[test]
fn a_refusal_never_repeats_what_was_read() {
    let secret = "s3cr3t-value-7d41";
    for text in [
        format!(
            "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: api-key\n  key: {secret}\n"
        ),
        format!("version: 1\nkubeconfig: [{secret}]\n"),
        format!("version: 1\nmetrics:\n  mode: {secret}\n"),
        format!("version: 1\nclusters:\n  - id: a\n    role: {secret}\n    context: c\n"),
        format!("version: 1\nclusters: [\n  {secret}\n"),
        format!("version: 1\ncoroot:\n  server: {secret}\n  sign_in: session\n"),
    ] {
        let words = refused(&text).to_string();
        assert!(!words.contains(secret), "{words}");
    }
}

#[test]
fn version_is_required_and_is_one() {
    assert_eq!(refused(""), Invalid::MissingVersion);
    assert_eq!(refused("clusters: []\n"), Invalid::MissingVersion);
    assert_eq!(refused("- a\n- b\n"), Invalid::MissingVersion);
    assert_eq!(refused("version: \"1\"\n"), Invalid::UnreadableVersion);
    assert_eq!(refused("version: 1.5\n"), Invalid::UnreadableVersion);
    assert_eq!(refused("version: -1\n"), Invalid::UnreadableVersion);
    assert_eq!(refused("version: 0\n"), Invalid::UnknownVersion(0));
    // A later version is named as such even when the rest is not ours.
    assert_eq!(
        refused("version: 2\nclusters: nonsense\n"),
        Invalid::UnknownVersion(2)
    );
    assert_eq!(
        refused("version: 2\n").to_string(),
        "it is version 2, and this app reads 1"
    );
    assert_eq!(refused("clusters: []\n").to_string(), "it has no version");
}

#[test]
fn the_version_is_not_an_unknown_key_and_is_written_fresh() {
    let setup = ok("version: 1\n");
    assert!(setup.unknown_keys().is_empty());
    assert!(to_text(&setup).contains("\nversion: 1\n"));
}

#[test]
fn a_file_over_the_limit_is_refused() {
    let mut text = String::from("version: 1\n# ");
    text.push_str(&"x".repeat(MAX_BYTES as usize));
    assert_eq!(refused(&text), Invalid::TooLarge);
}

#[test]
fn off_is_a_word_in_this_file_and_not_a_switch() {
    // YAML 1.1 readers take a bare `off` for false; there is no `results:
    // off` for that reason, and here it is refused where it stands.
    for value in ["off", "false", "no", "\"off\""] {
        let text = with_cluster(&format!("    results: {value}\n"));
        let invalid = refused(&text);
        assert_eq!(invalid, Invalid::WrongShape(at(6, 14)), "{value}");
        assert!(!invalid.to_string().contains("off"), "{invalid}");
    }
    let text = "version: 1\nmetrics:\n  mode: off\n";
    assert_eq!(refused(text), Invalid::WrongShape(at(3, 9)));
    // Words a YAML 1.1 reader would take for booleans stay strings here:
    // this reader takes only true and false, and the file reads back as
    // written.
    let text = "version: 1\nnote_a: off\nnote_b: 'yes'\nnote_c: \"n\"\n";
    let setup = ok(text);
    let written = to_text(&setup);
    assert_eq!(ok(&written), setup);
    for (key, word) in [("note_a", "off"), ("note_b", "yes"), ("note_c", "n")] {
        assert_eq!(setup.extra[key], serde_json::Value::String(word.into()));
    }
}

#[test]
fn a_key_or_token_is_keys_json_or_it_is_refused() {
    let coroot = |key: &str| {
        format!(
            "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: api-key\n  key: {key}\n"
        )
    };
    assert_eq!(ok(&coroot("keys.json")).coroot.unwrap().key, Some(KeysJson));
    for refused_value in ["abc123", "\"\"", "Keys.json", "keys.json.bak", "42", "true"] {
        let invalid = refused(&coroot(refused_value));
        assert_eq!(invalid, Invalid::KeyValue(at(5, 8)), "{refused_value}");
        assert!(!invalid.to_string().contains(refused_value), "{invalid}");
    }
    // A list or a mapping is no value at all: a wrong shape, as anywhere.
    for shape in ["[keys.json]", "{a: b}"] {
        let invalid = refused(&coroot(shape));
        assert!(
            matches!(invalid, Invalid::WrongShape(Some(_))),
            "{invalid:?}"
        );
    }
    let text = "version: 1\nmetrics:\n  mode: central\n  store:\n    url: https://metrics.example.test\n    token: hunter2\n";
    assert_eq!(refused(text), Invalid::KeyValue(at(6, 12)));
    let text = text.replace("hunter2", "keys.json");
    assert_eq!(ok(&text).metrics.store.unwrap().token, Some(KeysJson));
    let text = with_cluster(
        "    metrics:\n      source:\n        url: https://m.example.test\n        token: hunter2\n",
    );
    assert_eq!(refused(&text), Invalid::KeyValue(at(9, 16)));
}

#[test]
fn a_key_file_stands_instead_of_the_key_and_is_a_path() {
    let head = "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: api-key\n";
    let file = ok(&format!("{head}  api_key_file: /home/acme/coroot.key\n"));
    let coroot = file.coroot.as_ref().unwrap();
    assert_eq!(coroot.api_key_file, Some(home("/home/acme/coroot.key")));
    assert_eq!(coroot.key, None);
    assert_eq!(ok(&to_text(&file)), file);
    assert_eq!(
        refused(&format!("{head}  api_key_file: coroot.key\n")),
        Invalid::Rule(Fault::RelativePath("coroot.api_key_file".into()))
    );
    assert_eq!(
        refused(&format!(
            "{head}  key: keys.json\n  api_key_file: /home/acme/coroot.key\n"
        )),
        Invalid::Rule(Fault::KeyTwice)
    );
}

#[test]
fn the_workspace_rules_still_apply_and_name_the_entry() {
    let text = with_cluster("  - id: cluster-a\n    role: cicd\n    context: other\n");
    assert_eq!(
        refused(&text).to_string(),
        "the id “cluster-a” is used twice"
    );
    let text = with_cluster("    talos_context: cluster-a\n");
    assert_eq!(
        refused(&text),
        Invalid::Workspace(workspace::Invalid::TalosContextWithoutConfig(
            "cluster-a".into()
        ))
    );
    let text = "version: 1\nkubeconfig: kube/config\n";
    assert_eq!(
        refused(text),
        Invalid::Workspace(workspace::Invalid::RelativePath("the workspace".into()))
    );
}

fn rule(text: &str) -> String {
    match refused(text) {
        Invalid::Rule(fault) => fault.to_string(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn results_is_an_address_or_a_forward() {
    let both = with_cluster(
        "    results:\n      address: https://r.example.test\n      forward: {namespace: t, service: s, port: 1}\n",
    );
    assert_eq!(
        rule(&both),
        "the cluster “cluster-a” results must set exactly one of address or forward"
    );
    assert_eq!(
        rule(&with_cluster("    results: {}\n")),
        "the cluster “cluster-a” results must set exactly one of address or forward"
    );
    assert_eq!(
        rule(&with_cluster(
            "    results: {address: ftp://r.example.test}\n"
        )),
        "the cluster “cluster-a” results.address isn’t an http:// or https:// URL"
    );
    // A URL with a user is refused: it would hold a secret.
    assert_eq!(
        rule(&with_cluster(
            "    results: {address: 'https://u:p@r.example.test'}\n"
        )),
        "the cluster “cluster-a” results.address isn’t an http:// or https:// URL"
    );
    assert_eq!(
        rule(&with_cluster(
            "    results: {forward: {namespace: t, service: s, port: 0}}\n"
        )),
        "the cluster “cluster-a” results.forward.port isn’t a port"
    );
    assert_eq!(
        rule(&with_cluster(
            "    results: {forward: {namespace: ' ', service: s, port: 80}}\n"
        )),
        "the cluster “cluster-a” results.forward.namespace is empty"
    );
}

#[test]
fn an_entrys_coroot_and_metrics_are_checked() {
    assert_eq!(
        rule(&with_cluster("    coroot: {cluster: ''}\n")),
        "the cluster “cluster-a” coroot.cluster is empty"
    );
    assert_eq!(
        rule(&with_cluster("    coroot: {cluster: c, project: ''}\n")),
        "the cluster “cluster-a” coroot.project is empty"
    );
    assert_eq!(
        rule(&with_cluster("    metrics: {value: ''}\n")),
        "the cluster “cluster-a” metrics.value is empty"
    );
    assert_eq!(
        rule(&with_cluster("    metrics: {source: {}}\n")),
        "the cluster “cluster-a” metrics.source must set exactly one of service or url"
    );
    assert_eq!(
        rule(&with_cluster(
            "    metrics: {source: {service: {namespace: m, name: p, port: 9090}, token: keys.json}}\n"
        )),
        "the cluster “cluster-a” metrics.source.token only goes with url"
    );
    // An entry's own source is checked under every mode.
    for mode in ["automatic", "central", "per-entry"] {
        let text = format!(
            "version: 1\nmetrics:\n  mode: {mode}\n  store: {{url: 'https://m.example.test'}}\nclusters:\n  - id: a\n    role: core\n    context: a\n    metrics: {{source: {{url: nonsense}}}}\n"
        );
        assert_eq!(
            rule(&text),
            "the cluster “a” metrics.source.url isn’t an http:// or https:// URL"
        );
    }
}

#[test]
fn central_mode_needs_a_store_and_a_label_name() {
    assert_eq!(
        rule("version: 1\nmetrics:\n  mode: central\n"),
        "metrics.mode is central, and metrics.store is missing"
    );
    let store =
        "  store:\n    entry: a\n    service: {namespace: vm, name: vmselect, port: 8481}\n";
    for label in ["1cluster", "my-label", "", "a b"] {
        let text = format!("version: 1\nmetrics:\n  mode: central\n{store}  label: '{label}'\n");
        assert_eq!(
            rule(&text),
            "metrics.label isn’t a Prometheus label name",
            "{label:?}"
        );
    }
    let text = format!("version: 1\nmetrics:\n  mode: central\n{store}  label: _k8s_cluster\n");
    assert_eq!(ok(&text).metrics.label_name(), "_k8s_cluster");
}

#[test]
fn the_store_is_a_service_in_an_entry_or_a_url() {
    let text = |store: &str| format!("version: 1\nmetrics:\n  mode: central\n  store:\n{store}");
    let one_of = "metrics.store must set exactly one of entry with service, or url";
    assert_eq!(rule(&text("    entry: a\n")), one_of);
    assert_eq!(
        rule(&text("    service: {namespace: vm, name: v, port: 1}\n")),
        one_of
    );
    assert_eq!(rule(&text("    {}\n")), one_of);
    assert_eq!(
        rule(&text(
            "    entry: a\n    service: {namespace: vm, name: v, port: 1}\n    url: https://m.example.test\n"
        )),
        one_of
    );
    assert_eq!(
        rule(&text(
            "    entry: a\n    service: {namespace: vm, name: v, port: 1}\n    token: keys.json\n"
        )),
        "metrics.store.token only goes with url"
    );
    assert_eq!(
        rule(&text("    url: ftp://m.example.test\n")),
        "metrics.store.url isn’t an http:// or https:// URL"
    );
    assert_eq!(
        rule(&text(
            "    entry: a\n    service: {namespace: vm, name: '', port: 1}\n"
        )),
        "metrics.store.service.name is empty"
    );
    let url = ok(&text(
        "    url: https://m.example.test/select/0/prometheus\n    token: keys.json\n",
    ));
    assert_eq!(ok(&to_text(&url)), url);
}

#[test]
fn a_service_path_is_normalised_and_read_with_or_without_slashes() {
    let service = |path: &str| {
        let text = format!(
            "version: 1\nmetrics:\n  mode: central\n  store:\n    entry: a\n    service: {{namespace: vm, name: v, port: 1, path: '{path}'}}\n"
        );
        ok(&text).metrics.store.unwrap().service.unwrap()
    };
    for path in [
        "/select/0/prometheus",
        "select/0/prometheus/",
        "//select/./0/../prometheus//",
    ] {
        assert_eq!(service(path).path, "select/0/prometheus", "{path}");
    }
    assert_eq!(service("/").path, "");
    assert_eq!(service("").path, "");
    let setup = ok(
        "version: 1\nmetrics:\n  mode: central\n  store:\n    entry: a\n    service: {namespace: vm, name: v, port: 1, https: true, path: p}\n",
    );
    let written = to_text(&setup);
    assert!(written.contains("path: /p"), "{written}");
    assert!(written.contains("https: true"), "{written}");
    assert_eq!(ok(&written), setup);
}

#[test]
fn a_store_and_label_under_another_mode_are_kept_unread() {
    // Not checked, since nothing reads them, but not lost either.
    let text = "version: 1\nmetrics:\n  mode: per-entry\n  label: 'not a label'\n  store:\n    entry: a\n    url: https://m.example.test\n";
    let setup = ok(text);
    assert_eq!(setup.metrics.mode, Mode::PerEntry);
    assert_eq!(setup.metrics.label.as_deref(), Some("not a label"));
    let written = to_text(&setup);
    assert!(written.contains("mode: per-entry"), "{written}");
    assert_eq!(ok(&written), setup);
}

#[test]
fn the_coroot_connection_is_checked() {
    let text = |lines: &str| format!("version: 1\ncoroot:\n{lines}  sign_in: anonymous\n");
    assert_eq!(
        rule(&text("  server: coroot.example.test\n")),
        "coroot.server isn’t an http:// or https:// URL"
    );
    assert_eq!(
        rule(&text(
            "  server: https://coroot.example.test\n  project: ''\n"
        )),
        "coroot.project is empty"
    );
    for sign_in in ["api-key", "session", "anonymous"] {
        let text = format!(
            "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: {sign_in}\n"
        );
        let setup = ok(&text);
        assert_eq!(ok(&to_text(&setup)), setup);
    }
}

#[test]
fn the_supply_chain_is_checked() {
    let text = |lines: &str| format!("version: 1\nsupply_chain:\n{lines}");
    assert_eq!(
        rule(&text("  public_key_file: cosign.pub\n")),
        "supply_chain.public_key_file isn’t an absolute path"
    );
    assert_eq!(
        rule(&text(
            "  evidence: {result: r, commit: commit, digest: /d}\n"
        )),
        "supply_chain.evidence.commit isn’t a JSON pointer, which starts with /"
    );
    assert_eq!(
        rule(&text("  evidence: {result: r, commit: /c, digest: ''}\n")),
        "supply_chain.evidence.digest isn’t a JSON pointer, which starts with /"
    );
    assert_eq!(
        rule(&text("  evidence: {result: '', commit: /c, digest: /d}\n")),
        "supply_chain.evidence.result is empty"
    );
    let setup = ok(&text("  transparency_log: true\n"));
    assert!(setup.supply_chain.as_ref().unwrap().transparency_log);
    let written = to_text(&setup);
    assert!(written.contains("transparency_log: true"), "{written}");
    assert_eq!(ok(&written), setup);
    // Present but empty is still a block: the person wrote it.
    assert_eq!(
        ok(&text("  {}\n")).supply_chain,
        Some(SupplyChain::default())
    );
}

#[test]
fn the_workspace_json_shape_is_unchanged_by_the_new_entry_keys() {
    let entry: Entry = serde_json::from_str(
        r#"{"id":"a","role":"core","context":"a","results":{"address":"https://r.example.test"}}"#,
    )
    .unwrap();
    assert!(entry.results.is_some());
    let plain = serde_json::to_value(Entry::new("a", workspace::Role::Core, "a")).unwrap();
    assert_eq!(
        plain,
        serde_json::json!({"id": "a", "role": "core", "context": "a"})
    );
}

#[test]
fn commit_writes_a_file_that_reads_back_and_refuses_a_changed_one() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("config").join("freshkube.yaml");
    let setup = example().expect("the bundled example is valid");
    let seen = commit(&file, &Seen::Missing, &setup).unwrap();
    assert_eq!(seen, Seen::Bytes(std::fs::read(&file).unwrap()));
    assert_eq!(ok(&std::fs::read_to_string(&file).unwrap()), setup);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // No temporary file is left beside it.
    let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["freshkube.yaml"]);

    // The next save knows what the last one wrote.
    let mut next = setup.clone();
    next.metrics.mode = Mode::Automatic;
    let seen = commit(&file, &seen, &next).unwrap();
    assert_eq!(ok(&std::fs::read_to_string(&file).unwrap()), next);

    // A hand edit in between is not written over.
    std::fs::write(&file, "version: 1\n# edited\n").unwrap();
    let result = commit(&file, &seen, &setup);
    assert!(matches!(result, Err(SaveError::Changed)), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "version: 1\n# edited\n"
    );
    // Nor is a file that appeared when none was expected.
    let result = commit(&file, &Seen::Missing, &setup);
    assert!(matches!(result, Err(SaveError::Changed)), "{result:?}");
}

#[test]
fn commit_writes_nothing_for_an_invalid_setup() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    let mut setup = example().expect("the bundled example is valid");
    setup.coroot.as_mut().unwrap().server = "nonsense".into();
    let result = commit(&file, &Seen::Missing, &setup);
    assert!(
        matches!(
            result,
            Err(SaveError::Invalid(Invalid::Rule(Fault::NotUrl(_))))
        ),
        "{result:?}"
    );
    assert!(!file.exists());
}

#[test]
fn a_setup_too_large_to_read_back_is_not_written() {
    let mut setup = example().expect("the bundled example is valid");
    setup.extra.insert(
        "padding".into(),
        serde_json::Value::String("x".repeat(MAX_BYTES as usize)),
    );
    assert_eq!(setup.check_saveable(), Err(Invalid::TooLarge));
}

#[test]
fn the_first_settings_setup_keeps_legacy_workspace_unknown_keys() {
    let workspace = workspace::parse(br#"{"version":1,"future":{"enabled":true}}"#).unwrap();
    let setup = Setup::new(workspace);
    setup.check_saveable().unwrap();
    assert_eq!(setup.unknown_keys(), ["future"]);
    assert_eq!(parse(to_text(&setup).as_bytes()).unwrap(), setup);
}

#[test]
fn a_legacy_key_reserved_by_setup_is_refused_before_a_save() {
    let workspace = workspace::parse(br#"{"version":1,"metrics":{"future":true}}"#).unwrap();
    assert!(Setup::new(workspace).check_saveable().is_err());
}

/// Two clusters, each with `coroot` lines after its context.
fn with_links(connection: &str, a: &str, b: &str) -> String {
    format!(
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    context: cluster-a\n{a}  - id: cluster-c\n    role: core\n    context: cluster-c\n{b}{connection}"
    )
}

const SHOP: &str =
    "coroot:\n  server: https://coroot.example.test\n  sign_in: anonymous\n  project: shop-api\n";

#[test]
fn two_entries_may_not_link_one_coroot_cluster() {
    let link = "    coroot:\n      cluster: cluster-a\n";
    // Both take the connection's project.
    assert_eq!(
        rule(&with_links(SHOP, link, link)),
        "the Coroot cluster “cluster-a” of project “shop-api” is linked to “cluster-a” and “cluster-c”"
    );
    // One names that project itself: still the same project.
    let own = "    coroot:\n      project: shop-api\n      cluster: cluster-a\n";
    assert_eq!(
        refused(&with_links(SHOP, link, own)),
        Invalid::Rule(Fault::CorootTwice {
            first: "cluster-a".into(),
            second: "cluster-c".into(),
            project: "shop-api".into(),
            cluster: "cluster-a".into(),
        })
    );
}

#[test]
fn the_same_cluster_name_in_another_project_is_another_link() {
    let link = "    coroot:\n      cluster: cluster-a\n";
    let other = "    coroot:\n      project: shop-web\n      cluster: cluster-a\n";
    ok(&with_links(SHOP, link, other));
    // Two clusters of one project are two links.
    let second = "    coroot:\n      cluster: cluster-c\n";
    ok(&with_links(SHOP, link, second));
}

#[test]
fn a_linked_entry_needs_a_project_from_somewhere() {
    let link = "    coroot:\n      cluster: cluster-a\n";
    let own = "    coroot:\n      project: shop-api\n      cluster: cluster-c\n";
    // Neither the entry nor the connection names one.
    assert_eq!(
        rule(&with_links("", link, own)),
        "the cluster “cluster-a” names a Coroot cluster, and neither it nor coroot names a project"
    );
    // A connection without a project is no project either.
    let bare = "coroot:\n  server: https://coroot.example.test\n  sign_in: anonymous\n";
    assert_eq!(
        refused(&with_links(bare, link, own)),
        Invalid::Rule(Fault::CorootWithoutProject("cluster-a".into()))
    );
    // Each one's own project is enough, with no connection at all.
    let both = "    coroot:\n      project: shop-api\n      cluster: cluster-a\n";
    ok(&with_links("", both, own));
    // And so is the connection's, for an entry that names none.
    ok(&with_links(SHOP, link, own));
}

#[test]
fn an_unlinked_entry_needs_nothing_and_a_refusal_repeats_no_value() {
    ok(&with_links("", "", ""));
    let link = "    coroot:\n      cluster: cluster-a\n";
    assert!(!rule(&with_links(SHOP, link, link)).contains("coroot.example.test"));
}

#[test]
fn the_example_links_are_distinct_and_survive_a_save() {
    let setup = ok(EXAMPLE);
    assert_eq!(setup.validate(), Ok(()));
    assert_eq!(setup.check_saveable(), Ok(()));
    // Linking cluster-c to cluster-a's Coroot cluster of the same project
    // breaks the rule, and the edited setup is not saveable.
    let mut edited = setup.clone();
    edited.workspace.clusters[2].coroot = Some(EntryCoroot::new("cluster-a").into());
    assert!(edited.validate().is_err());
    assert!(edited.check_saveable().is_err());
}

#[test]
fn results_address_credentials_are_typed_and_round_trip() {
    let text = with_cluster(
        "    results:\n      address: https://results.example.test\n      token: keys.json\n      ca_file: /home/acme/results-ca.pem\n      future: kept\n",
    );
    let setup = ok(&text);
    let results = setup.workspace.clusters[0].results.as_ref().unwrap();
    assert_eq!(results.token, Some(KeysJson));
    assert_eq!(results.ca_file, Some(home("/home/acme/results-ca.pem")));
    assert_eq!(setup.unknown_keys(), vec!["cluster-a.results.future"]);
    assert_eq!(ok(&to_text(&setup)), setup);
    let invalid = refused(&text.replace("token: keys.json", "token: invented-secret"));
    assert!(matches!(invalid, Invalid::KeyValue(_)));
    assert!(!invalid.to_string().contains("invented-secret"));
    assert_eq!(
        rule(&text.replace("/home/acme/results-ca.pem", "relative.pem")),
        "the cluster “cluster-a” results.ca_file isn’t an absolute path"
    );
}

#[test]
fn results_address_settings_do_not_go_with_a_forward() {
    for (extra, field) in [
        ("      token: keys.json\n", "token"),
        ("      ca_file: /home/acme/ca.pem\n", "ca_file"),
    ] {
        let text = with_cluster(&format!(
            "    results:\n      forward: {{namespace: acme, service: results, port: 8080}}\n{extra}"
        ));
        assert_eq!(
            rule(&text),
            format!("the cluster “cluster-a” results.{field} only goes with address")
        );
    }
}

#[test]
fn results_plain_http_is_only_saved_for_literal_loopback() {
    for address in [
        "http://results.example.test",
        "http://192.0.2.1",
        "http://localhost",
    ] {
        let text = with_cluster(&format!("    results: {{address: '{address}'}}\n"));
        assert_eq!(
            rule(&text),
            "the cluster “cluster-a” results.address: plain HTTP is allowed only on loopback"
        );
    }
    for address in [
        "https://results.example.test",
        "http://127.0.0.1:8080",
        "http://127.0.0.2",
        "http://[::1]:8080",
    ] {
        ok(&with_cluster(&format!(
            "    results: {{address: '{address}'}}\n"
        )));
    }
}

#[test]
fn destination_url_credentials_are_refused_by_strict_parse_and_save_without_echoing() {
    use crate::workspace::{Destination, Key};
    for address in [
        "https://invented-private-marker@example.test",
        "https://user:invented-private-marker@example.test",
        "https://user:invented-private-marker@[invalid",
    ] {
        let text = format!(
            "version: 1\nclusters: []\ndestinations: [{{server: '{address}', entry: cluster-a}}]\n"
        );
        let invalid = parse(text.as_bytes()).unwrap_err();
        assert!(!invalid.to_string().contains("invented-private-marker"));
        assert!(!format!("{invalid:?}").contains("invented-private-marker"));
        assert!(matches!(
            invalid,
            Invalid::Workspace(workspace::Invalid::Destination {
                fault: workspace::Fault::Userinfo,
                ..
            })
        ));
        let mut setup = Setup::default();
        setup
            .workspace
            .destinations
            .push(Destination::new(Key::Server(address.into()), "cluster-a"));
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("freshkube.yaml");
        std::fs::write(&file, "version: 1\n").unwrap();
        let before = std::fs::read(&file).unwrap();
        let error = commit(&file, &Seen::Bytes(before.clone()), &setup).unwrap_err();
        assert!(!format!("{error:?}").contains("invented-private-marker"));
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}

#[test]
fn stage_naming_is_a_template_with_an_optional_project_key() {
    let text = |lines: &str| format!("version: 1\ndelivery:\n  stage_naming:\n{lines}");
    // Off when absent.
    assert_eq!(ok("version: 1\n").delivery, None);
    let setup = ok(&text("    name_template: '{stage}-{project}'\n"));
    let naming = setup.delivery.as_ref().unwrap().stage_naming.as_ref();
    assert_eq!(naming.unwrap().name_template, "{stage}-{project}");
    assert_eq!(naming.unwrap().project_key, None);
    let keyed = ok(&text(
        "    name_template: 'app-{project}-in-{stage}'\n    project_key: example.test/project\n",
    ));
    let naming = keyed.delivery.as_ref().unwrap().stage_naming.as_ref();
    assert_eq!(
        naming.unwrap().project_key.as_deref(),
        Some("example.test/project")
    );
    let written = to_text(&keyed);
    assert!(
        written.contains("project_key: example.test/project"),
        "{written}"
    );
    assert_eq!(ok(&written), keyed);
    // A setup without it writes nothing for it.
    assert!(!to_text(&ok("version: 1\n")).contains("delivery"));
}

#[test]
fn a_stage_naming_template_without_stage_is_a_fault_of_its_own() {
    let text = |lines: &str| format!("version: 1\ndelivery:\n  stage_naming:\n{lines}");
    assert_eq!(
        rule(&text("    name_template: 'app-{project}'\n")),
        "delivery.stage_naming.name_template has no {stage} in it, so it names no Stage"
    );
    assert_eq!(
        rule(&text("    name_template: ''\n")),
        "delivery.stage_naming.name_template is empty"
    );
    assert_eq!(
        rule(&text("    name_template: '{stage}'\n    project_key: ''\n")),
        "delivery.stage_naming.project_key is empty"
    );
    // The template is required once the block is there.
    assert!(matches!(
        refused(&text("    project_key: example.test/project\n")),
        Invalid::WrongShape(_)
    ));
}

#[test]
fn unknown_keys_under_delivery_are_kept_and_named() {
    let setup = ok("\
version: 1
delivery:
  delivery_note: 1
  stage_naming: {name_template: '{stage}', naming_note: 2}
");
    assert_eq!(
        setup.unknown_keys(),
        [
            "delivery.delivery_note",
            "delivery.stage_naming.naming_note"
        ]
    );
    assert_eq!(ok(&to_text(&setup)), setup);
}

#[test]
fn a_stage_naming_template_needs_only_a_stage_somewhere_in_it() {
    let text = |template: &str| {
        format!("version: 1\ndelivery:\n  stage_naming:\n    name_template: '{template}'\n")
    };
    for template in [
        "{stage}",
        "{stage}-{stage}",
        "{stage}-{x}",
        "app-{project}-{stage}",
    ] {
        let setup = ok(&text(template));
        let naming = setup.delivery.unwrap().stage_naming.unwrap();
        assert_eq!(naming.name_template, template);
    }
}

#[test]
fn the_build_namespace_is_an_optional_delivery_key() {
    // Off when absent, and a setup without it writes nothing for it.
    assert_eq!(ok("version: 1\n").delivery, None);
    assert!(!to_text(&ok("version: 1\n")).contains("delivery"));
    let setup = ok("version: 1\ndelivery:\n  build_namespace: acme-builds\n");
    assert_eq!(
        setup
            .delivery
            .as_ref()
            .and_then(|delivery| delivery.build_namespace.as_deref()),
        Some("acme-builds")
    );
    let written = to_text(&setup);
    assert!(
        written.contains("build_namespace: acme-builds"),
        "{written}"
    );
    assert_eq!(ok(&written), setup);
    assert_eq!(
        rule("version: 1\ndelivery:\n  build_namespace: ''\n"),
        "delivery.build_namespace is empty"
    );
}

#[test]
fn unknown_keys_under_delivery_are_kept_and_named_beside_the_build_namespace() {
    let setup = ok("version: 1\ndelivery:\n  build_namespace: acme-builds\n  delivery_note: 1\n");
    assert_eq!(setup.unknown_keys(), ["delivery.delivery_note"]);
    assert_eq!(ok(&to_text(&setup)), setup);
}

#[test]
fn the_build_namespace_must_be_a_namespace_name_and_is_never_repeated() {
    let long = "a".repeat(64);
    for bad in [" a/b", "../x", "Acme", "-a", "a-", "a b", long.as_str()] {
        assert_eq!(
            rule(&format!(
                "version: 1\ndelivery:\n  build_namespace: '{bad}'\n"
            )),
            "delivery.build_namespace isn’t a Kubernetes namespace name",
            "{bad:?}"
        );
    }
    for good in ["a", "acme-builds", "x1", &"a".repeat(63)] {
        ok(&format!(
            "version: 1\ndelivery:\n  build_namespace: {good}\n"
        ));
    }
}
