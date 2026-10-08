use super::*;
use chrono::TimeZone;

fn acme() -> Workspace {
    Workspace {
        kubeconfig: Some(PathBuf::from("/example/acme/kubeconfig")),
        clusters: vec![
            Entry::new("mgmt", Some(Role::Core), "acme-mgmt")
                .with_talosconfig("/example/acme/talosconfig"),
            Entry::new("ci", Some(Role::Cicd), "acme-ci"),
            Entry::new("prod", Some(Role::Environment), "acme-prod"),
            Entry::new("scratch", None, "acme-scratch"),
        ],
        extra: Default::default(),
    }
}

#[test]
fn a_workspace_round_trips_through_its_file() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    assert_eq!(load(&file), Loaded::Missing);
    save(&file, &acme()).unwrap();
    assert_eq!(load(&file), Loaded::Workspace(acme()));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"version\": 1"));
    assert!(text.contains("\"role\": \"cicd\""));
    // An entry with no role writes none.
    assert_eq!(text.matches("\"role\"").count(), 3);
}

#[test]
fn keys_this_version_does_not_know_survive_a_save() {
    let text = r#"{"version":1,"clusters":[{"id":"mgmt","role":"core","context":"acme-mgmt","note":"kept"}],
        "destinations":[{"server":"https://prod.example.test","entry":"mgmt"}],"sources":{"a":1}}"#;
    let workspace = parse(text.as_bytes()).unwrap();
    let again = parse(to_text(&workspace).as_bytes()).unwrap();
    assert_eq!(workspace, again);
    let value: serde_json::Value = serde_json::from_str(&to_text(&workspace)).unwrap();
    assert_eq!(
        value["destinations"][0]["server"],
        "https://prod.example.test"
    );
    assert_eq!(value["sources"]["a"], 1);
    assert_eq!(value["clusters"][0]["note"], "kept");
}

#[test]
fn a_file_the_app_cannot_use_is_refused_with_a_reason() {
    let cases: [(&str, Invalid); 9] = [
        ("not json", Invalid::NotJson),
        ("{}", Invalid::UnknownVersion(None)),
        (r#"{"version":2}"#, Invalid::UnknownVersion(Some(2))),
        (
            r#"{"version":1,"clusters":[{"id":"a","context":"x","role":"admin"}]}"#,
            Invalid::Malformed(String::new()),
        ),
        (
            r#"{"version":1,"clusters":[{"id":"","context":"x"}]}"#,
            Invalid::EmptyId,
        ),
        (
            r#"{"version":1,"clusters":[{"id":"a","context":"x"},{"id":"a","context":"y"}]}"#,
            Invalid::DuplicateId("a".into()),
        ),
        (
            r#"{"version":1,"clusters":[{"id":"a","context":" "}]}"#,
            Invalid::EmptyContext("a".into()),
        ),
        (
            r#"{"version":1,"clusters":[{"id":"a","context":"x","talosconfig":"rel"}]}"#,
            Invalid::RelativePath("a".into()),
        ),
        (
            r#"{"version":1,"kubeconfig":"rel"}"#,
            Invalid::RelativePath("the workspace".into()),
        ),
    ];
    for (text, expected) in cases {
        match (parse(text.as_bytes()), expected) {
            (Err(Invalid::Malformed(_)), Invalid::Malformed(_)) => {}
            (got, expected) => assert_eq!(got, Err(expected), "{text}"),
        }
    }
}

#[test]
fn too_many_clusters_and_oversized_files_are_refused() {
    let mut workspace = Workspace::default();
    for n in 0..=MAX_ENTRIES {
        workspace
            .clusters
            .push(Entry::new(format!("c{n}"), None, "x"));
    }
    assert_eq!(workspace.validate(), Err(Invalid::TooManyEntries));
    let long = Workspace {
        clusters: vec![Entry::new("a".repeat(MAX_ID_BYTES + 1), None, "x")],
        ..Default::default()
    };
    assert!(matches!(long.validate(), Err(Invalid::LongId(_))));

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    std::fs::write(&file, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
    assert_eq!(load(&file), Loaded::Refused(Invalid::TooLarge));
}

#[test]
fn saving_an_invalid_workspace_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("workspace.json");
    let bad = Workspace {
        clusters: vec![Entry::new("a", None, ""), Entry::new("a", None, "x")],
        ..Default::default()
    };
    assert!(save(&file, &bad).is_err());
    assert!(!file.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn a_refused_file_is_set_aside_and_never_over_an_earlier_backup() {
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

    assert_eq!(std::fs::read_to_string(first).unwrap(), "first");
    assert_eq!(std::fs::read_to_string(second).unwrap(), "second");
    assert_eq!(std::fs::read_to_string(third).unwrap(), "third");
}

#[test]
fn the_start_entry_is_the_remembered_one_else_the_first_core() {
    let workspace = acme();
    assert_eq!(workspace.start_entry(Some("prod")).unwrap().id, "prod");
    assert_eq!(workspace.start_entry(Some("gone")).unwrap().id, "mgmt");
    assert_eq!(workspace.start_entry(None).unwrap().id, "mgmt");
    let no_core = Workspace {
        clusters: vec![Entry::new("ci", Some(Role::Cicd), "acme-ci")],
        ..Default::default()
    };
    assert!(no_core.start_entry(Some("gone")).is_none());
}

#[test]
fn fresh_ids_come_from_the_context_and_never_repeat() {
    let mut workspace = acme();
    assert_eq!(workspace.fresh_id("Acme Staging!"), "acme-staging");
    assert_eq!(workspace.fresh_id("???"), "cluster");
    workspace.clusters.push(Entry::new("acme-ci", None, "x"));
    assert_eq!(workspace.fresh_id("acme-ci"), "acme-ci-2");
}
