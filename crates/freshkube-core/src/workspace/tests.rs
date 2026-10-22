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
            serde_json::json!({"version":1,"clusters":[
                {"id":"a","role":"core","context":"x","talosconfig":"/t","talos_context":" "}]}),
            Invalid::EmptyTalosContext("a".into()),
        ),
        (
            serde_json::json!({"version":1,"clusters":[
                {"id":"a","role":"core","context":"x","talos_context":"t"}]}),
            Invalid::TalosContextWithoutConfig("a".into()),
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
        "sources": {"kargo": "mgmt", "later": 1},
        "talosconfg": "misspelt",
    });
    let workspace = parse(&text(source)).unwrap();
    let written: serde_json::Value = serde_json::from_str(&to_text(&workspace)).unwrap();
    assert_eq!(
        written["destinations"][0]["server"],
        "https://prod.example.test"
    );
    assert_eq!(written["sources"]["kargo"], "mgmt");
    assert_eq!(written["sources"]["later"], 1);
    assert_eq!(written["talosconfg"], "misspelt");
    assert_eq!(written["clusters"][0]["note"], "kept");
    assert_eq!(written["version"], 1);
    assert_eq!(parse(to_text(&workspace).as_bytes()), Ok(workspace));
}

#[test]
fn an_unknown_key_is_named_wherever_it_is() {
    let workspace = parse(&text(serde_json::json!({
        "version": 1,
        "clusters": [
            {"id": "mgmt", "role": "core", "context": "x", "talosconfg": "/a"},
            {"id": "ci", "role": "cicd", "context": "y"},
        ],
        "destinations": [],
        "sources": {"argocd": "mgmt", "kargoo": "ci"},
        "kubeconfg": "/b",
    })))
    .unwrap();
    assert_eq!(
        workspace.unknown_keys(),
        vec!["kubeconfg", "mgmt.talosconfg", "sources.kargoo"]
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

fn now() -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(2026, 10, 8, 14, 30, 5)
        .unwrap()
}

#[test]
fn a_workspace_the_next_launch_would_refuse_for_its_size_is_not_written() {
    // Compact, the file is well inside the limit; written with indentation it
    // is not.
    let many: Vec<_> = (0..12_000).map(|n| serde_json::json!({"a": n})).collect();
    let source = text(serde_json::json!({"version": 1, "later": many}));
    assert!(source.len() < MAX_BYTES as usize);
    let workspace = parse(&source).unwrap();
    assert_eq!(workspace.check_saveable(), Err(Invalid::TooLarge));

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    assert!(save(&file, &workspace).is_err());
    assert!(!file.exists());
    assert!(matches!(
        commit(&file, &Seen::Missing, false, &workspace, now()),
        Err(SaveError::Invalid(Invalid::TooLarge))
    ));
    assert!(!file.exists());
}

#[test]
fn a_commit_writes_when_the_file_is_as_it_was_read_and_hands_back_what_it_wrote() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    let (loaded, seen) = load_seen(&file);
    assert_eq!((loaded, &seen), (Loaded::Missing, &Seen::Missing));
    let (aside, seen) = commit(&file, &seen, false, &sample(), now()).unwrap();
    assert_eq!(aside, None);
    assert_eq!(seen, Seen::Bytes(std::fs::read(&file).unwrap()));
    // The next commit goes by what the last one wrote.
    let mut smaller = sample();
    smaller.clusters.truncate(1);
    let (_, seen) = commit(&file, &seen, false, &smaller, now()).unwrap();
    assert_eq!(load(&file), Loaded::Workspace(smaller));
    assert_eq!(seen, Seen::Bytes(std::fs::read(&file).unwrap()));
}

#[test]
fn a_hand_edit_between_the_read_and_the_save_is_kept_and_nothing_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    save(&file, &sample()).unwrap();
    let (_, seen) = load_seen(&file);
    let edited = std::fs::read_to_string(&file)
        .unwrap()
        .replace("acme-ci", "acme-ci-edited");
    std::fs::write(&file, &edited).unwrap();

    let mut other = sample();
    other.clusters.truncate(1);
    assert!(matches!(
        commit(&file, &seen, false, &other, now()),
        Err(SaveError::Changed)
    ));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn a_file_another_window_created_after_the_read_is_not_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    let (_, seen) = load_seen(&file);
    assert_eq!(seen, Seen::Missing);
    std::fs::write(&file, "{\"version\":1}").unwrap();
    assert!(matches!(
        commit(&file, &seen, false, &sample(), now()),
        Err(SaveError::Changed)
    ));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"version\":1}");
}

#[test]
fn a_refused_file_that_changed_is_not_set_aside() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    std::fs::write(&file, "{\"version\":9}").unwrap();
    let (loaded, seen) = load_seen(&file);
    assert!(matches!(loaded, Loaded::Refused(_)));
    std::fs::write(&file, "{\"version\":1}").unwrap();
    assert!(matches!(
        commit(&file, &seen, true, &sample(), now()),
        Err(SaveError::Changed)
    ));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"version\":1}");
    assert!(!directory.path().join("workspace.json.bak").exists());
}

#[test]
fn a_refused_file_that_is_unchanged_is_set_aside_and_the_new_one_written() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    std::fs::write(&file, "{\"version\":9}").unwrap();
    let (_, seen) = load_seen(&file);
    let (aside, seen) = commit(&file, &seen, true, &sample(), now()).unwrap();
    assert_eq!(aside, Some(directory.path().join("workspace.json.bak")));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("workspace.json.bak")).unwrap(),
        "{\"version\":9}"
    );
    assert_eq!(load(&file), Loaded::Workspace(sample()));
    assert_eq!(seen, Seen::Bytes(std::fs::read(&file).unwrap()));
}

#[test]
fn a_file_that_cannot_be_set_aside_stops_the_commit_before_any_write() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    std::fs::create_dir(&file).unwrap();
    let (_, seen) = load_seen(&file);
    assert_eq!(seen, Seen::Unreadable);
    assert!(matches!(
        commit(&file, &seen, true, &sample(), now()),
        Err(SaveError::Aside(_))
    ));
    assert!(file.is_dir());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn a_talos_context_is_kept_and_an_absent_one_writes_nothing() {
    let mut entry = Entry::new("mgmt", Role::Core, "acme-mgmt");
    let mut workspace = Workspace {
        clusters: vec![entry.clone()],
        ..Default::default()
    };
    assert_eq!(to_text(&workspace).matches("talos_context").count(), 0);
    entry.talos_context = Some("acme-talos".into());
    entry.talosconfig = Some(absolute("talosconfig"));
    workspace.clusters = vec![entry];
    let written = to_text(&workspace);
    assert_eq!(parse(written.as_bytes()), Ok(workspace));
    assert_eq!(
        written.matches("\"talos_context\": \"acme-talos\"").count(),
        1
    );
}

fn three() -> Workspace {
    Workspace::new(vec![
        Entry::new("dev", Role::Environment, "acme-dev"),
        Entry::new("mgmt", Role::Core, "acme-mgmt"),
        Entry::new("ci", Role::Cicd, "acme-ci"),
    ])
}

#[test]
fn a_launch_starts_the_remembered_entry_first() {
    let workspace = three();
    let start = choose_start(&workspace, Some("ci")).unwrap();
    assert_eq!((start.entry.id.as_str(), start.note), ("ci", None));
}

#[test]
fn without_a_memory_it_starts_the_first_core_entry_and_says_nothing() {
    let workspace = three();
    let start = choose_start(&workspace, None).unwrap();
    assert_eq!((start.entry.id.as_str(), start.note), ("mgmt", None));
}

#[test]
fn without_a_core_entry_it_starts_the_first_listed() {
    let mut workspace = three();
    workspace.clusters.remove(1);
    let start = choose_start(&workspace, None).unwrap();
    assert_eq!((start.entry.id.as_str(), start.note), ("dev", None));
}

#[test]
fn a_remembered_entry_that_is_gone_is_named_once_beside_the_one_opened() {
    let workspace = three();
    let start = choose_start(&workspace, Some("retired")).unwrap();
    assert_eq!(start.entry.id, "mgmt");
    assert_eq!(
        start.note.as_deref(),
        Some("retired is no longer in the workspace; opened mgmt")
    );
    let mut without_core = three();
    without_core.clusters.remove(1);
    let start = choose_start(&without_core, Some("retired")).unwrap();
    assert_eq!(
        start.note.as_deref(),
        Some("retired is no longer in the workspace; opened dev")
    );
}

#[test]
fn an_empty_workspace_has_no_start_so_the_launch_is_today_s() {
    assert!(choose_start(&Workspace::default(), None).is_none());
    assert!(choose_start(&Workspace::default(), Some("retired")).is_none());
}

#[test]
fn destinations_map_a_server_or_a_name_to_an_entry() {
    let workspace = parse(&text(serde_json::json!({
        "version": 1,
        "clusters": [{"id": "prod", "role": "environment", "context": "acme-prod"}],
        "destinations": [
            {"server": "https://prod.example.test", "entry": "prod"},
            {"name": "prod-lon", "entry": "lon", "note": "kept"},
        ],
    })))
    .unwrap();
    let keys: Vec<(Key, &str)> = workspace
        .destinations
        .iter()
        .map(|row| (row.key(), row.entry.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            (Key::Server("https://prod.example.test".into()), "prod"),
            // An entry the workspace doesn't list is kept, to be pointed
            // elsewhere.
            (Key::Name("prod-lon".into()), "lon"),
        ]
    );
    assert_eq!(workspace.unknown_keys(), vec!["destinations[1].note"]);
    let written: serde_json::Value = serde_json::from_str(&to_text(&workspace)).unwrap();
    assert_eq!(written["destinations"][1]["note"], "kept");
    assert!(written["destinations"][0].get("name").is_none());
    assert_eq!(parse(to_text(&workspace).as_bytes()), Ok(workspace));
}

#[test]
fn a_workspace_without_destinations_writes_none() {
    assert!(!to_text(&sample()).contains("destinations"));
}

#[test]
fn a_destination_row_that_cannot_be_used_refuses_the_file_and_is_named() {
    let refused = |row: serde_json::Value| {
        parse(&text(serde_json::json!({
            "version": 1,
            "clusters": [{"id": "prod", "role": "environment", "context": "acme-prod"}],
            "destinations": [{"name": "fine", "entry": "prod"}, row],
        })))
        .unwrap_err()
    };
    let cases = [
        (
            serde_json::json!({"server": "https://a.example.test", "name": "a", "entry": "prod"}),
            "destinations[1] (server https://a.example.test) names both a server and a name",
        ),
        (
            serde_json::json!({"entry": "prod"}),
            "destinations[1] names no server or name",
        ),
        (
            serde_json::json!({"server": " ", "entry": "prod"}),
            "destinations[1] names an empty server",
        ),
        (
            serde_json::json!({"name": "", "entry": "prod"}),
            "destinations[1] names an empty name",
        ),
        (
            serde_json::json!({"server": "prod.example.test", "entry": "prod"}),
            "destinations[1] (server prod.example.test) names a server that isn’t an http or https address",
        ),
        (
            serde_json::json!({"server": "https://Kubernetes.default.svc:443/", "entry": "prod"}),
            "destinations[1] (server https://Kubernetes.default.svc:443/) names Argo CD's own cluster but its host entry cannot be determined; set sources.argocd to a usable cluster entry",
        ),
        (
            serde_json::json!({"name": "in-cluster", "entry": "prod"}),
            "destinations[1] (name in-cluster) names Argo CD's own cluster but its host entry cannot be determined; set sources.argocd to a usable cluster entry",
        ),
        (
            serde_json::json!({"name": "a", "entry": " "}),
            "destinations[1] (name a) names no workspace cluster",
        ),
        // The likeliest hand edit: a row whose entry was never typed.
        (
            serde_json::json!({"name": "a"}),
            "destinations[1] (name a) names no workspace cluster",
        ),
        (
            serde_json::json!({"name": "prod-lon ", "entry": "prod"}),
            "destinations[1] (name prod-lon ) names a server or name with spaces around it",
        ),
        (
            serde_json::json!({"server": 5, "entry": "prod"}),
            "destinations[1] isn’t a server or name and an entry, each a string, so it can’t be read",
        ),
        (
            serde_json::json!({"name": "a", "entry": ["prod"]}),
            "destinations[1] (name a) isn’t a server or name and an entry, each a string, so it can’t be read",
        ),
        (
            serde_json::json!("prod"),
            "destinations[1] isn’t a server or name and an entry, each a string, so it can’t be read",
        ),
    ];
    for (row, words) in cases {
        assert_eq!(refused(row.clone()).to_string(), words, "{row}");
    }
}

#[test]
fn in_cluster_aliases_follow_the_existing_argo_cd_source_selection() {
    let mut workspace = three();
    workspace
        .clusters
        .push(Entry::new("other-core", Role::Core, "other"));
    let keys = [
        Key::Server(IN_CLUSTER_SERVER.into()),
        Key::Server("https://Kubernetes.Default.Svc:443/".into()),
        Key::Name(IN_CLUSTER_NAME.into()),
    ];
    for (pin, host) in [(None, "mgmt"), (Some("ci"), "ci"), (Some("dev"), "dev")] {
        workspace.sources.set(Part::ArgoCd, pin.map(str::to_owned));
        workspace.destinations = keys
            .iter()
            .map(|key| Destination::new(key.clone(), host))
            .collect();
        assert_eq!(workspace.validate(), Ok(()));
        assert_eq!(parse(to_text(&workspace).as_bytes()), Ok(workspace.clone()));
        let sources = Sources::of(&workspace);
        assert_eq!(sources.get(Part::ArgoCd).entry().unwrap().id, host);
        assert_eq!(sources.workloads().len(), 1, "the host is listed once");
        assert_eq!(sources.workloads()[0].entry().unwrap().id, host);
        for key in &keys {
            assert_eq!(
                Destination::new(key.clone(), "other-core").fault_in(&workspace),
                Some(Fault::ArgoCdOther(host.into())),
                "an alias cannot select another host"
            );
        }
    }
    // Changing the pin changes which target is valid, with no stale alias state.
    workspace.sources.set(Part::ArgoCd, None);
    assert!(matches!(
        workspace.validate(),
        Err(Invalid::Destination {
            fault: Fault::ArgoCdOther(id),
            ..
        }) if id == "mgmt"
    ));
    workspace
        .destinations
        .iter_mut()
        .for_each(|row| row.entry = "mgmt".into());
    assert_eq!(workspace.validate(), Ok(()));
    workspace.clusters.swap(1, 3);
    assert!(matches!(
        workspace.validate(),
        Err(Invalid::Destination {
            fault: Fault::ArgoCdOther(id),
            ..
        }) if id == "other-core"
    ));
}

#[test]
fn in_cluster_aliases_require_a_known_host_and_never_supply_one() {
    for clusters in [vec![], vec![Entry::new("env", Role::Environment, "env")]] {
        let mut workspace = Workspace::new(clusters);
        for pin in [None, Some("missing")] {
            workspace.sources.set(Part::ArgoCd, pin.map(str::to_owned));
            for key in [
                Key::Server(IN_CLUSTER_SERVER.into()),
                Key::Name(IN_CLUSTER_NAME.into()),
            ] {
                workspace.destinations = vec![Destination::new(key, "env")];
                assert!(matches!(
                    workspace.validate(),
                    Err(Invalid::Destination {
                        fault: Fault::ArgoCdUnknown,
                        ..
                    })
                ));
            }
        }
    }
    let mut workspace = three();
    workspace.sources.set(Part::ArgoCd, Some("gone".into()));
    workspace.destinations = vec![Destination::new(Key::Name(IN_CLUSTER_NAME.into()), "mgmt")];
    assert!(
        matches!(
            workspace.validate(),
            Err(Invalid::Destination {
                fault: Fault::ArgoCdUnknown,
                ..
            })
        ),
        "a dangling pin never falls back to Core"
    );
    workspace.destinations[0].entry.clear();
    assert_eq!(
        workspace.destinations[0].fault_in(&workspace),
        Some(Fault::NoEntry)
    );
    // Names retain exact matching; an ordinary name still permits a gone target.
    workspace.destinations = vec![Destination::new(Key::Name("In-cluster".into()), "gone")];
    assert_eq!(workspace.validate(), Ok(()));
}

#[test]
fn too_many_destinations_refuse_the_file() {
    let mut workspace = sample();
    workspace.destinations = (0..=MAX_DESTINATIONS)
        .map(|n| Destination::new(Key::Name(format!("c{n}")), "prod"))
        .collect();
    assert_eq!(workspace.validate(), Err(Invalid::TooManyDestinations));
}

#[test]
fn a_destination_changes_what_it_matches_and_keeps_the_rest() {
    let mut workspace = parse(&text(serde_json::json!({
        "version": 1,
        "clusters": [],
        "destinations": [{"name": "a", "entry": "prod", "note": "kept"}],
    })))
    .unwrap();
    let row = &mut workspace.destinations[0];
    row.set_key(Key::Server("https://a.example.test".into()));
    assert_eq!(row.key(), Key::Server("https://a.example.test".into()));
    assert_eq!(row.entry, "prod");
    assert_eq!(workspace.unknown_keys(), vec!["destinations[0].note"]);
    assert_eq!(workspace.validate(), Ok(()));
}

#[test]
fn destinations_that_are_not_a_list_refuse_the_file() {
    let refused = parse(&text(serde_json::json!({
        "version": 1,
        "clusters": [],
        "destinations": {"name": "a", "entry": "prod"},
    })));
    assert!(matches!(refused, Err(Invalid::Malformed(_))), "{refused:?}");
}

#[test]
fn keys_name_the_same_destination_as_an_application_is_matched() {
    let server = |s: &str| Key::Server(s.into());
    let name = |s: &str| Key::Name(s.into());
    assert!(server("https://A.example.test:443/").same(&server("https://a.example.test")));
    assert!(!server("https://a.example.test:6443").same(&server("https://a.example.test")));
    assert!(name("prod").same(&name("prod")));
    assert!(!name("prod").same(&name("Prod")));
    assert!(!name("https://a.example.test").same(&server("https://a.example.test")));
}

mod sources {
    use super::*;

    fn entry(id: &str, role: Role) -> Entry {
        Entry::new(id, role, format!("{id}-context"))
    }

    fn chosen(sources: &Sources, part: Part) -> Option<(&str, Why)> {
        match sources.get(part) {
            Chosen::Entry { entry, why } => Some((entry.id.as_str(), *why)),
            _ => None,
        }
    }

    #[test]
    fn roles_choose_where_each_part_is_read() {
        let workspace = Workspace::new(vec![
            entry("cluster-c", Role::Environment),
            entry("cluster-a", Role::Core),
            entry("cluster-b", Role::Cicd),
        ]);
        let sources = Sources::of(&workspace);
        let cicd = Some(("cluster-b", Why::Role(Role::Cicd)));
        assert_eq!(chosen(&sources, Part::Kargo), cicd);
        assert_eq!(chosen(&sources, Part::Builds), cicd);
        assert_eq!(
            chosen(&sources, Part::ArgoCd),
            Some(("cluster-a", Why::Role(Role::Core)))
        );
    }

    #[test]
    fn without_a_cicd_entry_kargo_and_builds_come_from_core() {
        let workspace = Workspace::new(vec![
            entry("cluster-c", Role::Environment),
            entry("cluster-a", Role::Core),
        ]);
        let sources = Sources::of(&workspace);
        for part in Part::ALL {
            assert_eq!(
                chosen(&sources, part),
                Some(("cluster-a", Why::Role(Role::Core))),
                "{part:?}"
            );
        }
    }

    #[test]
    fn the_first_entry_listed_wins_a_tie() {
        let mut workspace = Workspace::new(vec![
            entry("cluster-a", Role::Core),
            entry("cluster-b", Role::Cicd),
            entry("cluster-d", Role::Core),
            entry("cluster-e", Role::Cicd),
        ]);
        let sources = Sources::of(&workspace);
        assert_eq!(chosen(&sources, Part::Kargo).unwrap().0, "cluster-b");
        assert_eq!(chosen(&sources, Part::ArgoCd).unwrap().0, "cluster-a");
        // Reordering the entries changes the choice.
        workspace.clusters.reverse();
        let sources = Sources::of(&workspace);
        assert_eq!(chosen(&sources, Part::Kargo).unwrap().0, "cluster-e");
        assert_eq!(chosen(&sources, Part::ArgoCd).unwrap().0, "cluster-d");
    }

    #[test]
    fn a_missing_role_is_said_never_guessed() {
        let workspace = Workspace::new(vec![
            entry("cluster-b", Role::Cicd),
            entry("cluster-c", Role::Environment),
        ]);
        let sources = Sources::of(&workspace);
        assert_eq!(chosen(&sources, Part::Kargo).unwrap().0, "cluster-b");
        let argocd = sources.get(Part::ArgoCd);
        assert_eq!(argocd, &Chosen::NoEntry(&[Role::Core]));
        assert_eq!(
            argocd.missing().as_deref(),
            Some("No Core entry in the workspace")
        );

        let environments = Workspace::new(vec![entry("cluster-c", Role::Environment)]);
        let sources = Sources::of(&environments);
        assert_eq!(
            sources.get(Part::Builds).missing().as_deref(),
            Some("No CI/CD or Core entry in the workspace")
        );
    }

    #[test]
    fn a_pinned_part_is_read_from_its_entry_and_a_gone_one_says_so() {
        let mut workspace = Workspace::new(vec![
            entry("cluster-a", Role::Core),
            entry("cluster-b", Role::Cicd),
            entry("cluster-c", Role::Environment),
        ]);
        workspace
            .sources
            .set(Part::Builds, Some("cluster-c".into()));
        workspace
            .sources
            .set(Part::ArgoCd, Some("cluster-x".into()));
        let sources = Sources::of(&workspace);
        assert_eq!(
            chosen(&sources, Part::Builds),
            Some(("cluster-c", Why::Pinned))
        );
        assert_eq!(
            chosen(&sources, Part::Kargo),
            Some(("cluster-b", Why::Role(Role::Cicd)))
        );
        // A pin to an entry that is gone never falls back to a role.
        assert_eq!(sources.get(Part::ArgoCd), &Chosen::Gone("cluster-x".into()));
        assert_eq!(
            sources.get(Part::ArgoCd).missing().as_deref(),
            Some("cluster-x is no longer in the workspace")
        );
    }

    #[test]
    fn a_workspace_of_one_reads_everything_from_the_open_connection() {
        let sources = Sources::of(&Workspace::default());
        for part in Part::ALL {
            assert_eq!(sources.get(part), &Chosen::Open, "{part:?}");
        }
        // A pin means nothing without entries.
        let mut pinned = Workspace::default();
        pinned.sources.set(Part::Kargo, Some("cluster-b".into()));
        assert_eq!(Sources::of(&pinned).get(Part::Kargo), &Chosen::Open);
    }

    #[test]
    fn the_example_reads_kargo_from_cicd_and_argo_cd_from_core() {
        let sources = Sources::of(&example());
        assert_eq!(chosen(&sources, Part::Kargo).unwrap().0, "cicd-fra");
        assert_eq!(chosen(&sources, Part::Builds).unwrap().0, "cicd-fra");
        assert_eq!(chosen(&sources, Part::ArgoCd).unwrap().0, "core-fra");
        let results = sources
            .get(Part::Builds)
            .entry()
            .unwrap()
            .results
            .as_ref()
            .unwrap();
        assert_eq!(
            results.forward,
            Some(crate::setup::Forward::new("acme", "results", 8443))
        );
        assert!(results.address.is_none() && results.token.is_none() && results.ca_file.is_none());
    }

    #[test]
    fn pins_round_trip_and_an_empty_one_is_refused() {
        let source = serde_json::json!({
            "version": 1,
            "clusters": [{"id": "cluster-a", "role": "core", "context": "a"}],
            "sources": {"builds": "cluster-a"},
        });
        let workspace = parse(&text(source)).unwrap();
        assert_eq!(workspace.sources.pin(Part::Builds), Some("cluster-a"));
        assert_eq!(parse(to_text(&workspace).as_bytes()), Ok(workspace));
        // No pins write no `sources`.
        let plain = Workspace::new(vec![entry("cluster-a", Role::Core)]);
        assert!(!to_text(&plain).contains("sources"));

        let empty = serde_json::json!({
            "version": 1,
            "clusters": [{"id": "cluster-a", "role": "core", "context": "a"}],
            "sources": {"kargo": " "},
        });
        assert_eq!(parse(&text(empty)), Err(Invalid::EmptySource("kargo")));
        let wrong = serde_json::json!({"version": 1, "sources": {"kargo": 3}});
        assert!(matches!(parse(&text(wrong)), Err(Invalid::Malformed(_))));
    }

    fn ids(chosen: &[Chosen]) -> Vec<String> {
        chosen
            .iter()
            .map(|chosen| match chosen {
                Chosen::Entry { entry, why } => {
                    assert_eq!(*why, Why::Destination);
                    entry.id.clone()
                }
                Chosen::Gone(id) => format!("gone {id}"),
                Chosen::Skipped(row) => row.label(),
                Chosen::Open => "open".into(),
                Chosen::NoEntry(_) => "none".into(),
            })
            .collect()
    }

    #[test]
    fn workloads_are_read_from_each_mapped_entry_once() {
        let mut workspace = Workspace::new(vec![
            entry("cluster-a", Role::Core),
            entry("cluster-c", Role::Environment),
            entry("cluster-d", Role::Environment),
        ]);
        assert!(Sources::of(&workspace).workloads().is_empty());
        let server = |host: &str| Key::Server(format!("https://{host}.example.test"));
        workspace.destinations = vec![
            Destination::new(server("d"), "cluster-d"),
            Destination::new(Key::Name("c".into()), "cluster-c"),
            Destination::new(server("c"), "cluster-c"),
            Destination::new(server("x"), "cluster-x"),
        ];
        assert_eq!(
            ids(Sources::of(&workspace).workloads()),
            vec!["cluster-d", "cluster-c", "gone cluster-x"]
        );
        assert_eq!(
            ids(Sources::of(&Workspace::default()).workloads()),
            vec!["open"]
        );
        assert_eq!(
            ids(Sources::of(&crate::workspace::example()).workloads()),
            vec!["dev-fra", "stage-fra", "prod-ams"]
        );
    }
}

#[test]
fn a_context_takes_the_stricter_level_of_the_entries_naming_it() {
    let at = |id: &str, context: &str, level: Option<Protection>| {
        let mut entry = Entry::new(id, Role::Environment, context);
        entry.protection = level;
        entry
    };
    let workspace = Workspace::new(vec![
        at("a", "shared", Some(Protection::Open)),
        at("b", "shared", Some(Protection::Protected)),
        at("c", "alone", None),
        at("d", "open", Some(Protection::Open)),
    ]);
    let none = SourceRecovery::default();
    assert_eq!(workspace.level_of("shared", &none), Protection::Protected);
    assert_eq!(workspace.level_of("alone", &none), Protection::Guarded);
    assert_eq!(workspace.level_of("open", &none), Protection::Open);
    // A context no entry names is guarded, whatever it is called.
    assert_eq!(workspace.level_of("production", &none), Protection::Guarded);
}
