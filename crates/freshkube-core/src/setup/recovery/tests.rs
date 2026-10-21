use super::*;
use crate::workspace::{Chosen, Part, Sources};

const GOOD: &str = "{id: first, role: core, context: first, future: {keep: true}}";
const LAST: &str = "{id: last, role: environment, context: last}";

fn recover(text: &str) -> Recovered {
    read(text.as_bytes()).unwrap()
}

#[test]
fn every_cluster_fault_skips_one_whole_row_with_original_indexes() {
    let cases = [
        ("17", "id"),
        ("{}", "id"),
        ("{id: bad, role: other, context: bad}", "role"),
        ("{id: bad, role: core}", "context"),
        ("{id: '', role: core, context: bad}", "id"),
        ("{id: bad, role: core, context: ''}", "context"),
        (
            "{id: bad, role: core, context: bad, talos_context: ''}",
            "talos_context",
        ),
        (
            "{id: bad, role: core, context: bad, talos_context: other}",
            "talos_context",
        ),
        (
            "{id: bad, role: core, context: bad, talosconfig: relative}",
            "talosconfig",
        ),
        (
            "{id: bad, role: core, context: bad, results: 17}",
            "results",
        ),
        (
            "{id: bad, role: core, context: bad, results: {forward: {namespace: '', service: svc, port: 80}}}",
            "results.forward.namespace",
        ),
        (
            "{id: bad, role: core, context: bad, results: {forward: {namespace: ns, service: '', port: 80}}}",
            "results.forward.service",
        ),
        (
            "{id: bad, role: core, context: bad, results: {forward: {namespace: ns, service: svc, port: 0}}}",
            "results.forward.port",
        ),
        (
            "{id: bad, role: core, context: bad, results: {forward: {namespace: ns, service: svc, port: wrong}}}",
            "results.forward.port",
        ),
        (
            "{id: bad, role: core, context: bad, results: {address: invalid-endpoint}}",
            "results.address",
        ),
        (
            "{id: bad, role: core, context: bad, results: {address: 'http://results.example.test'}}",
            "results.address",
        ),
        (
            "{id: bad, role: core, context: bad, results: {address: 'https://results.example.test', ca_file: relative}}",
            "results.ca_file",
        ),
        (
            "{id: bad, role: core, context: bad, results: {}}",
            "results",
        ),
        (
            "{id: bad, role: core, context: bad, results: {address: 'https://results.example.test', forward: {namespace: ns, service: svc, port: 80}}}",
            "results",
        ),
        (
            "{id: bad, role: core, context: bad, results: {token: keys.json}}",
            "results.token",
        ),
        (
            "{id: bad, role: core, context: bad, results: {ca_file: relative}}",
            "results.ca_file",
        ),
        (
            "{id: bad, role: core, context: bad, coroot: {cluster: ''}}",
            "coroot.cluster",
        ),
        (
            "{id: bad, role: core, context: bad, coroot: {cluster: scope, project: ''}}",
            "coroot.project",
        ),
        (
            "{id: bad, role: core, context: bad, coroot: {cluster: scope}}",
            "coroot.project",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {value: ''}}",
            "metrics.value",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {}}}",
            "metrics.source",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {url: invalid-endpoint}}}",
            "metrics.source.url",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {service: {namespace: '', name: svc, port: 80}}}}",
            "metrics.source.service.namespace",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {service: {namespace: ns, name: '', port: 80}}}}",
            "metrics.source.service.name",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {service: {namespace: ns, name: svc, port: 0}}}}",
            "metrics.source.service.port",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {service: {namespace: ns, name: svc, port: 80}, token: keys.json}}}",
            "metrics.source.token",
        ),
        (
            "{id: bad, role: core, context: bad, metrics: {source: {service: {namespace: ns, name: svc, port: 80}, url: 'https://metrics.example.test'}}}",
            "metrics.source",
        ),
    ];
    for (bad, field) in cases {
        let text = format!("version: 1\nclusters:\n- {GOOD}\n- {bad}\n- []\n- {LAST}\n");
        let read = recover(&text);
        assert_eq!(
            read.setup
                .workspace
                .clusters
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "last"],
            "{bad}"
        );
        assert_eq!(
            read.warnings.iter().map(|w| w.index).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(read.warnings[0].field, field, "{bad}");
        assert!(!read.source_recovery.all_clusters_skipped);
        assert!(
            super::super::parse(text.as_bytes()).is_err(),
            "strict parse must refuse {bad}"
        );
        read.setup.validate().unwrap();
        assert!(super::super::to_text(&read.setup).contains("future:"));
    }
    let long = "x".repeat(65);
    let read = recover(&format!(
        "version: 1\nclusters: [{{id: {long}, role: core, context: bad}}]"
    ));
    assert!(read.warnings[0].id.is_none());
    assert_eq!(read.warnings[0].field, "id");
}

#[test]
fn every_destination_fault_is_local_and_never_repeats_the_key() {
    let cases = [
        ("17", "row"),
        ("{server: []}", "server"),
        ("{name: []}", "name"),
        ("{entry: []}", "entry"),
        (
            "{server: 'https://private-marker.test', name: private-marker, entry: first}",
            "server/name",
        ),
        ("{entry: first}", "server/name"),
        ("{server: '', entry: first}", "server"),
        ("{name: '', entry: first}", "name"),
        ("{name: ' private-marker ', entry: first}", "server/name"),
        ("{server: 'private-marker', entry: first}", "server"),
        ("{name: private-marker}", "entry"),
        ("{name: in-cluster, entry: last}", "entry"),
    ];
    for (bad, field) in cases {
        let read = recover(&format!(
            "version: 1\nclusters: [{GOOD}, {LAST}]\ndestinations:\n- {{name: before, entry: first}}\n- {bad}\n- []\n- {{name: after, entry: last}}\n"
        ));
        assert_eq!(read.setup.workspace.destinations.len(), 2, "{bad}");
        assert_eq!(
            read.warnings.iter().map(|w| w.index).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(read.warnings[0].field, field);
        assert!(!read.warning_words().unwrap().contains("private-marker"));
        assert!(!format!("{:?}", read.warnings).contains("private-marker"));
    }
}

#[test]
fn shared_faults_are_fatal_even_after_an_unreadable_row() {
    let cases = [
        "kubeconfig: relative",
        "sources: {argocd: ''}",
        "sources: {kargo: []}",
        "metrics: []",
        "coroot: []",
        "supply_chain: []",
        "sources: []",
        "metrics: {mode: central}",
        "metrics: {mode: central, label: bad-label, store: {url: 'https://metrics.example.test'}}",
        "metrics: {mode: central, store: {entry: '', service: {namespace: ns, name: svc, port: 80}}}",
        "metrics: {mode: central, store: {entry: first, service: {namespace: '', name: svc, port: 80}}}",
        "metrics: {mode: central, store: {entry: first, service: {namespace: ns, name: '', port: 80}}}",
        "metrics: {mode: central, store: {entry: first, service: {namespace: ns, name: svc, port: 0}}}",
        "metrics: {mode: central, store: {url: invalid}}",
        "metrics: {mode: central, store: {}}",
        "metrics: {mode: central, store: {entry: first, service: {namespace: ns, name: svc, port: 80}, token: keys.json}}",
        "coroot: {server: invalid, sign_in: anonymous}",
        "coroot: {server: 'https://coroot.example.test', sign_in: api-key, key: keys.json, api_key_file: relative}",
        "coroot: {server: 'https://coroot.example.test', sign_in: api-key, api_key_file: relative}",
        "coroot: {server: 'https://coroot.example.test', sign_in: anonymous, project: ''}",
        "supply_chain: {public_key_file: relative}",
        "supply_chain: {evidence: {result: '', commit: /commit, digest: /digest}}",
        "supply_chain: {evidence: {result: result, commit: invalid, digest: /digest}}",
        "supply_chain: {evidence: {result: result, commit: /commit, digest: invalid}}",
    ];
    for fault in cases {
        let text = format!("version: 1\nclusters: [[]]\n{fault}\n");
        assert!(read(text.as_bytes()).is_err(), "{fault}");
    }
    for text in [
        "[]",
        "null",
        "{}",
        "version: []",
        "version: 999",
        "version: [",
        "version: 1\nversion: 1",
        "version: 1\nclusters: {}",
        "version: 1\ndestinations: {}",
        "version: 1\nclusters: null",
    ] {
        assert!(read(text.as_bytes()).is_err(), "{text}");
    }
}

#[test]
fn raw_credential_preflight_cannot_be_hidden_by_any_skipped_row_or_dormant_block() {
    let marker = "invented-private-marker";
    let cases = [
        format!("clusters: [[], {{id: broken, results: {{token: {marker}}}}}]"),
        format!("clusters: [[], {{id: broken, metrics: {{source: {{token: {marker}}}}}}}]"),
        format!(
            "clusters: [[], {{id: broken, results: {{address: 'https://user:{marker}@example.test'}}}}]"
        ),
        format!(
            "clusters: [[], {{id: broken, metrics: {{source: {{url: 'https://{marker}@example.test'}}}}}}]"
        ),
        format!("metrics: {{mode: automatic, store: {{token: {marker}}}}}"),
        format!("metrics: {{mode: automatic, store: {{url: 'https://{marker}@example.test'}}}}"),
        format!("coroot: {{server: 'https://{marker}@example.test'}}"),
        format!("coroot: {{key: {marker}}}"),
        format!("destinations: [[], {{server: 'https://{marker}@example.test'}}]"),
    ];
    for fault in cases {
        let why = read(format!("version: 1\n{fault}\n").as_bytes()).unwrap_err();
        assert_eq!(why, Invalid::KeyValue(None), "{fault}");
        assert!(!why.to_string().contains(marker));
        assert!(!format!("{why:?}").contains(marker));
    }
    // Wrong-shaped key/token values are forbidden as well.
    for token in ["[]", "{}", "true", "17"] {
        assert_eq!(
            read(format!("version: 1\nclusters: [{{results: {{token: {token}}}}}]").as_bytes())
                .unwrap_err(),
            Invalid::KeyValue(None)
        );
    }
    let read = recover(
        "version: 1\nclusters: [{id: bad, role: core, context: bad, results: {address: invented-private-marker}}]",
    );
    assert!(!read.warning_words().unwrap().contains(marker));
    assert!(!format!("{:?}", read.warnings).contains(marker));
}

#[test]
fn duplicates_and_input_bounds_remain_fatal_before_recovery() {
    for rows in [
        format!("{GOOD}, {{id: first}}"),
        "{id: repeated}, {id: repeated}".into(),
    ] {
        assert!(matches!(
            read(format!("version: 1\nclusters: [{rows}]").as_bytes()),
            Err(Invalid::Workspace(workspace::Invalid::DuplicateId(_)))
        ));
    }
    for (collection, count) in [
        ("clusters", MAX_ENTRIES + 1),
        ("destinations", MAX_DESTINATIONS + 1),
    ] {
        let text = format!(
            "version: 1\n{collection}: [{}]",
            vec!["[]"; count].join(",")
        );
        assert!(read(text.as_bytes()).is_err());
    }
    assert_eq!(
        read(&vec![b' '; MAX_BYTES as usize + 1]).unwrap_err(),
        Invalid::TooLarge
    );
}

#[test]
fn coroot_first_otherwise_valid_link_wins_with_inherited_projects() {
    let read = recover(
        "version: 1\ncoroot: {server: 'https://coroot.example.test', sign_in: anonymous, project: shared}\nclusters:\n- {id: broken, role: core, context: '', coroot: {cluster: scope}}\n- {id: first, role: core, context: first, coroot: {cluster: scope}}\n- {id: second, role: core, context: second, coroot: {cluster: scope, project: shared}}\n- {id: third, role: core, context: third, coroot: {cluster: scope, project: separate}}\n",
    );
    assert_eq!(
        read.setup
            .workspace
            .clusters
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "third"]
    );
    assert_eq!(
        read.warnings.iter().map(|w| w.index).collect::<Vec<_>>(),
        [0, 2]
    );
    assert_eq!(
        read.warnings[1].reason,
        RowReason::CorootTwice {
            first: "first".into()
        }
    );
    assert!(read.warnings[1].to_string().contains("second"));
    assert!(read.warnings[1].to_string().contains("first"));
    assert!(!read.warnings[1].to_string().contains("shared"));
}

#[test]
fn all_skipped_is_used_but_unavailable_and_pins_and_mappings_never_retarget() {
    let read = recover(
        "version: 1\nclusters: [{id: broken, role: core, context: ''}]\nsources: {argocd: broken}\ndestinations: [{name: remote, entry: broken}, {name: in-cluster, entry: broken}]",
    );
    assert!(read.source_recovery.all_clusters_skipped);
    assert_eq!(read.setup.workspace.destinations.len(), 1);
    assert_eq!(read.warnings.len(), 2);
    let sources = Sources::of_recovered(&read.setup.workspace, &read.source_recovery);
    assert_eq!(sources.get(Part::ArgoCd), &Chosen::Gone("broken".into()));
    assert!(matches!(sources.get(Part::Builds), Chosen::Skipped(_)));
    assert_eq!(sources.workloads(), &[Chosen::Gone("broken".into())]);
    for text in ["version: 1", "version: 1\nclusters: []"] {
        let empty = recover(text);
        assert!(!empty.source_recovery.all_clusters_skipped);
        assert_eq!(empty.setup, Setup::default());
        assert_eq!(
            Sources::of_recovered(&empty.setup.workspace, &empty.source_recovery).get(Part::ArgoCd),
            &Chosen::Open
        );
    }
    let read = recover("version: 1\nclusters: [[]]\ndestinations: [[]]");
    assert!(
        Sources::of_recovered(&read.setup.workspace, &read.source_recovery)
            .workloads()
            .is_empty()
    );
    assert!(read.setup.workspace.destinations.is_empty());
}

#[test]
fn dormant_shared_semantics_are_kept_and_unknown_keys_round_trip() {
    let read = recover(&format!(
        "version: 1\nclusters: [{GOOD}, []]\nmetrics: {{mode: automatic, label: invalid-label, store: {{url: invalid}}}}\nfuture: {{keep: true}}\ndestinations: [{{name: remote, entry: first, future: true}}, []]"
    ));
    let back = super::super::parse(super::super::to_text(&read.setup).as_bytes()).unwrap();
    assert_eq!(back, read.setup);
    assert!(back.unknown_keys().contains(&"first.future".into()));
    assert!(
        back.unknown_keys()
            .contains(&"destinations[0].future".into())
    );
}

#[test]
fn warning_order_follows_collections_and_original_rows_in_the_input() {
    let read = recover("version: 1\ndestinations: [[], []]\nclusters: [[], []]\n");
    assert_eq!(
        read.warnings
            .iter()
            .map(|w| (w.row, w.index))
            .collect::<Vec<_>>(),
        [
            (Row::Destination, 0),
            (Row::Destination, 1),
            (Row::Cluster, 0),
            (Row::Cluster, 1)
        ]
    );
}

#[test]
fn skipping_a_role_choice_never_selects_a_later_row_or_another_role() {
    use crate::workspace::SkippedRow;
    // Core fallback and CI/CD precedence both use the original input rows.
    for (rows, selected) in [
        (
            "[{id: bad-core, role: core, context: ''}, {id: next-core, role: core, context: good}]",
            [0, 0, 0],
        ),
        (
            "[{id: bad-core, role: core, context: ''}, {id: next-core, role: core, context: good}, {id: bad-ci, role: cicd, context: ''}, {id: next-ci, role: cicd, context: good}]",
            [2, 2, 0],
        ),
        (
            "[{id: bad-core, role: core, context: bad, results: []}, {id: next-core, role: core, context: good}, {id: bad-ci, role: cicd, context: bad, metrics: []}, {id: next-ci, role: cicd, context: good}]",
            [2, 2, 0],
        ),
    ] {
        let read = recover(&format!("version: 1\nclusters: {rows}"));
        let sources = Sources::of_recovered(&read.setup.workspace, &read.source_recovery);
        for (part, index) in Part::ALL.into_iter().zip(selected) {
            let id = if index == 0 { "bad-core" } else { "bad-ci" };
            assert_eq!(
                sources.get(part),
                &Chosen::Skipped(SkippedRow {
                    index,
                    id: Some(id.into())
                }),
                "{part:?}: {rows}"
            );
            let words = sources.get(part).missing().unwrap();
            assert!(words.contains(&format!("clusters[{index}] ({id}) was skipped")));
        }
        assert!(sources.entry("next-core").is_none());
        assert!(sources.entry("next-ci").is_none());
    }
}

#[test]
fn unreadable_role_choice_names_its_index_and_pins_still_take_precedence() {
    let text = "version: 1\nclusters: [{role: core, context: bad}, {id: usable, role: core, context: good}]\ndestinations: [{name: in-cluster, entry: usable}]\n";
    let read = recover(text);
    let sources = Sources::of_recovered(&read.setup.workspace, &read.source_recovery);
    for part in Part::ALL {
        assert_eq!(
            sources.get(part).missing().as_deref(),
            Some("clusters[0] was skipped in freshkube.yaml")
        );
    }
    assert!(read.setup.workspace.destinations.is_empty());
    assert_eq!(
        read.warnings[1].reason,
        RowReason::Destination(workspace::Fault::ArgoCdUnknown)
    );
    let pinned = recover(&format!(
        "{text}sources: {{kargo: usable, builds: absent, argocd: usable}}\n"
    ));
    let sources = Sources::of_recovered(&pinned.setup.workspace, &pinned.source_recovery);
    assert_eq!(sources.get(Part::Kargo).entry().unwrap().id, "usable");
    assert_eq!(sources.get(Part::ArgoCd).entry().unwrap().id, "usable");
    assert_eq!(sources.get(Part::Builds), &Chosen::Gone("absent".into()));
    assert_eq!(pinned.setup.workspace.destinations.len(), 1);
}

#[test]
fn an_unrelated_skip_does_not_make_a_healthy_role_choice_unavailable() {
    let read = recover(
        "version: 1\nclusters: [{id: chosen, role: core, context: good}, {id: later, role: core, context: ''}, {id: env, role: environment, context: ''}]\ndestinations: [{name: in-cluster, entry: chosen}]\n",
    );
    let sources = Sources::of_recovered(&read.setup.workspace, &read.source_recovery);
    for part in Part::ALL {
        assert_eq!(sources.get(part).entry().unwrap().id, "chosen");
    }
    assert_eq!(read.setup.workspace.destinations.len(), 1);
}

#[test]
fn unknown_roles_get_a_safe_reason_distinct_from_missing_or_wrong_shape() {
    for (role, unknown) in [
        ("invented-private-marker", true),
        ("''", true),
        ("null", false),
        ("[]", false),
        ("{}", false),
    ] {
        let recovered = recover(&format!(
            "version: 1\nclusters: [{GOOD}, {{id: skipped, context: skipped, role: {role}}}, {LAST}]\n"
        ));
        let warning = &recovered.warnings[0];
        assert_eq!(warning.field, "role");
        assert_eq!(warning.reason == RowReason::UnknownRole, unknown);
        assert_eq!(recovered.setup.workspace.clusters.len(), 2);
        assert!(!format!("{warning:?}").contains("invented-private-marker"));
        assert!(!warning.to_string().contains("invented-private-marker"));
        if unknown {
            assert_eq!(
                warning.to_string(),
                "skipped clusters[1] (skipped): role: is unknown; use core, cicd or environment"
            );
        }
    }
    let recovered = recover("version: 1\nclusters: [{id: skipped, context: skipped}]\n");
    assert_eq!(recovered.warnings[0].reason, RowReason::Shape);
    assert_eq!(recovered.warnings[0].field, "role");
}

#[test]
fn row_recovery_keeps_and_validates_shared_delivery_settings() {
    let recovered = recover(&format!(
        "version: 1\nclusters: [{GOOD}, []]\ndelivery:\n  build_namespace: builds\n  stage_naming: {{name_template: '{{stage}}-{{project}}'}}\n"
    ));
    let delivery = recovered.setup.delivery.unwrap();
    assert_eq!(delivery.build_namespace.as_deref(), Some("builds"));
    assert_eq!(
        delivery.stage_naming.unwrap().name_template,
        "{stage}-{project}"
    );
    assert!(
        read(b"version: 1\nclusters: [[]]\ndelivery: {build_namespace: invalid/name}\n").is_err()
    );
}
