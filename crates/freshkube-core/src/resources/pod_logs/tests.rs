use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::resources::watch::tests::server;

const POD: &str = "/api/v1/namespaces/shop/pods/web-0";

fn pod(policy: &str, phase: &str, status: serde_json::Value) -> String {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop", "uid": "u-1"},
        "spec": {"restartPolicy": policy, "containers": [{"name": "app"}]},
        "status": {"phase": phase, "containerStatuses": [
            {"name": "app", "restartCount": status["restarts"], "state": status["state"],
             "lastState": status.get("last").cloned().unwrap_or(json!({}))}
        ]}
    })
    .to_string()
}

fn running(restarts: u32) -> serde_json::Value {
    json!({"restarts": restarts, "state": {"running": {"startedAt": "2026-10-01T12:00:00Z"}}})
}

fn exited(code: i64, reason: &str) -> serde_json::Value {
    json!({"terminated": {"exitCode": code, "reason": reason,
        "finishedAt": "2026-10-01T12:05:10Z"}})
}

fn request() -> LogRequest {
    LogRequest {
        namespace: "shop".into(),
        pod: "web-0".into(),
        container: "app".into(),
        previous: false,
        tail: Some(500),
        resume: None,
    }
}

/// Answers a GET of the pod from `pods` and a log request from `logs`, each
/// in turn. Once a list runs out, its last answer repeats.
fn scripted(
    pods: Vec<(u16, String)>,
    logs: Vec<(u16, String)>,
) -> (Client, Arc<Mutex<Vec<String>>>) {
    let turns = Arc::new(Mutex::new((0, 0)));
    server(move |uri| {
        let mut turns = turns.lock().unwrap();
        let (list, turn) = if uri.contains("/log?") {
            (&logs, &mut turns.1)
        } else {
            assert!(uri.starts_with(POD), "{uri}");
            (&pods, &mut turns.0)
        };
        let answer = list[(*turn).min(list.len() - 1)].clone();
        *turn += 1;
        answer
    })
}

async fn follow(client: Client, request: LogRequest) -> Vec<PodLogUpdate> {
    let (sender, mut receiver) = mpsc::channel(8);
    let task = tokio::spawn(follow_pod_log(client, request, sender));
    let mut updates = Vec::new();
    while let Some(update) = receiver.recv().await {
        updates.push(update);
    }
    task.await.unwrap();
    updates
}

/// Updates as short strings, lines by their text after the timestamp.
fn summary(updates: &[PodLogUpdate]) -> Vec<String> {
    updates
        .iter()
        .map(|update| match update {
            PodLogUpdate::Waiting(reason) => format!("waiting {reason}"),
            PodLogUpdate::Streaming => "streaming".into(),
            PodLogUpdate::Line(line) => line.split_once(' ').map_or(line.as_str(), |l| l.1).into(),
            PodLogUpdate::Reconnecting {
                attempt,
                delay,
                failure,
            } => {
                format!(
                    "reconnecting {attempt} in {}s: {:?}",
                    delay.as_secs(),
                    failure.kind
                )
            }
            PodLogUpdate::Restarting(ended) => {
                format!("restarting after {}", ended.as_ref().unwrap())
            }
            PodLogUpdate::Ended(Some(ended)) => format!("ended {ended}"),
            PodLogUpdate::Ended(None) => "ended".into(),
            PodLogUpdate::Failed(failure) => format!("failed {:?}", failure.kind),
        })
        .collect()
}

fn lines(lines: &[(&str, &str)]) -> (u16, String) {
    let body: String = lines
        .iter()
        .map(|(time, text)| format!("2026-10-01T12:{time}Z {text}\n"))
        .collect();
    (200, body)
}

fn status(code: u16, reason: &str) -> (u16, String) {
    let body = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": format!("{reason} for test"), "reason": reason, "code": code});
    (code, body.to_string())
}

#[test]
fn containers_list_init_app_and_ephemeral_with_their_state() {
    let object: Value = serde_yaml::from_str(
        r#"
metadata:
  annotations:
    kubectl.kubernetes.io/default-container: sidecar
spec:
  restartPolicy: Always
  initContainers:
  - name: migrate
  - name: proxy
    restartPolicy: Always
  containers:
  - name: web
  - name: sidecar
  ephemeralContainers:
  - name: debugger
status:
  phase: Running
  initContainerStatuses:
  - name: migrate
    restartCount: 0
    state: {terminated: {exitCode: 0, reason: Completed, finishedAt: '2026-10-01T11:00:00Z'}}
  - name: proxy
    restartCount: 0
    state: {running: {startedAt: '2026-10-01T11:00:05Z'}}
  containerStatuses:
  - name: web
    restartCount: 4
    state: {waiting: {reason: CrashLoopBackOff}}
    lastState: {terminated: {exitCode: 137, reason: OOMKilled, finishedAt: '2026-10-01T11:58:40Z'}}
  - name: sidecar
    restartCount: 0
    state: {running: {}}
"#,
    )
    .unwrap();
    let pod = pod_containers(&object);
    let names: Vec<_> = pod
        .containers
        .iter()
        .map(|container| (container.name.as_str(), container.role))
        .collect();
    assert_eq!(
        names,
        [
            ("migrate", ContainerRole::Init),
            ("proxy", ContainerRole::Init),
            ("web", ContainerRole::App),
            ("sidecar", ContainerRole::App),
            ("debugger", ContainerRole::Ephemeral),
        ]
    );
    assert_eq!(pod.default.as_deref(), Some("sidecar"));

    let web = pod.get("web").unwrap();
    assert_eq!(web.restarts, 4);
    assert_eq!(
        web.state,
        ContainerState::Waiting("CrashLoopBackOff".into())
    );
    let last = web.last_termination.as_ref().unwrap();
    assert_eq!(last.to_string(), "exit 137 (OOMKilled)");
    assert!(web.started() && web.has_previous());

    let migrate = pod.get("migrate").unwrap();
    let ContainerState::Terminated(done) = &migrate.state else {
        panic!("{migrate:?}");
    };
    // An init container runs again only after a failure; a sidecar always.
    assert!(!migrate.restarts_after(done));
    assert!(migrate.restarts_after(&Termination {
        exit_code: 1,
        ..done.clone()
    }));
    assert!(pod.get("proxy").unwrap().restarts_after(done));
    // No status yet: it hasn't started, and nothing ran before.
    let debugger = pod.get("debugger").unwrap();
    assert_eq!(debugger.state, ContainerState::Waiting(String::new()));
    assert!(!debugger.started() && !debugger.has_previous());
    assert!(!debugger.restarts_after(done));
}

#[test]
fn the_default_is_the_first_app_container_and_finished_pods_restart_nothing() {
    let object: Value = serde_json::from_str(&pod("OnFailure", "Succeeded", running(0))).unwrap();
    let pod = pod_containers(&object);
    assert_eq!(pod.default.as_deref(), Some("app"));
    let app = pod.get("app").unwrap();
    let failed = Termination {
        exit_code: 1,
        reason: "Error".into(),
        finished: None,
    };
    assert!(!app.restarts_after(&failed));
    // An annotation naming no container is ignored.
    let mut object = object;
    object["metadata"]["annotations"] =
        serde_yaml::from_str("{kubectl.kubernetes.io/default-container: gone}").unwrap();
    assert_eq!(pod_containers(&object).default.as_deref(), Some("app"));
}

#[tokio::test]
async fn lines_are_cut_stripped_and_read_whatever_their_bytes() {
    let mut body = b"2026-10-01T12:00:00Z one\r\n".to_vec();
    body.extend(b"2026-10-01T12:00:00Z ");
    body.extend(vec![b'x'; MAX_LINE_BYTES * 2]);
    body.extend(b"\n2026-10-01T12:00:01Z bad \xff byte\nno newline at the end");
    let (sender, mut receiver) = mpsc::channel(8);
    let mut position = LogPosition::default();
    let reader = futures::io::Cursor::new(body);
    let read = tokio::spawn(async move {
        pump(reader, &mut position, &sender)
            .await
            .map(|r| (r.lines, position))
    });
    let mut lines = Vec::new();
    while let Some(PodLogUpdate::Line(line)) = receiver.recv().await {
        lines.push(line);
    }
    let (count, position) = read.await.unwrap().unwrap();
    assert_eq!(count, 4);
    assert_eq!(lines[0], "2026-10-01T12:00:00Z one");
    assert_eq!(lines[1].len(), MAX_LINE_BYTES);
    assert_eq!(lines[2], "2026-10-01T12:00:01Z bad \u{fffd} byte");
    assert_eq!(lines[3], "no newline at the end");
    // The last timestamped line sets the position; the untimed one doesn't.
    let mut expected = LogPosition::default();
    expected.record("2026-10-01T12:00:01Z bad");
    assert_eq!(position, expected);
}

#[tokio::test]
async fn resuming_skips_what_was_read_even_within_one_second() {
    let mut position = LogPosition::default();
    for line in [
        "2026-10-01T12:00:00.5Z a",
        "2026-10-01T12:00:01.25Z b",
        "2026-10-01T12:00:01.25Z c",
    ] {
        position.record(line);
    }
    // Kubernetes resumes from the whole second, so b and c come again.
    let body = "2026-10-01T12:00:01.1Z early\n2026-10-01T12:00:01.25Z b\n2026-10-01T12:00:01.25Z c\n2026-10-01T12:00:01.25Z d\n2026-10-01T12:00:02Z e\n";
    let (sender, mut receiver) = mpsc::channel(8);
    let reader = futures::io::Cursor::new(body.as_bytes().to_vec());
    tokio::spawn(async move { pump(reader, &mut position, &sender).await.map(|_| ()) });
    let mut read = Vec::new();
    while let Some(PodLogUpdate::Line(line)) = receiver.recv().await {
        read.push(line.split_once(' ').unwrap().1.to_owned());
    }
    assert_eq!(read, ["d", "e"]);
}

#[tokio::test(start_paused = true)]
async fn a_container_that_exits_for_good_ends_its_log() {
    let ended = json!({"restarts": 0, "state": exited(0, "Completed")});
    let (client, seen) = scripted(
        vec![(200, pod("Never", "Succeeded", ended))],
        vec![lines(&[("00:00", "starting"), ("05:09", "done")])],
    );
    let updates = follow(client, request()).await;
    assert_eq!(
        summary(&updates),
        ["streaming", "starting", "done", "ended exit 0 (Completed)"]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], POD);
    assert_eq!(
        seen[1],
        format!("{POD}/log?&container=app&follow=true&tailLines=500&timestamps=true")
    );
    assert_eq!(seen.len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_log_resumes_after_its_last_line_until_refused() {
    let (client, seen) = scripted(
        vec![(200, pod("Always", "Running", running(0)))],
        vec![
            lines(&[("00:00.10", "a"), ("00:00.20", "b")]),
            // From the second again: a and b come back, then c.
            lines(&[("00:00.10", "a"), ("00:00.20", "b"), ("00:00.30", "c")]),
            status(403, "Forbidden"),
        ],
    );
    let updates = follow(client, request()).await;
    assert_eq!(
        summary(&updates),
        [
            "streaming",
            "a",
            "b",
            "reconnecting 1 in 1s: Unreachable",
            "streaming",
            "c",
            "reconnecting 1 in 1s: Unreachable",
            "failed Forbidden",
        ]
    );
    let seen = seen.lock().unwrap();
    let logs: Vec<_> = seen.iter().filter(|uri| uri.contains("/log?")).collect();
    assert!(logs[0].contains("tailLines=500"));
    // A resume asks from the last line's second, not for the tail again.
    assert!(
        logs[1].contains("sinceTime=2026-10-01T12%3A00%3A00Z"),
        "{}",
        logs[1]
    );
    assert!(!logs[1].contains("tailLines"));
}

#[tokio::test(start_paused = true)]
async fn a_crash_looping_container_marks_each_restart_and_follows_the_next() {
    let crashed = |restarts: u32, state: serde_json::Value| json!({"restarts": restarts, "state": state, "last": exited(137, "OOMKilled")});
    let backoff = json!({"waiting": {"reason": "CrashLoopBackOff"}});
    let (client, _) = scripted(
        vec![
            (200, pod("Always", "Running", crashed(3, backoff.clone()))),
            // The log closed: the instance it showed had ended.
            (200, pod("Always", "Running", crashed(3, backoff.clone()))),
            (200, pod("Always", "Running", crashed(3, backoff))),
            (
                200,
                pod(
                    "Always",
                    "Running",
                    crashed(4, json!({"running": {"startedAt": "2026-10-01T12:06:00Z"}})),
                ),
            ),
            (
                200,
                pod(
                    "Always",
                    "Running",
                    crashed(4, json!({"running": {"startedAt": "2026-10-01T12:06:00Z"}})),
                ),
            ),
            status(404, "NotFound"),
        ],
        vec![
            lines(&[("05:00", "old instance")]),
            lines(&[("06:01", "new instance")]),
        ],
    );
    let updates = follow(client, request()).await;
    assert_eq!(
        summary(&updates),
        [
            "streaming",
            "old instance",
            "restarting after exit 137 (OOMKilled)",
            "waiting CrashLoopBackOff",
            "streaming",
            "new instance",
            "failed NotFound",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_container_that_has_not_started_is_waited_for() {
    let creating = json!({"restarts": 0, "state": {"waiting": {"reason": "ContainerCreating"}}});
    let (client, _) = scripted(
        vec![
            (200, pod("Never", "Pending", creating.clone())),
            (200, pod("Never", "Pending", creating)),
            (200, pod("Never", "Running", running(0))),
            (200, pod("Never", "Running", running(0))),
            (
                200,
                pod(
                    "Never",
                    "Failed",
                    json!({"restarts": 0, "state": exited(1, "Error")}),
                ),
            ),
        ],
        vec![lines(&[("00:01", "boom")])],
    );
    let updates = follow(client, request()).await;
    assert_eq!(
        summary(&updates),
        [
            "waiting ContainerCreating",
            "streaming",
            "boom",
            "ended exit 1 (Error)"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_previous_instance_is_read_to_its_end_or_says_why_not() {
    let (client, seen) = scripted(vec![], vec![lines(&[("05:00", "last words")])]);
    let previous = LogRequest {
        previous: true,
        tail: Some(100),
        ..request()
    };
    let updates = follow(client, previous.clone()).await;
    assert_eq!(summary(&updates), ["streaming", "last words", "ended"]);
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [format!(
            "{POD}/log?&container=app&previous=true&tailLines=100&timestamps=true"
        )]
    );

    let (client, _) = scripted(vec![], vec![status(400, "BadRequest")]);
    let updates = follow(client, previous).await;
    assert_eq!(summary(&updates), ["failed Other"]);
}

#[tokio::test(start_paused = true)]
async fn repeated_failures_give_up_after_the_last_attempt() {
    let (client, seen) = scripted(vec![status(500, "InternalError")], vec![]);
    let updates = follow(client, request()).await;
    assert_eq!(
        summary(&updates),
        [
            "reconnecting 1 in 1s: Other",
            "reconnecting 2 in 2s: Other",
            "reconnecting 3 in 4s: Other",
            "reconnecting 4 in 8s: Other",
            "failed Other",
        ]
    );
    assert_eq!(seen.lock().unwrap().len(), MAX_ATTEMPTS as usize);
}
