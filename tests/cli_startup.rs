//! The read commands start nothing else: no log file, no preferences, no
//! window. Each run gets an empty home, config and temp directory, and must
//! leave them empty (a kubeconfig the test wrote aside).

use std::path::Path;
use std::process::{Command, Output};

fn freshkube(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_freshkube"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_DATA_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env("APPDATA", home)
        .env("TMPDIR", home)
        .env("TMP", home)
        .env("TEMP", home)
        .output()
        .expect("the freshkube binary runs")
}

fn left_behind(home: &Path) -> Vec<String> {
    let mut names: Vec<String> = walk(home);
    names.sort();
    names
}

fn walk(directory: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        names.push(path.display().to_string());
        if path.is_dir() {
            names.extend(walk(&path));
        }
    }
    names
}

#[test]
fn check_with_no_cluster_is_a_usage_error_that_leaves_nothing_behind() {
    let home = tempfile::tempdir().unwrap();
    let output = freshkube(home.path(), &["check"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("choose a cluster"), "{stderr}");
    assert_eq!(left_behind(home.path()), Vec::<String>::new());
}

#[test]
fn a_log_file_option_is_refused_beside_a_command_and_nothing_is_created() {
    let home = tempfile::tempdir().unwrap();
    let log = home.path().join("run.log");
    let output = freshkube(home.path(), &["--log-file", log.to_str().unwrap(), "check"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(!log.exists());
    assert_eq!(left_behind(home.path()), Vec::<String>::new());
}

#[test]
fn a_command_on_a_dead_server_exits_3_and_writes_no_log_or_preferences() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let kubeconfig = elsewhere.path().join("config");
    std::fs::write(
        &kubeconfig,
        "apiVersion: v1\nkind: Config\nclusters:\n- name: dead\n  cluster: {server: 'http://127.0.0.1:1'}\n\
         contexts:\n- name: c\n  context: {cluster: dead, user: u}\nusers:\n- name: u\n  user: {token: tok-not-for-output}\n",
    )
    .unwrap();
    let output = freshkube(
        home.path(),
        &[
            "get",
            "nodes",
            "--kubeconfig",
            kubeconfig.to_str().unwrap(),
            "--kube-context",
            "c",
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains('?'), "{text}");
    assert!(!text.contains("tok-not-for-output"));
    assert_eq!(left_behind(home.path()), Vec::<String>::new());
}
