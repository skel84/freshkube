use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};

use super::*;
use crate::setup::{Mode, Setup};

const WORKSPACE_JSON: &str = r#"{
  "version": 1,
  "flavour": "keep me",
  "clusters": [
    {"id": "cluster-a", "role": "core", "context": "cluster-a", "note": "and me"},
    {"id": "cluster-b", "role": "cicd", "context": "ctx-b"}
  ]
}"#;

const COROOT_JSON: &str = r#"{
  "url": "https://coroot.example.test",
  "auth": "api-key",
  "project": "shop-api",
  "reconnect": true,
  "remember": true,
  "theme": "kept"
}"#;

const MONITORING_JSON: &str = r#"{
  "dashboards": "/home/acme/dashboards",
  "prometheus": {"cluster-a": {"namespace": "x", "name": "y", "port": 1}},
  "sources": {
    "cluster-a": {"kind": "service", "namespace": "vm", "name": "vmselect",
                  "port": 8481, "https": false, "path": "/select/0/prometheus/"},
    "ctx-b": {"kind": "url", "url": "https://metrics.example.test/", "token": false},
    "elsewhere": {"kind": "url", "url": "https://other.example.test/", "token": false}
  }
}"#;

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap()
}

struct Home {
    directory: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn with(files: &[(&str, &str)]) -> Self {
        let home = Self::new();
        for (name, text) in files {
            home.write(name, text);
        }
        home
    }

    fn full() -> Self {
        Self::with(&[
            ("workspace.json", WORKSPACE_JSON),
            ("coroot.json", COROOT_JSON),
            ("monitoring.json", MONITORING_JSON),
        ])
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn preferences(&self) -> PathBuf {
        self.path("preferences.json")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.path(name), text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    fn has(&self, name: &str) -> bool {
        self.path(name).exists()
    }

    fn migrate(&self) -> Outcome {
        migrate(&self.preferences(), now())
    }

    /// Every file in the folder and its bytes.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        snapshot(self.directory.path())
    }

    fn setup(&self) -> Setup {
        parse(self.read("freshkube.yaml").as_bytes()).expect("freshkube.yaml is a setup")
    }
}

fn snapshot(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn done(moved: &[&'static str]) -> Outcome {
    Outcome::Done {
        moved: moved.to_vec(),
    }
}

#[test]
fn a_launch_with_nothing_to_do_does_nothing() {
    let home = Home::new();
    assert_eq!(home.migrate(), Outcome::Nothing);
    assert!(home.snapshot().is_empty());

    // Only the files that stay: no setup is made from them.
    home.write("monitoring.json", MONITORING_JSON);
    home.write("preferences.json", "{}");
    let before = home.snapshot();
    assert_eq!(home.migrate(), Outcome::Nothing);
    assert_eq!(home.snapshot(), before);
}

#[test]
fn the_files_move_into_one_setup_and_the_old_ones_become_backups() {
    let home = Home::full();
    let monitoring = home.read("monitoring.json");
    let outcome = home.migrate();
    assert_eq!(outcome, done(&["workspace.json", "coroot.json"]));
    assert_eq!(
        outcome.notice().as_deref(),
        Some("Moved workspace.json and coroot.json into freshkube.yaml; the old files are .bak")
    );

    let setup = home.setup();
    assert_eq!(setup.workspace.clusters.len(), 2);
    assert_eq!(setup.workspace.clusters[0].id, "cluster-a");
    assert_eq!(setup.metrics.mode, Mode::Automatic);
    let coroot = setup.coroot.expect("Coroot moved");
    assert_eq!(coroot.server, "https://coroot.example.test");
    assert_eq!(coroot.sign_in, SignIn::ApiKey);
    assert_eq!(coroot.project.as_deref(), Some("shop-api"));
    assert_eq!(coroot.key, Some(KeysJson));

    assert!(!home.has("workspace.json") && !home.has("coroot.json"));
    assert_eq!(home.read("workspace.json.bak"), WORKSPACE_JSON);
    assert_eq!(home.read("coroot.json.bak"), COROOT_JSON);
    assert_eq!(home.read("monitoring.json"), monitoring, "left as it is");
    assert!(!home.read("freshkube.yaml").contains("keys.json: "));
}

#[cfg(unix)]
#[test]
fn the_new_file_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::full();
    home.migrate();
    let mode = std::fs::metadata(home.path("freshkube.yaml"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn unknown_keys_survive_the_move() {
    let home = Home::full();
    home.migrate();
    let setup = home.setup();
    assert_eq!(
        setup.unknown_keys(),
        ["cluster-a.note", "coroot.theme", "flavour"]
    );
    assert!(home.read("freshkube.yaml").contains("keep me"));
}

#[test]
fn reconnect_goes_to_preferences_and_the_other_keys_stay() {
    let home = Home::with(&[
        ("workspace.json", WORKSPACE_JSON),
        ("coroot.json", COROOT_JSON),
        ("preferences.json", r#"{"text_size": 14, "mystery": [1]}"#),
    ]);
    home.migrate();
    let preferences: serde_json::Value = serde_json::from_str(&home.read("preferences.json")).unwrap();
    assert_eq!(preferences["coroot_reconnect"], true);
    assert_eq!(preferences["text_size"], 14);
    assert_eq!(preferences["mystery"], serde_json::json!([1]));
    assert!(!home.read("freshkube.yaml").contains("reconnect"));
}

#[test]
fn no_preferences_file_is_made_when_nothing_reconnects() {
    let coroot = COROOT_JSON.replace("\"reconnect\": true", "\"reconnect\": false");
    let home = Home::with(&[("workspace.json", WORKSPACE_JSON), ("coroot.json", &coroot)]);
    home.migrate();
    assert!(!home.has("preferences.json"));
}

#[test]
fn a_key_is_named_only_for_a_key_taking_sign_in_that_remembers() {
    for (auth, remember, key) in [
        ("api-key", true, true),
        ("session", true, true),
        ("anonymous", true, false),
        ("api-key", false, false),
    ] {
        let coroot =
            format!(r#"{{"url": "https://coroot.example.test", "auth": "{auth}", "remember": {remember}}}"#);
        let home = Home::with(&[("workspace.json", WORKSPACE_JSON), ("coroot.json", &coroot)]);
        home.migrate();
        let saved = home.setup().coroot.unwrap();
        assert_eq!(saved.key.is_some(), key, "{auth} {remember}");
    }
}

#[test]
fn the_source_monitoring_chose_for_an_entrys_context_is_copied() {
    let home = Home::full();
    home.migrate();
    let setup = home.setup();
    let a = setup.workspace.clusters[0].metrics.as_ref().unwrap();
    let service = a.source.as_ref().unwrap().service.as_ref().unwrap();
    assert_eq!(
        (service.namespace.as_str(), service.name.as_str(), service.port),
        ("vm", "vmselect", 8481)
    );
    assert_eq!(service.path, "select/0/prometheus");
    let b = setup.workspace.clusters[1].metrics.as_ref().unwrap();
    assert_eq!(
        b.source.as_ref().unwrap().url.as_deref(),
        Some("https://metrics.example.test/")
    );
    // The mode stays automatic; a context that is no entry is not copied.
    assert_eq!(setup.metrics.mode, Mode::Automatic);
    assert!(setup.metrics.store.is_none());
    assert!(!home.read("freshkube.yaml").contains("other.example.test"));
}

#[test]
fn a_source_with_a_token_in_the_credential_store_is_not_copied() {
    let monitoring = MONITORING_JSON.replace(
        r#""url": "https://metrics.example.test/", "token": false"#,
        r#""url": "https://metrics.example.test/", "token": true"#,
    );
    let home = Home::with(&[
        ("workspace.json", WORKSPACE_JSON),
        ("monitoring.json", &monitoring),
    ]);
    home.migrate();
    assert!(home.setup().workspace.clusters[1].metrics.is_none());
    assert_eq!(home.read("monitoring.json"), monitoring);
}

#[test]
fn a_source_the_entry_already_names_is_not_replaced() {
    let workspace = r#"{"version": 1, "clusters": [{"id": "cluster-a", "role": "core",
        "context": "cluster-a",
        "metrics": {"value": "ci-a", "source": {"url": "https://mine.example.test"}}}]}"#;
    let home = Home::with(&[("workspace.json", workspace), ("monitoring.json", MONITORING_JSON)]);
    home.migrate();
    let setup = home.setup();
    let metrics = setup.workspace.clusters[0].metrics.as_ref().unwrap();
    assert_eq!(metrics.value.as_deref(), Some("ci-a"));
    assert_eq!(
        metrics.source.as_ref().unwrap().url.as_deref(),
        Some("https://mine.example.test")
    );
}

#[test]
fn an_unusable_coroot_file_counts_as_no_coroot() {
    for coroot in ["not json", r#"{"url": "https://c.example.test", "auth": "magic"}"#, r#"{"auth": "session"}"#, r#"{"url": "not a url", "auth": "session"}"#] {
        let home = Home::with(&[("workspace.json", WORKSPACE_JSON), ("coroot.json", coroot)]);
        let outcome = home.migrate();
        assert_eq!(outcome, done(&["workspace.json", "coroot.json"]), "{coroot}");
        assert!(home.setup().coroot.is_none(), "{coroot}");
        assert_eq!(home.setup().workspace.clusters.len(), 2);
        assert_eq!(home.read("coroot.json.bak"), coroot, "nothing is lost");
    }
}

#[test]
fn coroot_alone_still_makes_a_setup() {
    let home = Home::with(&[("coroot.json", COROOT_JSON)]);
    assert_eq!(home.migrate(), done(&["coroot.json"]));
    let setup = home.setup();
    assert!(setup.workspace.clusters.is_empty());
    assert!(setup.coroot.is_some());
}

#[test]
fn an_unusable_workspace_file_migrates_nothing() {
    for workspace in [
        "not json".to_owned(),
        r#"{"version": 2, "clusters": []}"#.to_owned(),
        r#"{"version": 1, "clusters": [{"id": "a", "role": "core", "context": "a"}, {"id": "a", "role": "core", "context": "b"}]}"#.to_owned(),
    ] {
        let home = Home::with(&[
            ("workspace.json", &workspace),
            ("coroot.json", COROOT_JSON),
            ("monitoring.json", MONITORING_JSON),
        ]);
        let before = home.snapshot();
        assert_eq!(home.migrate(), Outcome::Kept(Kept::WorkspaceUnusable));
        assert_eq!(home.snapshot(), before, "nothing was written or moved");
        // And the next launch tries again, with the same result.
        assert_eq!(home.migrate(), Outcome::Kept(Kept::WorkspaceUnusable));
        assert_eq!(home.snapshot(), before);
    }
}

#[test]
fn a_workspace_that_cannot_be_written_back_as_read_stays_in_use() {
    // An unknown top-level key that the setup writes for itself.
    let workspace = r#"{"version": 1, "clusters": [], "metrics": {"mode": "central"}}"#;
    let home = Home::with(&[("workspace.json", workspace), ("coroot.json", COROOT_JSON)]);
    let before = home.snapshot();
    assert_eq!(home.migrate(), Outcome::Kept(Kept::NotSafe));
    assert_eq!(home.snapshot(), before);
}

#[test]
fn preferences_that_cannot_take_the_flag_keep_every_file_in_use() {
    let home = Home::with(&[
        ("workspace.json", WORKSPACE_JSON),
        ("coroot.json", COROOT_JSON),
        ("preferences.json", "[ this is not an object"),
    ]);
    let before = home.snapshot();
    assert_eq!(home.migrate(), Outcome::Kept(Kept::NotSafe));
    assert_eq!(home.snapshot(), before);
}

#[test]
fn a_setup_that_exists_wins_and_the_old_files_are_set_aside() {
    let mine = "version: 1\nclusters:\n  - id: cluster-c\n    role: core\n    context: cluster-c\n";
    let home = Home::full();
    home.write("freshkube.yaml", mine);
    assert_eq!(home.migrate(), done(&["workspace.json", "coroot.json"]));
    assert_eq!(home.read("freshkube.yaml"), mine, "nothing is merged");
    assert_eq!(home.read("workspace.json.bak"), WORKSPACE_JSON);
    assert_eq!(home.read("coroot.json.bak"), COROOT_JSON);
    assert!(!home.has("preferences.json"), "nothing is read from them");
}

#[test]
fn a_setup_that_cannot_be_used_leaves_the_old_files_alone() {
    for text in ["version: [", "version: 2\n", "clusters: []\n"] {
        let home = Home::full();
        home.write("freshkube.yaml", text);
        let before = home.snapshot();
        assert_eq!(home.migrate(), Outcome::Kept(Kept::SetupUnusable), "{text}");
        assert_eq!(home.snapshot(), before);
    }
}

#[test]
fn a_launch_that_stopped_after_the_workspace_was_set_aside_is_finished() {
    let home = Home::full();
    home.migrate();
    // Put coroot.json back, as if the launch stopped between the two renames.
    home.write("coroot.json", COROOT_JSON);
    std::fs::remove_file(home.path("coroot.json.bak")).unwrap();
    let setup = home.read("freshkube.yaml");
    assert_eq!(home.migrate(), done(&["coroot.json"]));
    assert_eq!(home.read("freshkube.yaml"), setup);
    assert!(home.has("coroot.json.bak") && !home.has("coroot.json"));
    assert_eq!(home.migrate(), Outcome::Nothing);
}

#[test]
fn a_launch_that_stopped_after_the_flag_was_kept_is_finished() {
    let home = Home::with(&[
        ("workspace.json", WORKSPACE_JSON),
        ("coroot.json", COROOT_JSON),
        // The person has since turned reconnecting off.
        ("preferences.json", r#"{"coroot_reconnect": false}"#),
    ]);
    assert_eq!(home.migrate(), done(&["workspace.json", "coroot.json"]));
    assert!(home.read("preferences.json").contains("false"), "kept as it was");
    assert!(home.has("freshkube.yaml"));
}

#[test]
fn a_launch_that_stopped_before_linking_leaves_nothing_in_the_way() {
    let home = Home::full();
    // The temporary file a launch that died left behind.
    home.write("freshkube.yaml.1.0.tmp", "version: 1\n");
    assert_eq!(home.migrate(), done(&["workspace.json", "coroot.json"]));
    assert!(!home.setup().workspace.clusters.is_empty());
    assert_eq!(home.read("freshkube.yaml.1.0.tmp"), "version: 1\n");
}

#[test]
fn a_backup_that_exists_is_never_replaced() {
    let home = Home::full();
    home.write("workspace.json.bak", "an older backup");
    home.migrate();
    assert_eq!(home.read("workspace.json.bak"), "an older backup");
    assert_eq!(
        home.read("workspace.20261019T120000Z.bak"),
        WORKSPACE_JSON
    );
}

#[test]
fn a_launch_that_loses_the_race_reads_the_file_that_won() {
    let home = Home::full();
    let theirs = "version: 1\nclusters:\n  - id: cluster-z\n    role: core\n    context: cluster-z\n";
    let outcome = run(&home.preferences(), now(), || {
        // Another launch links its file first.
        std::fs::write(home.path("freshkube.yaml"), theirs).unwrap();
    });
    assert_eq!(outcome, done(&["workspace.json", "coroot.json"]));
    assert_eq!(home.read("freshkube.yaml"), theirs, "never replaced");
    assert!(
        home.snapshot().keys().all(|name| !name.ends_with(".tmp")),
        "{:?}",
        home.snapshot().keys()
    );
    assert_eq!(home.read("workspace.json.bak"), WORKSPACE_JSON);
}

#[test]
fn launches_at_once_make_one_setup_and_lose_nothing() {
    for _ in 0..20 {
        let home = Home::full();
        let barrier = std::sync::Barrier::new(6);
        let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        home.migrate()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(
            outcomes.iter().all(|o| matches!(o, Outcome::Done { .. } | Outcome::Nothing)),
            "{outcomes:?}"
        );
        let setup = home.setup();
        assert_eq!(setup.workspace.clusters.len(), 2);
        assert!(setup.coroot.is_some());
        let files = home.snapshot();
        assert!(!files.contains_key("workspace.json") && !files.contains_key("coroot.json"));
        for (name, bytes) in &files {
            assert!(!name.ends_with(".tmp"), "{name}");
            if name.starts_with("workspace.") && name.ends_with(".bak") {
                assert_eq!(bytes, WORKSPACE_JSON.as_bytes(), "{name}");
            }
            if name.starts_with("coroot.") && name.ends_with(".bak") {
                assert_eq!(bytes, COROOT_JSON.as_bytes(), "{name}");
            }
        }
        assert!(files.contains_key("workspace.json.bak") && files.contains_key("coroot.json.bak"));
        assert_eq!(home.read("monitoring.json"), MONITORING_JSON);
    }
}

#[test]
fn the_notice_names_what_moved_and_is_empty_when_nothing_did() {
    assert_eq!(done(&[]).notice(), None);
    assert_eq!(Outcome::Nothing.notice(), None);
    assert_eq!(Outcome::Kept(Kept::NotSafe).notice(), None);
    assert_eq!(
        done(&["workspace.json"]).notice().as_deref(),
        Some("Moved workspace.json into freshkube.yaml; the old file is .bak")
    );
}
