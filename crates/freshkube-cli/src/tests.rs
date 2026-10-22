//! The commands against a fake API: every exit code, the JSON, the refusals,
//! the explicit cluster and the Secret rule. No real cluster, no credentials.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use clap::Parser;
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, KubeClientSource};
use freshkube_core::failure::{Failure, FailureKind};
use freshkube_core::fake_api::{self, Recorder, json as answer};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::*;

const SECRET_VALUE: &str = "c3VwZXItc2VjcmV0LXZhbHVl";
const TOKEN: &str = "tok-0123456789-not-for-output";

#[derive(Parser)]
struct Line {
    #[command(subcommand)]
    command: Command,
}

fn parse(words: &[&str]) -> Result<Command, clap::Error> {
    let mut line = vec!["freshkube"];
    line.extend(words);
    Line::try_parse_from(line).map(|line| line.command)
}

fn command(words: &str) -> Command {
    let words: Vec<&str> = words.split_whitespace().collect();
    parse(&words).expect("a valid command line")
}

fn table(columns: &[&str], rows: &[(&str, Option<&str>, &[&str])]) -> Value {
    json!({
        "kind": "Table",
        "apiVersion": "meta.k8s.io/v1",
        "metadata": {"resourceVersion": "1"},
        "columnDefinitions": columns.iter().map(|name| json!({"name": name, "type": "string", "priority": 0})).collect::<Vec<_>>(),
        "rows": rows.iter().map(|(name, namespace, cells)| json!({
            "cells": cells,
            "object": {
                "kind": "PartialObjectMetadata",
                "metadata": {"name": name, "namespace": namespace},
                // A server that over-shares: nothing here may reach the output.
                "data": {"password": SECRET_VALUE},
                "stringData": {"token": SECRET_VALUE},
            },
        })).collect::<Vec<_>>(),
    })
}

fn status(code: u16, reason: &str) -> (u16, String) {
    answer(
        code,
        json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
               "reason": reason, "code": code, "message": format!("{reason} by the fake API")}),
    )
}

fn healthy() -> HashMap<String, (u16, String)> {
    let ok = |value: Value| answer(200, value);
    HashMap::from([
        (
            "/api/v1/nodes".into(),
            ok(table(
                &["Name", "Status", "Age"],
                &[
                    ("node-a", None, &["node-a", "Ready", "9d"]),
                    ("node-b", None, &["node-b", "Ready", "9d"]),
                ],
            )),
        ),
        (
            "/api/v1/pods".into(),
            ok(table(
                &["Name", "Ready", "Status", "Restarts", "Age"],
                &[
                    (
                        "api-1",
                        Some("shop"),
                        &["api-1", "1/1", "Running", "0", "2d"],
                    ),
                    (
                        "job-1",
                        Some("shop"),
                        &["job-1", "0/1", "Completed", "0", "2d"],
                    ),
                ],
            )),
        ),
        (
            "/apis/apps/v1/deployments".into(),
            ok(table(
                &["Name", "Ready", "Up-to-date", "Available", "Age"],
                &[("api", Some("shop"), &["api", "2/2", "2", "2", "2d"])],
            )),
        ),
        (
            "/apis/apps/v1/statefulsets".into(),
            ok(table(
                &["Name", "Ready", "Age"],
                &[("db", Some("shop"), &["db", "1/1", "2d"])],
            )),
        ),
        (
            "/api/v1/secrets".into(),
            ok(table(
                &["Name", "Type", "Data", "Age"],
                &[(
                    "db-credentials",
                    Some("shop"),
                    &["db-credentials", "Opaque", "2", "2d"],
                )],
            )),
        ),
    ])
}

/// A cluster that answers from a table of routes, or is unreachable.
struct Fake {
    client: Result<kube::Client, Failure>,
    recorder: Recorder,
    opened: Mutex<Vec<Request>>,
    logins: Mutex<Vec<bool>>,
}

impl Fake {
    fn answering(routes: HashMap<String, (u16, String)>) -> Arc<Self> {
        let (client, recorder) =
            fake_api::client(move |request| routes.get(&request.path).cloned());
        Arc::new(Self {
            client: Ok(client),
            recorder,
            opened: Mutex::default(),
            logins: Mutex::default(),
        })
    }

    fn unreachable() -> Arc<Self> {
        Arc::new(Self {
            client: Err(Failure::new(FailureKind::Unreachable, "connection refused")),
            recorder: Recorder::default(),
            opened: Mutex::default(),
            logins: Mutex::default(),
        })
    }
}

struct Access(Arc<Fake>);

impl KubeClientSource for Access {
    fn client(&self) -> BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.client.clone();
        Box::pin(async move { client })
    }
}

impl Connector for Arc<Fake> {
    fn open(&self, request: &Request, allow_login: bool) -> Result<Target, Stop> {
        self.opened.lock().unwrap().push(request.clone());
        self.logins.lock().unwrap().push(allow_login);
        Ok(Target {
            entry: request.entry.clone(),
            context: request.context.clone(),
            source: ClusterSource {
                id: "fake".into(),
                context: request.context.clone(),
                access: ClusterAccess::Live(Arc::new(Access(self.clone()))),
            },
        })
    }
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.out)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.out))
    }
}

async fn run_in(fake: &Arc<Fake>, setup: Option<std::path::PathBuf>, words: &str) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let env = Env {
        default_setup: setup,
        connector: fake,
    };
    let code = execute(&command(words), &env, &mut out, &mut err).await;
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

async fn run(fake: &Arc<Fake>, words: &str) -> Run {
    run_in(fake, None, words).await
}

const KUBE: &str = "--kubeconfig /k --kube-context acme-prod";

fn setup_file(directory: &tempfile::TempDir) -> std::path::PathBuf {
    let path = directory.path().join("freshkube.yaml");
    let mut workspace = freshkube_core::workspace::example();
    // The example links core-fra to a Coroot project the file would have to name.
    workspace.clusters[0].coroot = None;
    let setup = freshkube_core::setup::Setup::new(workspace);
    std::fs::write(&path, freshkube_core::setup::to_text(&setup)).unwrap();
    path
}

#[tokio::test]
async fn a_healthy_cluster_checks_clean_with_exit_0() {
    let fake = Fake::answering(healthy());
    let result = run(&fake, &format!("check {KUBE}")).await;
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(result.out.contains("+ Nodes are ready"), "{}", result.out);
    assert!(result.err.is_empty());
}

#[tokio::test]
async fn a_failing_pod_exits_1_with_its_fix() {
    let mut routes = healthy();
    routes.insert(
        "/api/v1/pods".into(),
        answer(
            200,
            table(
                &["Name", "Ready", "Status", "Restarts", "Age"],
                &[(
                    "api-2",
                    Some("shop"),
                    &["api-2", "0/1", "CrashLoopBackOff", "9", "2d"],
                )],
            ),
        ),
    );
    let fake = Fake::answering(routes);
    let result = run(&fake, &format!("check {KUBE}")).await;
    assert_eq!(result.code, 1);
    assert!(result.out.contains("shop/api-2"), "{}", result.out);
    assert!(result.out.contains("fix:"), "{}", result.out);
    let pods = &result.json_for(&fake, "pods-healthy").await;
    assert_eq!(pods["state"], "fail");
    assert_eq!(pods["items"][0]["name"], "api-2");
    assert!(pods["items"][0]["fix"].is_string());
    assert_eq!(pods["items"][0]["view"]["page"], "resources");
}

impl Run {
    /// The check with `id` from the same command run with `--json`.
    async fn json_for(&self, fake: &Arc<Fake>, id: &str) -> Value {
        let again = run(fake, &format!("check {KUBE} --json")).await;
        again.json()["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["id"] == id)
            .cloned()
            .expect("the check")
    }
}

fn warning_only() -> HashMap<String, (u16, String)> {
    let mut routes = healthy();
    routes.insert(
        "/apis/apps/v1/deployments".into(),
        answer(
            200,
            table(
                &["Name", "Ready", "Up-to-date", "Available", "Age"],
                &[("api", Some("shop"), &["api", "1/2", "2", "1", "2d"])],
            ),
        ),
    );
    routes
}

#[tokio::test]
async fn warnings_exit_0_unless_fail_on_warn() {
    let fake = Fake::answering(warning_only());
    let plain = run(&fake, &format!("check {KUBE}")).await;
    assert_eq!(plain.code, 0, "{}", plain.out);
    assert!(plain.out.contains("! Deployments"), "{}", plain.out);
    let strict = run(&fake, &format!("check {KUBE} --fail-on warn")).await;
    assert_eq!(strict.code, 1);
}

#[tokio::test]
async fn a_refused_read_names_its_verb_kind_namespace_and_context_and_exits_4() {
    let mut routes = healthy();
    routes.insert(
        "/api/v1/namespaces/shop/pods".into(),
        status(403, "Forbidden"),
    );
    let fake = Fake::answering(routes);
    let text = run(&fake, &format!("get pods -n shop {KUBE}")).await;
    assert_eq!(text.code, 4);
    assert!(
        text.out
            .contains(r#"refused: list pods in namespace "shop" on context "acme-prod""#),
        "{}",
        text.out
    );
    let json = run(&fake, &format!("get pods -n shop {KUBE} --json")).await;
    assert_eq!(json.code, 4);
    let json = json.json();
    let problem = &json["problems"][0];
    assert_eq!(problem["state"], "refused");
    assert_eq!(problem["verb"], "list");
    assert_eq!(problem["kind"], "pods");
    assert_eq!(problem["namespace"], "shop");
    assert_eq!(problem["context"], "acme-prod");
    assert_eq!(json["exit"], 4);
}

#[tokio::test]
async fn an_unreachable_cluster_shows_a_question_mark_and_exits_3() {
    let fake = Fake::unreachable();
    let get = run(&fake, &format!("get nodes {KUBE}")).await;
    assert_eq!(get.code, 3);
    assert!(get.out.starts_with("? unknown: list nodes"), "{}", get.out);
    let check = run(&fake, &format!("check {KUBE}")).await;
    assert_eq!(check.code, 3);
    assert!(check.out.contains("? Nodes are ready"), "{}", check.out);
    let json = run(&fake, &format!("check {KUBE} --json")).await.json();
    assert_eq!(json["checks"][0]["state"], "unknown");
}

#[tokio::test]
async fn a_rejected_sign_in_exits_5() {
    let mut routes = healthy();
    routes.insert("/api/v1/nodes".into(), status(401, "Unauthorized"));
    let fake = Fake::answering(routes);
    assert_eq!(run(&fake, &format!("get nodes {KUBE}")).await.code, 5);
}

#[tokio::test]
async fn when_several_apply_the_fixed_order_decides() {
    // A refusal and a sign-in: the refusal (4) wins over the sign-in (5).
    let mut routes = healthy();
    routes.insert("/api/v1/nodes".into(), status(403, "Forbidden"));
    routes.insert("/api/v1/pods".into(), status(401, "Unauthorized"));
    let fake = Fake::answering(routes);
    assert_eq!(run(&fake, &format!("check {KUBE}")).await.code, 4);
    // A failed check and a refusal: the failure (1) wins.
    let mut routes = healthy();
    routes.insert("/api/v1/nodes".into(), status(403, "Forbidden"));
    routes.insert(
        "/api/v1/pods".into(),
        answer(
            200,
            table(
                &["Name", "Ready", "Status"],
                &[("p", Some("shop"), &["p", "0/1", "Error"])],
            ),
        ),
    );
    let fake = Fake::answering(routes);
    assert_eq!(run(&fake, &format!("check {KUBE}")).await.code, 1);
    // A usage error is checked first, before anything is read.
    let fake = Fake::answering(healthy());
    let usage = run(
        &fake,
        "get secrets --reveal --kubeconfig /k --kube-context acme-prod",
    )
    .await;
    assert_eq!(usage.code, 2);
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test]
async fn json_output_has_the_versioned_shape_and_stdout_is_only_json() {
    let fake = Fake::answering(healthy());
    let get = run(&fake, &format!("get pods {KUBE} --json")).await;
    assert_eq!(get.out.lines().count(), 1, "one JSON document: {}", get.out);
    let json = get.json();
    assert_eq!(json["version"], 1);
    assert_eq!(json["command"], "get");
    assert_eq!(json["exit"], 0);
    assert_eq!(json["cluster"]["context"], "acme-prod");
    assert!(json["cluster"]["entry"].is_null());
    assert_eq!(json["kind"]["key"], "pods");
    assert_eq!(
        json["columns"],
        json!(["Name", "Ready", "Status", "Restarts", "Age"])
    );
    let item = &json["items"][0];
    assert_eq!(item["namespace"], "shop");
    assert_eq!(item["name"], "api-1");
    assert_eq!(item["cells"]["Status"], "Running");
    assert_eq!(item["state"], "good");
    assert_eq!(
        item["view"],
        json!({"page": "resources", "kind": "pods", "namespace": "shop", "name": "api-1"})
    );
    // The completed job is fine, not a failure.
    assert_eq!(json["items"][1]["state"], "good");

    let check = run(&fake, &format!("check {KUBE} --json")).await.json();
    assert_eq!(check["version"], 1);
    assert_eq!(check["command"], "check");
    for entry in check["checks"].as_array().unwrap() {
        for key in ["id", "title", "state", "summary", "items"] {
            assert!(entry.get(key).is_some(), "{entry}");
        }
        assert!(
            ["pass", "warn", "fail", "unknown", "refused", "sign-in"]
                .contains(&entry["state"].as_str().unwrap())
        );
    }
    assert_eq!(check["summary"]["pass"], 4);

    // Errors go to stderr, and a usage error leaves stdout empty.
    let usage = run(
        &fake,
        "get nonsense --kubeconfig /k --kube-context c --json",
    )
    .await;
    assert_eq!(usage.code, 2);
    assert!(usage.out.is_empty());
    assert!(usage.err.contains("unknown kind"), "{}", usage.err);
}

#[tokio::test]
async fn a_not_ready_pod_that_runs_is_a_warning() {
    let mut routes = healthy();
    routes.insert(
        "/api/v1/pods".into(),
        answer(
            200,
            table(
                &["Name", "Ready", "Status"],
                &[("slow", Some("shop"), &["slow", "1/2", "Running"])],
            ),
        ),
    );
    let fake = Fake::answering(routes);
    let json = run(&fake, &format!("get pods {KUBE} --json")).await.json();
    assert_eq!(json["items"][0]["state"], "warn");
}

#[tokio::test]
async fn naming_an_object_reads_one_and_a_missing_one_exits_1() {
    let fake = Fake::answering(healthy());
    let found = run(&fake, &format!("get nodes node-a {KUBE}")).await;
    assert_eq!(found.code, 0);
    assert!(
        fake.recorder
            .lines()
            .iter()
            .any(|l| l.contains("fieldSelector=metadata.name%3Dnode-a")),
        "{:?}",
        fake.recorder.lines()
    );
    let mut routes = healthy();
    routes.insert(
        "/api/v1/nodes".into(),
        answer(200, table(&["Name", "Status"], &[])),
    );
    let fake = Fake::answering(routes);
    let missing = run(&fake, &format!("get nodes node-z {KUBE}")).await;
    assert_eq!(missing.code, 1);
    assert!(missing.out.contains("not found"), "{}", missing.out);
}

#[tokio::test]
async fn every_request_is_a_get_of_a_collection() {
    let fake = Fake::answering(healthy());
    run(&fake, &format!("check {KUBE}")).await;
    run(&fake, &format!("get pods {KUBE}")).await;
    run(&fake, &format!("get secrets {KUBE}")).await;
    let requests = fake.recorder.requests();
    assert!(requests.len() >= 6);
    fake.recorder.assert_all("the CLI only reads", |request| {
        request.method.as_str() == "GET"
            && !request.query.contains("watch")
            && !request.path.contains("/exec")
            && !request.path.contains("/portforward")
            && !request.path.contains("/proxy")
    });
}

#[tokio::test]
async fn secrets_are_listed_by_metadata_and_no_value_reaches_the_output() {
    let fake = Fake::answering(healthy());
    let text = run(&fake, &format!("get secrets {KUBE}")).await;
    assert_eq!(text.code, 0);
    assert!(text.out.contains("db-credentials"));
    assert!(text.out.contains("Opaque"));
    let line = fake
        .recorder
        .lines()
        .into_iter()
        .find(|line| line.contains("/secrets"))
        .expect("a secrets read");
    assert!(line.contains("includeObject=Metadata"), "{line}");
    for words in [
        format!("get secrets {KUBE} --json"),
        format!("get secrets -n shop {KUBE}"),
        format!("check {KUBE}"),
        format!("check {KUBE} --json"),
        format!("get pods {KUBE} -o wide"),
    ] {
        let result = run(&fake, &words).await;
        assert!(
            !result.out.contains(SECRET_VALUE),
            "{words}: {}",
            result.out
        );
        assert!(
            !result.err.contains(SECRET_VALUE),
            "{words}: {}",
            result.err
        );
    }
}

#[tokio::test]
async fn asking_for_a_secret_value_is_a_usage_error() {
    for words in [
        "get secrets --show-values",
        "get secrets --reveal",
        "get secrets db --decode",
        "get secrets db --key password",
        "get secrets -o yaml",
        "get secrets -o json",
        "get secrets/db/password",
        "get secrets db/password",
        "get pods --reveal",
    ] {
        let fake = Fake::answering(healthy());
        let result = run(&fake, &format!("{words} {KUBE}")).await;
        assert_eq!(result.code, 2, "{words}");
        assert!(result.out.is_empty(), "{words}");
        assert!(
            fake.recorder.requests().is_empty(),
            "{words}: nothing is read"
        );
        if words.contains("secrets") {
            assert!(
                result.err.contains("never requested"),
                "{words}: {}",
                result.err
            );
        }
    }
}

#[tokio::test]
async fn no_choice_is_a_usage_error_that_lists_the_entries_and_reads_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let setup = setup_file(&directory);
    let fake = Fake::answering(healthy());
    let result = run_in(&fake, Some(setup.clone()), "check").await;
    assert_eq!(result.code, 2);
    assert!(result.err.contains("core-fra"), "{}", result.err);
    assert!(result.err.contains("prod-ams"), "{}", result.err);
    assert!(result.err.contains("never used"), "{}", result.err);
    assert!(
        fake.opened.lock().unwrap().is_empty(),
        "no cluster was opened"
    );
    assert!(fake.recorder.requests().is_empty());

    // An unknown entry lists them too, and still opens nothing.
    let result = run_in(&fake, Some(setup), "get pods --cluster nowhere").await;
    assert_eq!(result.code, 2);
    assert!(
        result.err.contains(r#"no cluster entry "nowhere""#),
        "{}",
        result.err
    );
    assert!(fake.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_entry_opens_exactly_its_own_context() {
    let directory = tempfile::tempdir().unwrap();
    let setup = setup_file(&directory);
    let fake = Fake::answering(healthy());
    let result = run_in(
        &fake,
        Some(setup.clone()),
        "get nodes --cluster prod-ams --json",
    )
    .await;
    assert_eq!(result.code, 0, "{}", result.err);
    let opened = fake.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].entry.as_deref(), Some("prod-ams"));
    assert_eq!(opened[0].context, "prod-ams");
    assert_eq!(result.json()["cluster"]["entry"], "prod-ams");
    // `--setup` names the file for CI.
    let other = Fake::answering(healthy());
    let result = run(
        &other,
        &format!("get nodes --cluster dev-fra --setup {}", setup.display()),
    )
    .await;
    assert_eq!(result.code, 0, "{}", result.err);
}

#[test]
fn half_a_kubeconfig_pair_and_mixed_choices_are_refused_by_the_parser() {
    assert!(parse(&["check", "--kubeconfig", "/k"]).is_err());
    assert!(parse(&["check", "--kube-context", "c"]).is_err());
    assert!(
        parse(&[
            "check",
            "--cluster",
            "a",
            "--kube-context",
            "c",
            "--kubeconfig",
            "/k"
        ])
        .is_err()
    );
    assert!(parse(&["check", "--setup", "/s"]).is_err());
    assert!(parse(&["get"]).is_err());
}

const KUBECONFIG: &str = "apiVersion: v1
kind: Config
current-context: elsewhere
clusters:
- name: dead
  cluster:
    server: https://127.0.0.1:1
contexts:
- name: elsewhere
  context: {cluster: dead, user: token-user}
- name: chosen
  context: {cluster: dead, user: token-user}
- name: signs-in
  context: {cluster: dead, user: plugin-user}
users:
- name: token-user
  user: {token: TOKEN}
- name: plugin-user
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      command: /nonexistent/login-plugin
      interactiveMode: IfAvailable
";

fn kubeconfig(directory: &tempfile::TempDir) -> std::path::PathBuf {
    let path = directory.path().join("config");
    std::fs::write(&path, KUBECONFIG.replace("TOKEN", TOKEN)).unwrap();
    path
}

async fn real(words: &str) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let env = Env {
        default_setup: None,
        connector: &Kubeconfig,
    };
    let code = execute(&command(words), &env, &mut out, &mut err).await;
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

#[test]
fn the_kubeconfig_connector_opens_the_named_context_never_the_current_one() {
    let directory = tempfile::tempdir().unwrap();
    let file = kubeconfig(&directory);
    let request = Request {
        entry: None,
        sources: vec![file],
        context: "chosen".into(),
    };
    let target = Kubeconfig.open(&request, false).expect("opened");
    assert_eq!(target.context, "chosen");
    assert_eq!(target.source.context, "chosen");
    match &target.source.access {
        ClusterAccess::Direct(direct) => assert_eq!(direct.context(), "chosen"),
        other => panic!("unexpected access {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_context_is_a_usage_error_and_not_the_current_one() {
    let directory = tempfile::tempdir().unwrap();
    let file = kubeconfig(&directory);
    let result = real(&format!(
        "get nodes --kubeconfig {} --kube-context missing",
        file.display()
    ))
    .await;
    assert_eq!(result.code, 2);
    assert!(
        result.err.contains(r#"no context "missing""#),
        "{}",
        result.err
    );
    assert!(
        result.err.contains("elsewhere"),
        "the contexts are listed: {}",
        result.err
    );
    assert!(result.out.is_empty());
}

#[tokio::test]
async fn a_credential_plugin_needs_allow_login_and_exits_5_without_it() {
    let directory = tempfile::tempdir().unwrap();
    let file = kubeconfig(&directory);
    let words = format!(
        "get nodes --kubeconfig {} --kube-context signs-in",
        file.display()
    );
    let result = real(&words).await;
    assert_eq!(result.code, 5);
    assert!(result.err.contains("--allow-login"), "{}", result.err);
    assert!(result.out.is_empty());
}

#[tokio::test]
async fn a_dead_server_is_unknown_and_no_credential_reaches_the_output() {
    let directory = tempfile::tempdir().unwrap();
    let file = kubeconfig(&directory);
    for json in ["", " --json"] {
        let words = format!(
            "check --kubeconfig {} --kube-context chosen{json}",
            file.display()
        );
        let result = real(&words).await;
        assert_eq!(result.code, 3, "{}{}", result.out, result.err);
        for text in [&result.out, &result.err] {
            assert!(!text.contains(TOKEN), "{text}");
            assert!(!text.contains("apiVersion: v1"), "{text}");
        }
    }
}
