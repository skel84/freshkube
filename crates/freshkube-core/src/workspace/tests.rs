use super::*;

/// A path that is absolute on every platform.
fn absolute(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("freshkube-workspace-tests")
        .join(name)
}

fn text(value: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn a_file_loads_as_the_workspace_it_describes() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    assert_eq!(load(&file), Loaded::Missing);
    let kubeconfig = absolute("kubeconfig");
    let talosconfig = absolute("talosconfig");
    std::fs::write(
        &file,
        text(serde_json::json!({
            "version": 1,
            "kubeconfig": kubeconfig,
            "clusters": [
                {"id": "mgmt", "role": "core", "context": "acme-mgmt", "talosconfig": talosconfig},
                {"id": "ci", "role": "cicd", "context": "acme-ci"},
                {"id": "prod", "role": "environment", "context": "acme-prod"},
            ],
        })),
    )
    .unwrap();
    let Loaded::Workspace(workspace) = load(&file) else {
        panic!("{:?}", load(&file));
    };
    assert_eq!(workspace.kubeconfig, Some(kubeconfig));
    let mut mgmt = Entry::new("mgmt", Role::Core, "acme-mgmt");
    mgmt.talosconfig = Some(talosconfig);
    assert_eq!(
        workspace.clusters,
        vec![
            mgmt,
            Entry::new("ci", Role::Cicd, "acme-ci"),
            Entry::new("prod", Role::Environment, "acme-prod"),
        ]
    );
}

#[test]
fn keys_this_version_does_not_know_are_ignored() {
    let workspace = parse(&text(serde_json::json!({
        "version": 1,
        "clusters": [{"id": "mgmt", "role": "core", "context": "acme-mgmt", "note": "x"}],
        "destinations": [{"server": "https://prod.example.test", "entry": "mgmt"}],
    })))
    .unwrap();
    assert_eq!(workspace.clusters.len(), 1);
}

#[test]
fn a_file_the_app_cannot_use_is_refused_with_a_reason() {
    let cases: Vec<(serde_json::Value, Invalid)> = vec![
        (serde_json::json!({}), Invalid::MissingVersion),
        (
            serde_json::json!({"version": "1"}),
            Invalid::UnreadableVersion,
        ),
        (
            serde_json::json!({"version": 1.0}),
            Invalid::UnreadableVersion,
        ),
        (
            serde_json::json!({"version": 2}),
            Invalid::UnknownVersion(2),
        ),
        (
            serde_json::json!({"version":1,"clusters":[{"id":"","role":"core","context":"x"}]}),
            Invalid::EmptyId,
        ),
        (
            serde_json::json!({"version":1,"clusters":[
                {"id":"a","role":"core","context":"x"},{"id":"a","role":"cicd","context":"y"}]}),
            Invalid::DuplicateId("a".into()),
        ),
        (
            serde_json::json!({"version":1,"clusters":[{"id":"a","role":"core","context":" "}]}),
            Invalid::EmptyContext("a".into()),
        ),
        (
            serde_json::json!({"version":1,"clusters":[
                {"id":"a","role":"core","context":"x","talosconfig":"relative"}]}),
            Invalid::RelativePath("a".into()),
        ),
        (
            serde_json::json!({"version":1,"kubeconfig":"relative"}),
            Invalid::RelativePath("the workspace".into()),
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(parse(&text(value.clone())), Err(expected), "{value}");
    }
    assert_eq!(parse(b"not json"), Err(Invalid::NotJson));
}

#[test]
fn an_entry_needs_a_known_role() {
    for entry in [
        serde_json::json!({"id": "a", "context": "x"}),
        serde_json::json!({"id": "a", "context": "x", "role": "admin"}),
        serde_json::json!({"id": "a", "context": "x", "role": null}),
    ] {
        let refused = parse(&text(
            serde_json::json!({"version": 1, "clusters": [entry]}),
        ));
        assert!(matches!(refused, Err(Invalid::Malformed(_))), "{refused:?}");
    }
}

#[test]
fn too_many_clusters_long_ids_and_oversized_files_are_refused() {
    let mut workspace = Workspace::default();
    for n in 0..=MAX_ENTRIES {
        workspace
            .clusters
            .push(Entry::new(format!("c{n}"), Role::Environment, "x"));
    }
    assert_eq!(workspace.validate(), Err(Invalid::TooManyEntries));
    let long = Workspace {
        clusters: vec![Entry::new("a".repeat(MAX_ID_BYTES + 1), Role::Core, "x")],
        ..Default::default()
    };
    assert!(matches!(long.validate(), Err(Invalid::LongId(_))));

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    std::fs::write(&file, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
    assert_eq!(load(&file), Loaded::Refused(Invalid::TooLarge));
}

#[test]
fn loading_leaves_a_refused_file_where_it_is() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    std::fs::write(&file, "{\"version\":9}").unwrap();
    assert_eq!(load(&file), Loaded::Refused(Invalid::UnknownVersion(9)));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"version\":9}");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn the_example_workspace_is_valid_and_has_one_core() {
    let workspace = example();
    assert_eq!(workspace.validate(), Ok(()));
    assert_eq!(workspace.clusters.len(), 6);
    assert_eq!(
        workspace
            .clusters
            .iter()
            .filter(|entry| entry.role == Role::Core)
            .count(),
        1
    );
}

fn sample() -> Workspace {
    Workspace {
        kubeconfig: Some(absolute("kubeconfig")),
        clusters: vec![
            {
                let mut mgmt = Entry::new("mgmt", Role::Core, "acme-mgmt");
                mgmt.talosconfig = Some(absolute("talosconfig"));
                mgmt
            },
            Entry::new("ci", Role::Cicd, "acme-ci"),
            Entry::new("prod", Role::Environment, "acme-prod"),
        ],
        ..Default::default()
    }
}

#[test]
fn a_workspace_round_trips_through_its_file() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    save(&file, &sample()).unwrap();
    assert_eq!(load(&file), Loaded::Workspace(sample()));
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.starts_with("{\n  \"version\": 1,"));
    // An entry without a talosconfig writes none.
    assert_eq!(written.matches("\"talosconfig\":").count(), 1);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn keys_this_version_does_not_know_survive_a_save() {
    let source = serde_json::json!({
        "version": 1,
        "clusters": [{"id": "mgmt", "role": "core", "context": "acme-mgmt", "note": "kept"}],
        "destinations": [{"server": "https://prod.example.test", "entry": "mgmt"}],
        "sources": {"a": 1},
        "talosconfg": "misspelt",
    });
    let workspace = parse(&text(source)).unwrap();
    let written: serde_json::Value = serde_json::from_str(&to_text(&workspace)).unwrap();
    assert_eq!(
        written["destinations"][0]["server"],
        "https://prod.example.test"
    );
    assert_eq!(written["sources"]["a"], 1);
    assert_eq!(written["talosconfg"], "misspelt");
    assert_eq!(written["clusters"][0]["note"], "kept");
    assert_eq!(written["version"], 1);
    assert_eq!(parse(to_text(&workspace).as_bytes()), Ok(workspace));
}

#[test]
fn an_unknown_key_is_named_but_a_reserved_one_is_not() {
    let workspace = parse(&text(serde_json::json!({
        "version": 1,
        "clusters": [
            {"id": "mgmt", "role": "core", "context": "x", "talosconfg": "/a"},
            {"id": "ci", "role": "cicd", "context": "y"},
        ],
        "destinations": [],
        "sources": {},
        "kubeconfg": "/b",
    })))
    .unwrap();
    assert_eq!(
        workspace.unknown_keys(),
        vec!["kubeconfg", "mgmt.talosconfg"]
    );
    assert!(sample().unknown_keys().is_empty());
}

#[test]
fn saving_an_invalid_workspace_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    let bad = Workspace {
        clusters: vec![
            Entry::new("a", Role::Core, ""),
            Entry::new("a", Role::Cicd, "x"),
        ],
        ..Default::default()
    };
    assert!(save(&file, &bad).is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn a_save_replaces_the_file_whole() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    save(&file, &sample()).unwrap();
    let mut smaller = sample();
    smaller.clusters.truncate(1);
    save(&file, &smaller).unwrap();
    assert_eq!(load(&file), Loaded::Workspace(smaller));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn a_refused_file_is_set_aside_and_never_over_an_earlier_backup() {
    use chrono::TimeZone;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    let now = chrono::Utc
        .with_ymd_and_hms(2026, 10, 8, 14, 30, 5)
        .unwrap();

    std::fs::write(&file, "first").unwrap();
    let first = set_aside(&file, now).unwrap();
    assert_eq!(first, directory.path().join("workspace.json.bak"));
    assert!(!file.exists());

    std::fs::write(&file, "second").unwrap();
    let second = set_aside(&file, now).unwrap();
    assert_eq!(
        second,
        directory.path().join("workspace.20261008T143005Z.bak")
    );

    std::fs::write(&file, "third").unwrap();
    let third = set_aside(&file, now).unwrap();
    assert_eq!(
        third,
        directory.path().join("workspace.20261008T143005Z-2.bak")
    );

    for (backup, content) in [(first, "first"), (second, "second"), (third, "third")] {
        assert_eq!(std::fs::read_to_string(backup).unwrap(), content);
    }
    assert!(!file.exists());
}

#[test]
fn setting_aside_a_missing_file_fails_and_makes_no_backup() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    assert!(set_aside(&file, chrono::Utc::now()).is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn fresh_ids_come_from_the_context_and_never_repeat() {
    let mut workspace = sample();
    assert_eq!(workspace.fresh_id("Acme Staging!"), "acme-staging");
    assert_eq!(workspace.fresh_id("???"), "cluster");
    assert_eq!(workspace.fresh_id("acme-ci"), "acme-ci");
    workspace
        .clusters
        .push(Entry::new("acme-ci", Role::Cicd, "x"));
    assert_eq!(workspace.fresh_id("acme-ci"), "acme-ci-2");
    assert!(workspace.fresh_id(&"x".repeat(200)).len() <= MAX_ID_BYTES);
}
