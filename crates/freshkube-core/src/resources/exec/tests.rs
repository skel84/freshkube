use serde_json::json;
use tokio::io::duplex;

use super::*;
use crate::resources::watch::tests::server;

const POD: &str = "/api/v1/namespaces/shop/pods/web-0";

fn request() -> ExecRequest {
    ExecRequest {
        namespace: "shop".into(),
        pod: "web-0".into(),
        uid: "u-1".into(),
        container: "app".into(),
        size: ExecSize {
            columns: 120,
            rows: 40,
        },
    }
}

fn pod(uid: &str, restarts: u32, state: serde_json::Value) -> String {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop", "uid": uid},
        "spec": {"containers": [{"name": "app"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "restartCount": restarts, "state": state}]}
    })
    .to_string()
}

fn running() -> serde_json::Value {
    json!({"running": {"startedAt": "2026-10-02T12:00:00Z"}})
}

fn missing() -> (u16, String) {
    let status = json!({"kind": "Status", "apiVersion": "v1", "metadata": {},
        "status": "Failure", "message": "pods \"web-0\" not found", "reason": "NotFound",
        "code": 404});
    (404, status.to_string())
}

fn status(value: serde_json::Value) -> Status {
    serde_json::from_value(value).unwrap()
}

fn exit_status(code: &str) -> Status {
    status(json!({"metadata": {}, "status": "Failure",
        "message": format!("command terminated with non-zero exit code: exit status {code}"),
        "reason": "NonZeroExitCode",
        "details": {"causes": [{"reason": "ExitCode", "message": code}]}}))
}

#[test]
fn neighbouring_input_is_joined_and_only_the_last_resize_kept() {
    let size = |columns| ExecSize { columns, rows: 24 };
    let items = vec![
        ExecInput::Bytes(b"l".to_vec()),
        ExecInput::Bytes(b"s".to_vec()),
        ExecInput::Resize(size(80)),
        ExecInput::Resize(size(90)),
        ExecInput::Bytes(b"\r".to_vec()),
        ExecInput::Resize(size(100)),
    ];
    assert_eq!(
        coalesce(items),
        vec![
            ExecInput::Bytes(b"ls".to_vec()),
            ExecInput::Resize(size(90)),
            ExecInput::Bytes(b"\r".to_vec()),
            ExecInput::Resize(size(100)),
        ],
        "bytes typed before a resize stay before it"
    );
    assert_eq!(coalesce(Vec::new()), Vec::new());
}

#[test]
fn an_exit_status_gives_the_code() {
    let success = status(json!({"metadata": {}, "status": "Success"}));
    assert_eq!(classify_status(&success, true), ExecEnd::Exited(0));
    assert_eq!(classify_status(&exit_status("3"), true), ExecEnd::Exited(3));
    // A shell that ran and whose last command wasn't found still exited.
    assert_eq!(
        classify_status(&exit_status("127"), true),
        ExecEnd::Exited(127)
    );
}

#[test]
fn a_container_without_a_shell_is_named() {
    let ExecEnd::Failed(failure) = classify_status(&exit_status("127"), false) else {
        panic!("127 before any output means no shell");
    };
    assert_eq!(failure.kind, ExecFailureKind::NoShell);
    assert!(failure.is_permanent());
    let runtime = status(json!({"metadata": {}, "status": "Failure",
        "message": "OCI runtime exec failed: exec failed: unable to start container process: \
            exec: \"sh\": executable file not found in $PATH: unknown",
        "reason": "InternalError"}));
    let ExecEnd::Failed(failure) = classify_status(&runtime, false) else {
        panic!("the runtime's message means no shell");
    };
    assert_eq!(failure.kind, ExecFailureKind::NoShell);
    assert!(failure.to_string().starts_with("No shell · OCI runtime"));
    let other = status(json!({"metadata": {}, "status": "Failure"}));
    assert_eq!(
        classify_status(&other, true),
        ExecEnd::Failed(ExecFailure::new(
            ExecFailureKind::Request(FailureKind::Other),
            "The shell failed"
        ))
    );
}

#[test]
fn a_refused_exec_keeps_its_category() {
    let refused = |code: u16| {
        ExecFailure::from_kube(kube::Error::UpgradeConnection(
            UpgradeConnectionError::ProtocolSwitch(http::StatusCode::from_u16(code).unwrap()),
        ))
    };
    let forbidden = refused(403);
    assert_eq!(
        forbidden.kind,
        ExecFailureKind::Request(FailureKind::Forbidden)
    );
    assert!(forbidden.is_permanent());
    assert_eq!(
        forbidden.to_string(),
        "Forbidden · Not allowed to run a shell in this pod"
    );
    assert_eq!(
        refused(404).kind,
        ExecFailureKind::Request(FailureKind::NotFound)
    );
    assert_eq!(
        refused(401).kind,
        ExecFailureKind::Request(FailureKind::Unauthorized)
    );
    let bad = refused(400);
    assert_eq!(bad.kind, ExecFailureKind::Request(FailureKind::Other));
    assert_eq!(
        bad.message,
        "The API server refused the shell (400 Bad Request)"
    );
    assert!(!bad.is_permanent());
    assert!(!ExecFailure::new(ExecFailureKind::NotRunning, "x").is_permanent());
}

#[tokio::test]
async fn a_pod_that_cant_take_a_shell_fails_before_exec() {
    let cases = [
        (
            missing(),
            ExecFailureKind::Request(FailureKind::NotFound),
            "The pod was deleted",
        ),
        (
            (200, pod("u-2", 0, running())),
            ExecFailureKind::Request(FailureKind::NotFound),
            "The pod was replaced",
        ),
        (
            (
                200,
                pod("u-1", 0, json!({"waiting": {"reason": "CrashLoopBackOff"}})),
            ),
            ExecFailureKind::NotRunning,
            "CrashLoopBackOff",
        ),
        (
            (
                200,
                pod(
                    "u-1",
                    0,
                    json!({"terminated": {"exitCode": 0, "reason": "Completed"}}),
                ),
            ),
            ExecFailureKind::NotRunning,
            "It exited (exit 0 (Completed))",
        ),
    ];
    for ((code, body), kind, message) in cases {
        let (client, seen) = server(move |_| (code, body.clone()));
        let Err(failure) = start_exec(client, request()).await else {
            panic!("{message}: no shell may start");
        };
        assert_eq!(failure, ExecFailure::new(kind, message));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![POD.to_owned()],
            "{message}: only the pod is read"
        );
    }
    let mut other = request();
    other.container = "sidecar".into();
    let (client, _) = server(|_| (200, pod("u-1", 0, running())));
    let failure = start_exec(client, other).await.err().unwrap();
    assert_eq!(failure.message, "The pod has no container named sidecar");
}

#[tokio::test]
async fn exec_asks_for_a_tty_shell_in_the_chosen_container() {
    let (client, seen) = server(|uri| {
        if uri.starts_with(&format!("{POD}/exec")) {
            (403, String::new())
        } else {
            (200, pod("u-1", 2, running()))
        }
    });
    let failure = start_exec(client, request()).await.err().unwrap();
    assert_eq!(
        failure.kind,
        ExecFailureKind::Request(FailureKind::Forbidden)
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    let uri = &seen[1];
    assert!(uri.starts_with(&format!("{POD}/exec?")), "{uri}");
    for part in [
        "container=app",
        "stdin=true",
        "stdout=true",
        "tty=true",
        "command=sh",
        "command=-c",
    ] {
        assert!(uri.contains(part), "{part} in {uri}");
    }
    assert!(!uri.contains("stderr=true"), "a TTY merges stderr: {uri}");
}

#[tokio::test]
async fn an_exec_closed_without_a_status_asks_the_pod_why() {
    let cases = [
        (missing(), ExecEnd::PodGone("The pod was deleted".into())),
        (
            (200, pod("u-2", 0, running())),
            ExecEnd::PodGone("The pod was replaced".into()),
        ),
        (
            (200, pod("u-1", 3, running())),
            ExecEnd::PodGone("The container restarted".into()),
        ),
        (
            (
                200,
                pod(
                    "u-1",
                    2,
                    json!({"terminated": {"exitCode": 137, "reason": "OOMKilled"}}),
                ),
            ),
            ExecEnd::PodGone("The container stopped".into()),
        ),
        (
            (200, pod("u-1", 2, running())),
            ExecEnd::Disconnected(Failure::new(FailureKind::Unreachable, "reset by peer")),
        ),
    ];
    for ((code, body), end) in cases {
        let (client, _) = server(move |_| (code, body.clone()));
        let got = ended_without_status(&client, &request(), 2, Some("reset by peer".into())).await;
        assert_eq!(got, end);
    }
    let (client, _) = server(|_| (200, pod("u-1", 2, running())));
    assert_eq!(
        ended_without_status(&client, &request(), 2, None).await,
        ExecEnd::Disconnected(Failure::new(
            FailureKind::Unreachable,
            "The connection closed"
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn output_goes_out_in_batches_of_eight_milliseconds_or_64_kib() {
    let (mut shell, stdout) = duplex(4 * BATCH_BYTES);
    let (sink, mut output) = mpsc::channel(16);
    let reading = tokio::spawn(async move { read_output(stdout, &sink).await });
    shell.write_all(b"one ").await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    shell.write_all(b"two").await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    shell.write_all(b"three").await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    shell
        .write_all(&vec![b'x'; BATCH_BYTES + 10])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(shell);
    assert_eq!(reading.await.unwrap(), Some(true));
    let mut batches = Vec::new();
    while let Ok(ExecOutput::Bytes(bytes)) = output.try_recv() {
        batches.push(bytes);
    }
    assert_eq!(batches[0], b"one two", "within 8 ms: one batch");
    assert_eq!(batches[1], b"three");
    assert_eq!(batches[2].len(), BATCH_BYTES, "a batch stops at 64 KiB");
    assert_eq!(batches[3].len(), 10);
    assert_eq!(batches.len(), 4);
}

#[tokio::test(start_paused = true)]
async fn output_ends_with_the_stream_and_stops_when_nobody_reads() {
    let (shell, stdout) = duplex(64);
    drop(shell);
    let (sink, _output) = mpsc::channel(1);
    assert_eq!(read_output(stdout, &sink).await, Some(false), "no output");
    let (mut shell, stdout) = duplex(64);
    let (sink, output) = mpsc::channel(1);
    drop(output);
    shell.write_all(b"prompt$ ").await.unwrap();
    assert_eq!(read_output(stdout, &sink).await, None);
}

#[tokio::test]
async fn input_is_written_in_order_until_the_user_closes_it() {
    let (mut stdin, mut shell) = duplex(1024);
    let (sizes, mut resized) = size_channel::channel(10);
    let (input, queue) = mpsc::channel(16);
    let size = |columns| ExecSize { columns, rows: 30 };
    for item in [
        ExecInput::Bytes(b"echo ".to_vec()),
        ExecInput::Bytes(b"hi\r".to_vec()),
        ExecInput::Resize(size(100)),
        ExecInput::Resize(size(132)),
    ] {
        input.send(item).await.unwrap();
    }
    drop(input);
    assert!(
        write_input(queue, &mut stdin, sizes).await,
        "the user closed it"
    );
    drop(stdin);
    let mut typed = Vec::new();
    shell.read_to_end(&mut typed).await.unwrap();
    assert_eq!(typed, b"echo hi\r");
    let sent = resized.try_recv().unwrap();
    assert_eq!((sent.width, sent.height), (132, 30));
    assert!(resized.try_recv().is_err(), "one resize only");
}

/// Streams over in-memory pipes: what the session writes to the shell,
/// what the shell writes, and the handles that end it.
struct Pipes {
    streams: Streams<tokio::io::DuplexStream, tokio::io::DuplexStream>,
    typed: tokio::io::DuplexStream,
    shell: tokio::io::DuplexStream,
    input: mpsc::Sender<ExecInput>,
    output: mpsc::Receiver<ExecOutput>,
    close: oneshot::Sender<()>,
}

fn pipes() -> Pipes {
    let (stdin, typed) = duplex(64);
    let (shell, stdout) = duplex(64);
    let (sizes, _) = size_channel::channel(10);
    let (input, queue) = mpsc::channel(4);
    let (sink, output) = mpsc::channel(4);
    let (close, closing) = oneshot::channel();
    Pipes {
        streams: Streams {
            input: queue,
            stdin,
            sizes,
            stdout,
            output: sink,
            close: closing,
        },
        typed,
        shell,
        input,
        output,
        close,
    }
}

/// Everything the session typed into the shell so far.
async fn typed(pipe: &mut tokio::io::DuplexStream) -> Vec<u8> {
    let mut typed = vec![0; 64];
    let read = pipe.read(&mut typed).await.unwrap();
    typed.truncate(read);
    typed
}

#[tokio::test(start_paused = true)]
async fn ending_a_session_interrupts_then_ends_the_shell() {
    let mut pipes = pipes();
    pipes.shell.write_all(b"$ ").await.unwrap();
    drop(pipes.input);
    let started = tokio::time::Instant::now();
    // The server never closes; the session still ends, at the deadline.
    let ran = pipes.streams.run().await.unwrap();
    assert!(ran.closed);
    assert!(ran.output_seen);
    assert_eq!(started.elapsed(), INTERRUPT_PAUSE + CLOSE_DEADLINE);
    assert_eq!(typed(&mut pipes.typed).await, b"\x03\x04");
    assert_eq!(
        pipes.output.recv().await,
        Some(ExecOutput::Bytes(b"$ ".to_vec()))
    );
}

#[tokio::test(start_paused = true)]
async fn a_shell_that_exits_on_control_d_ends_the_session_at_once() {
    let mut pipes = pipes();
    let session = tokio::spawn(pipes.streams.run());
    drop(pipes.input);
    let started = tokio::time::Instant::now();
    assert_eq!(typed(&mut pipes.typed).await, b"\x03");
    assert_eq!(typed(&mut pipes.typed).await, b"\x04");
    assert_eq!(started.elapsed(), INTERRUPT_PAUSE);
    pipes.shell.write_all(b"exit\r\n").await.unwrap();
    drop(pipes.shell);
    let ran = session.await.unwrap().unwrap();
    assert!(ran.closed);
    assert_eq!(started.elapsed(), INTERRUPT_PAUSE);
    assert_eq!(
        pipes.output.recv().await,
        Some(ExecOutput::Bytes(b"exit\r\n".to_vec()))
    );
}

#[tokio::test(start_paused = true)]
async fn the_guard_closes_politely_though_nobody_reads_or_writes() {
    let mut pipes = pipes();
    // The input stays open and the output is gone: closing still ends it.
    drop(pipes.output);
    pipes.close.send(()).unwrap();
    let session = tokio::spawn(pipes.streams.run());
    assert_eq!(typed(&mut pipes.typed).await, b"\x03");
    assert_eq!(typed(&mut pipes.typed).await, b"\x04");
    pipes.shell.write_all(b"exit\r\n").await.unwrap();
    drop(pipes.shell);
    assert!(session.await.unwrap().is_none(), "nobody to tell");
    drop(pipes.input);
}

#[tokio::test(start_paused = true)]
async fn when_nobody_reads_the_shell_still_ends_politely() {
    let mut pipes = pipes();
    drop(pipes.output);
    pipes.shell.write_all(b"$ ").await.unwrap();
    let started = tokio::time::Instant::now();
    assert!(pipes.streams.run().await.is_none());
    assert_eq!(typed(&mut pipes.typed).await, b"\x03\x04");
    // The first batch waits out its window before it finds nobody reads.
    assert_eq!(
        started.elapsed(),
        BATCH_TIME + INTERRUPT_PAUSE + CLOSE_DEADLINE
    );
    drop(pipes.input);
}

#[tokio::test(start_paused = true)]
async fn a_session_ends_with_its_output() {
    let mut pipes = pipes();
    drop(pipes.shell);
    let ran = pipes.streams.run().await.unwrap();
    assert!(!ran.closed);
    assert!(!ran.output_seen);
    assert!(pipes.output.recv().await.is_none());
    drop((pipes.input, pipes.close, pipes.typed));
}
