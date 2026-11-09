//! `changes`, `waiting-to-ship` and `get application` against a fake API that
//! serves an invented Kargo project, and a fake GitHub. Every request to the
//! cluster is a GET; no value or token reaches the output.

use chrono::{Duration, Utc};
use freshkube_delivery::github::Handle;
use freshkube_delivery::testing::github::{Reply, Server};

use super::*;

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn list(items: Vec<Value>) -> (u16, String) {
    answer(
        200,
        json!({"kind": "List", "apiVersion": "v1", "metadata": {"resourceVersion": "1"}, "items": items}),
    )
}

fn freight(name: &str, alias: &str, sha: &str, age: Duration) -> Value {
    json!({
        "metadata": {
            "namespace": "storefront", "name": name,
            "creationTimestamp": (Utc::now() - age).to_rfc3339(),
            "labels": {"kargo.akuity.io/alias": alias},
        },
        "origin": {"name": "main"},
        "commits": [{
            "repoURL": "https://github.com/acme/storefront",
            "id": sha, "message": "Fix the totals",
        }],
        // A server that over-shares: nothing here may reach the output.
        "data": {"password": SECRET_VALUE},
    })
}

/// dev runs `f-new` (verified three days ago); prod, fed by dev, runs `f-old`.
fn stages() -> Vec<Value> {
    let started = (Utc::now() - Duration::days(3)).to_rfc3339();
    vec![
        json!({
            "metadata": {"namespace": "storefront", "name": "dev"},
            "spec": {"requestedFreight": [{"origin": {"name": "main"}, "sources": {"direct": true}}]},
            "status": {"freightHistory": [{
                "items": {"Warehouse/main": {"name": "f-new"}},
                "verificationHistory": [{"id": "v1", "phase": "Successful", "startTime": started}],
            }]},
        }),
        json!({
            "metadata": {"namespace": "storefront", "name": "prod"},
            "spec": {"requestedFreight": [{"origin": {"name": "main"}, "sources": {"stages": ["dev"]}}]},
            "status": {"freightHistory": [{"items": {"Warehouse/main": {"name": "f-old"}}}]},
        }),
    ]
}

fn kargo(freights: Vec<Value>, stages: Vec<Value>) -> HashMap<String, (u16, String)> {
    let base = "/apis/kargo.akuity.io/v1alpha1";
    HashMap::from([
        (
            "/apis".into(),
            answer(
                200,
                json!({"kind": "APIGroupList", "groups": [{
                    "name": "kargo.akuity.io",
                    "versions": [{"groupVersion": "kargo.akuity.io/v1alpha1", "version": "v1alpha1"}],
                    "preferredVersion": {"version": "v1alpha1"},
                }]}),
            ),
        ),
        (
            base.into(),
            answer(
                200,
                json!({"kind": "APIResourceList", "groupVersion": "kargo.akuity.io/v1alpha1",
                       "resources": [{"name": "projects"}, {"name": "freights"}, {"name": "stages"}]}),
            ),
        ),
        (
            format!("{base}/projects"),
            list(vec![json!({"metadata": {"name": "storefront"}})]),
        ),
        (
            format!("{base}/namespaces/storefront/freights"),
            list(freights),
        ),
        (format!("{base}/namespaces/storefront/stages"), list(stages)),
    ])
}

fn world() -> HashMap<String, (u16, String)> {
    kargo(
        vec![
            freight("f-new", "wonky-otter", SHA, Duration::days(4)),
            freight("f-old", "quiet-heron", OLD_SHA, Duration::days(9)),
        ],
        stages(),
    )
}

fn pulls(path: &str) -> Value {
    let sha = path.split('/').nth(5).unwrap_or_default().to_owned();
    json!([{
        "number": 418, "title": "Fix the totals", "state": "closed",
        "merged_at": (Utc::now() - Duration::days(4)).to_rfc3339(), "merge_commit_sha": sha,
        "head": {"sha": "c".repeat(40)}, "base": {"ref": "main"},
        "user": {"login": "ken"},
    }])
}

/// A fake GitHub that names one merged pull request for any commit.
fn github() -> (Server, Handle) {
    let server = Server::new(|request| Reply::json(200, "", &pulls(&request.path)));
    let handle = Handle::anonymous_at(&server.address);
    (server, handle)
}

fn with_github(fake: &Arc<Fake>, handle: Handle) {
    *fake.github.lock().unwrap() = Some(handle);
}

fn only_reads(fake: &Arc<Fake>) {
    assert!(!fake.recorder.requests().is_empty());
    fake.recorder.assert_all("the CLI only reads", |request| {
        request.method.as_str() == "GET"
            && !request.query.contains("watch")
            && !request.path.contains("/exec")
            && !request.path.contains("/portforward")
            && !request.path.contains("/proxy")
    });
}

fn nothing_secret(run: &Run) {
    for text in [&run.out, &run.err] {
        assert!(!text.contains(SECRET_VALUE), "{text}");
        assert!(!text.contains(TOKEN), "{text}");
    }
}

#[tokio::test]
async fn waiting_to_ship_lists_the_gap_with_its_pull_request_and_exits_0() {
    let (server, handle) = github();
    let fake = Fake::answering(world());
    with_github(&fake, handle);
    let result = run(&fake, &format!("waiting-to-ship {KUBE}")).await;
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(result.out.contains("storefront  waiting"), "{}", result.out);
    assert!(result.out.contains("wonky-otter"), "{}", result.out);
    assert!(result.out.contains("pull request #418"), "{}", result.out);
    assert!(result.out.contains("waited 3 d"), "{}", result.out);
    let json = run(&fake, &format!("waiting-to-ship {KUBE} --json")).await;
    assert_eq!(json.code, 0);
    let json = json.json();
    assert_eq!(json["version"], 1);
    assert_eq!(json["command"], "waiting-to-ship");
    assert_eq!(json["cluster"]["context"], "acme-prod");
    let app = &json["apps"][0];
    assert_eq!(app["name"], "storefront");
    assert_eq!(app["state"], "waiting");
    assert_eq!(app["furthest"], "prod");
    let item = &app["items"][0];
    assert_eq!(item["freight"], "f-new");
    assert_eq!(item["pull"]["read"], "found");
    assert_eq!(item["pull"]["number"], 418);
    assert_eq!(item["in_stage"], "dev");
    assert!(item["waited_seconds"].as_i64().unwrap() >= 3 * 86_400);
    assert_eq!(app["stages"][0]["name"], "dev");
    assert_eq!(app["stages"][1]["final"], true);
    // GitHub was asked only for the Freight in the gap.
    assert!(!server.seen().is_empty());
    only_reads(&fake);
}

#[tokio::test]
async fn older_than_turns_a_wait_into_exit_1() {
    let (_server, handle) = github();
    let fake = Fake::answering(world());
    with_github(&fake, handle);
    let nag = run(&fake, &format!("waiting-to-ship {KUBE} --older-than 2d")).await;
    assert_eq!(nag.code, 1, "{}", nag.out);
    assert!(nag.out.contains("waited 2 d 0 h or longer"), "{}", nag.out);
    let json = run(
        &fake,
        &format!("waiting-to-ship {KUBE} --older-than 2d --json"),
    )
    .await;
    assert_eq!(json.code, 1);
    assert_eq!(json.json()["overdue"][0]["freight"], "f-new");
    let calm = run(&fake, &format!("waiting-to-ship {KUBE} --older-than 5d")).await;
    assert_eq!(calm.code, 0, "{}", calm.out);
}

#[tokio::test]
async fn a_bad_span_is_a_usage_error_before_anything_is_read() {
    let fake = Fake::answering(world());
    for line in [
        format!("waiting-to-ship {KUBE} --older-than 2"),
        format!("waiting-to-ship {KUBE} --older-than soon --json"),
        format!("changes storefront {KUBE} --since 1y"),
    ] {
        let result = run(&fake, &line).await;
        assert_eq!(result.code, 2, "{line}");
        assert!(result.out.is_empty(), "{line}");
        assert!(
            result.err.contains("unit") || result.err.contains("span"),
            "{}",
            result.err
        );
    }
    assert!(fake.opened.lock().unwrap().is_empty());
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test]
async fn a_rate_limited_github_is_shown_and_never_a_failure() {
    let server = Server::new(|_| {
        Reply::json(
            403,
            "x-ratelimit-remaining: 0\r\nx-ratelimit-limit: 60\r\nx-ratelimit-reset: 4102444800\r\n",
            &json!({"message": "API rate limit exceeded"}),
        )
    });
    let fake = Fake::answering(world());
    with_github(&fake, Handle::anonymous_at(&server.address));
    let result = run(&fake, &format!("waiting-to-ship {KUBE} --json")).await;
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    let item = &result.json()["apps"][0]["items"][0];
    assert_eq!(item["pull"]["read"], "not-read");
    assert_eq!(item["freight"], "f-new");
    let text = run(&fake, &format!("waiting-to-ship {KUBE}")).await;
    assert!(text.out.contains("pull request not read"), "{}", text.out);
}

#[tokio::test]
async fn kargo_refused_exits_4_and_unreachable_exits_3() {
    let mut routes = world();
    routes.insert(
        "/apis/kargo.akuity.io/v1alpha1/projects".into(),
        status(403, "Forbidden"),
    );
    let refused = Fake::answering(routes);
    let result = run(&refused, &format!("waiting-to-ship {KUBE} --json")).await;
    assert_eq!(result.code, 4, "{}{}", result.out, result.err);
    assert_eq!(result.json()["notes"][0]["gap"], "refused");
    assert_eq!(result.json()["exit"], 4);
    let dead = Fake::unreachable();
    let result = run(&dead, &format!("waiting-to-ship {KUBE}")).await;
    assert_eq!(result.code, 3, "{}", result.out);
    assert!(result.out.starts_with("? "), "{}", result.out);
    let result = run(&dead, &format!("changes storefront {KUBE}")).await;
    assert_eq!(result.code, 3, "{}", result.out);
    let result = run(&dead, &format!("get application storefront {KUBE}")).await;
    assert_eq!(result.code, 3, "{}", result.out);
}

#[tokio::test]
async fn a_rejected_sign_in_exits_5() {
    let fake = Fake::failing(Failure::new(FailureKind::Unauthorized, "401 Unauthorized"));
    let result = run(&fake, &format!("waiting-to-ship {KUBE}")).await;
    assert_eq!(result.code, 5, "{}", result.out);
}

#[tokio::test]
async fn a_cluster_must_be_named_for_every_new_command() {
    let fake = Fake::answering(world());
    for line in [
        "changes storefront",
        "waiting-to-ship",
        "get application storefront",
    ] {
        let result = run(&fake, line).await;
        assert_eq!(result.code, 2, "{line}");
        assert!(result.out.is_empty());
        assert!(result.err.contains("the current context is never used"));
    }
    assert!(fake.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn get_application_shows_the_stages_and_asks_github_nothing() {
    let (server, handle) = github();
    let fake = Fake::answering(world());
    with_github(&fake, handle);
    let result = run(&fake, &format!("get application storefront {KUBE}")).await;
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(result.out.contains("dev"), "{}", result.out);
    assert!(result.out.contains("prod (last)"), "{}", result.out);
    assert!(result.out.contains("runs f-old"), "{}", result.out);
    let json = run(&fake, &format!("get application/storefront {KUBE} --json")).await;
    let json = json.json();
    assert_eq!(json["command"], "get");
    assert_eq!(json["application"]["name"], "storefront");
    assert_eq!(json["application"]["stages"][0]["name"], "dev");
    assert_eq!(json["application"]["stages"][0]["runs"][0], "f-new");
    assert_eq!(json["application"]["view"]["page"], "application");
    assert!(
        server.seen().is_empty(),
        "get application reads no pull request"
    );
    only_reads(&fake);
}

#[tokio::test]
async fn an_application_that_is_not_there_exits_1() {
    let fake = Fake::answering(world());
    let result = run(&fake, &format!("get application nope {KUBE} --json")).await;
    assert_eq!(result.code, 1, "{}", result.out);
    let json = result.json();
    assert_eq!(json["problems"][0]["state"], "not-found");
    assert!(json["application"].is_null());
    let result = run(&fake, &format!("changes nope {KUBE}")).await;
    assert_eq!(result.code, 1, "{}", result.out);
    assert!(
        result.out.contains("no Kargo project \"nope\""),
        "{}",
        result.out
    );
    assert!(result.out.contains("storefront"), "{}", result.out);
}

#[tokio::test]
async fn get_application_without_a_name_or_with_a_value_option_is_a_usage_error() {
    let fake = Fake::answering(world());
    for line in [
        format!("get application {KUBE}"),
        format!("get application storefront {KUBE} --show-values"),
        format!("get application storefront {KUBE} -o yaml"),
    ] {
        let result = run(&fake, &line).await;
        assert_eq!(result.code, 2, "{line}");
    }
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test]
async fn changes_lists_each_freight_with_its_stages_and_says_what_it_could_not_read() {
    let fake = Fake::answering(world());
    let result = run(&fake, &format!("changes storefront {KUBE} --json")).await;
    let json = result.json();
    // Argo CD isn't served here, so what joins a Stage to its pods is unknown:
    // told in the notes, and exit 3 rather than a guess.
    assert_eq!(result.code, 3, "{}{}", result.out, result.err);
    assert_eq!(json["version"], 1);
    assert_eq!(json["command"], "changes");
    assert_eq!(json["application"], "storefront");
    assert_eq!(json["stages"], json!(["dev", "prod"]));
    let keys: Vec<&str> = json["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["freight"].as_str().unwrap())
        .collect();
    assert!(
        keys.contains(&"f-new") && keys.contains(&"f-old"),
        "{keys:?}"
    );
    let new = json["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["freight"] == "f-new")
        .unwrap();
    assert_eq!(new["alias"], "wonky-otter");
    assert_eq!(new["change"]["kind"], "commit");
    assert_eq!(new["stages"][0]["name"], "dev");
    assert_eq!(new["stages"][1]["reach"], "not-reached");
    assert_eq!(new["view"]["page"], "change");
    assert!(!json["notes"].as_array().unwrap().is_empty());
    only_reads(&fake);
    nothing_secret(&result);
    let text = run(&fake, &format!("changes storefront {KUBE}")).await;
    assert!(
        text.out.contains("Changes of storefront on acme-prod"),
        "{}",
        text.out
    );
    assert!(text.out.contains("Stages: dev → prod"), "{}", text.out);
    assert!(text.out.contains("wonky-otter"), "{}", text.out);
    nothing_secret(&text);
}

#[tokio::test]
async fn stage_and_since_narrow_the_changes() {
    let fake = Fake::answering(world());
    let by_stage = run(
        &fake,
        &format!("changes storefront {KUBE} --stage prod --json"),
    )
    .await;
    let json = by_stage.json();
    let rows = json["changes"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["freight"], "f-old");
    assert_eq!(rows[0]["stages"].as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["stages"][0]["name"], "prod");
    let recent = run(
        &fake,
        &format!("changes storefront {KUBE} --since 6d --json"),
    )
    .await;
    let json = recent.json();
    let rows = json["changes"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["freight"], "f-new");
    assert_eq!(json["since"], 6 * 86_400);
    let unknown = run(&fake, &format!("changes storefront {KUBE} --stage staging")).await;
    assert_eq!(unknown.code, 1, "{}", unknown.out);
    assert!(unknown.out.contains("Stages: dev, prod"), "{}", unknown.out);
}

#[tokio::test]
async fn a_project_with_no_freight_has_nothing_to_report_and_exits_0() {
    let fake = Fake::answering(kargo(Vec::new(), stages()));
    let result = run(&fake, &format!("changes storefront {KUBE}")).await;
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(result.out.contains("(0 changes)"), "{}", result.out);
    only_reads(&fake);
}

#[tokio::test]
async fn a_setup_file_names_the_entry_and_the_workspace_is_read_through_it() {
    let directory = tempfile::tempdir().unwrap();
    let setup = setup_file(&directory);
    let (_server, handle) = github();
    let fake = Fake::answering(world());
    with_github(&fake, handle);
    let result = run_in(
        &fake,
        Some(setup),
        "waiting-to-ship --cluster prod-ams --json",
    )
    .await;
    let opened = fake.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].entry.as_deref(), Some("prod-ams"));
    assert!(
        opened[0].setup.is_some(),
        "the entry's setup travels with it"
    );
    assert_eq!(result.json()["cluster"]["entry"], "prod-ams");
    nothing_secret(&result);
}
