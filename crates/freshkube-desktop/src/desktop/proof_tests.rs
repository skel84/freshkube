//! Proof for the session registry (#44). Nothing here changes behaviour: it
//! pins what a parked, hidden or replaced session may and may not do.
use super::session::SessionKey;
use super::switch_tests::live_launch;
use super::{Page, Pilot};
use gpui_kit::test::TestAppContextExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A Kubernetes API that refuses every request but /version and remembers what
/// it was asked: the request line and its Accept header. Anything that
/// contacts a cluster shows here, whatever it does with the answer.
struct FakeApi {
    port: u16,
    requests: Arc<Mutex<Vec<(String, String)>>>,
}

impl FakeApi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let seen = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let seen = seen.clone();
                std::thread::spawn(move || {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte) {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    let text = String::from_utf8_lossy(&head).into_owned();
                    let line = text.lines().next().unwrap_or_default().to_owned();
                    let accept = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("accept:")
                                .map(str::to_owned)
                        })
                        .unwrap_or_default();
                    let version = line.starts_with("GET /version ");
                    seen.lock().unwrap().push((line, accept));
                    // The connection check is answered, so the cluster reads
                    // as connected; anything else is refused.
                    let answer = if version {
                        let body = r#"{"major":"1","minor":"30","gitVersion":"v1.30.0","platform":"linux/amd64"}"#;
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    };
                    _ = stream.write_all(answer.as_bytes());
                });
            }
        });
        Self { port, requests }
    }

    fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Requests for a printer table: what the Resources lists send.
    fn table_reads(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, accept)| accept.contains("as=table"))
            .count()
    }
}

/// A folder whose workspace lists `one` and `two`, Kubernetes-only entries
/// whose kubeconfig contexts point at the two fake APIs.
fn folder(one: &FakeApi, two: &FakeApi) -> tempfile::TempDir {
    let guard = tempfile::tempdir().unwrap();
    let kubeconfig = guard.path().join("kubeconfig");
    let server = |name: &str, api: &FakeApi| {
        format!(
            "- name: {name}\n  cluster:\n    server: http://127.0.0.1:{}\n",
            api.port
        )
    };
    std::fs::write(
        &kubeconfig,
        format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: one\nclusters:\n{}{}contexts:\n- name: one\n  context:\n    cluster: one\n    user: u\n- name: two\n  context:\n    cluster: two\n    user: u\nusers:\n- name: u\n  user:\n    token: not-a-real-token\n",
            server("one", one),
            server("two", two),
        ),
    )
    .unwrap();
    std::fs::write(
        guard.path().join("workspace.json"),
        format!(
            r#"{{"version":1,"kubeconfig":{},"clusters":[{{"id":"one","role":"core","context":"one"}},{{"id":"two","role":"core","context":"two"}}]}}"#,
            serde_json::to_string(&kubeconfig).unwrap()
        ),
    )
    .unwrap();
    guard
}

fn key(id: &str) -> SessionKey {
    SessionKey::Entry(id.into())
}

/// Lets real-time work run: the fake API is on other threads.
fn let_threads_run() {
    std::thread::sleep(Duration::from_millis(400));
}

async fn launched(
    cx: &mut TestAppContext,
    guard: &tempfile::TempDir,
    one: &FakeApi,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(10), |_, cx| {
        !view.read(cx).config_loading
    })
    .await;
    cx.wait_for(handle, Duration::from_secs(10), |_, _| one.hits() > 0)
        .await;
    (runtime, handle, view)
}

#[gpui_kit::test]
async fn a_parked_session_contacts_nothing_and_the_open_one_does(cx: &mut TestAppContext) {
    let (one, two) = (FakeApi::start(), FakeApi::start());
    let guard = folder(&one, &two);
    let (_runtime, handle, view) = launched(cx, &guard, &one).await;
    assert_eq!(two.hits(), 0, "an entry not opened is not contacted");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.activate_entry("two", window, cx))
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(10), |_, _| two.hits() > 0)
        .await;
    let_threads_run();
    let parked = one.hits();
    // Everything that can reach a cluster is asked to: a manual Refresh, the
    // shell's cycle, more time. The parked entry hears nothing.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.refresh_summary(window, cx);
        })
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    let_threads_run();
    assert_eq!(one.hits(), parked, "the parked entry was contacted");
    assert!(two.hits() > 0);
    cx.read(|cx| assert_eq!(view.read(cx).registry.active_key(), &key("two")));
}

#[gpui_kit::test]
async fn a_hidden_resources_page_reads_nothing_and_a_shown_one_does(cx: &mut TestAppContext) {
    let (one, two) = (FakeApi::start(), FakeApi::start());
    let guard = folder(&one, &two);
    let (_runtime, handle, view) = launched(cx, &guard, &one).await;
    let_threads_run();
    assert_eq!(
        one.table_reads(),
        0,
        "no list is read while Resources is hidden"
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(Page::Resources, window, cx))
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(10), |_, _| {
        one.table_reads() > 0
    })
    .await;
    let shown = one.table_reads();
    // Leaving it stops the page: nothing more is listed while it is hidden.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let_threads_run();
    let settled = one.table_reads();
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    let_threads_run();
    assert!(settled >= shown);
    assert_eq!(one.table_reads(), settled, "a hidden page read again");
}

// Two example sessions -----------------------------------------------------

use crate::resources::model::{ObjectRef, ResourceIdentity};
use crate::resources::{self, example, live};
use freshkube_core::resources::{ForwardTarget, builtin};

/// The example pod `name` of cluster `context`, as a row names it.
fn pod_of(context: &str, name: &str) -> ResourceIdentity {
    let (_, rows) = example::read(context, "pods", None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| row.identity.name == name)
        .unwrap()
        .identity
}

/// The same-named pod in two example clusters: another object in each, with
/// its own UID and the connection of the cluster that holds it.
const SAME_NAME: &str = "coredns-6c4f8d9b-bbbbb";

fn link_to(identity: ResourceIdentity) -> ObjectRef {
    identity.into()
}

fn open_in_window(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    link: ObjectRef,
) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                link,
                resources::Tab::Overview,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
}

/// Which object the Resources detail pane holds, and the cluster the shell
/// reads it from: (object's connection, UID, source connection, entry).
fn holding(
    cx: &mut TestAppContext,
    view: &Entity<Pilot>,
) -> Option<(String, String, Option<String>, SessionKey)> {
    cx.read(|cx| {
        let pilot = view.read(cx);
        pilot
            .resources
            .read(cx)
            .detail_identity(cx)
            .map(|identity| {
                (
                    identity.connection.clone(),
                    identity.uid.clone(),
                    pilot.kube_source().map(|source| source.id),
                    pilot.registry.active_key().clone(),
                )
            })
    })
}

fn switch_to(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_same_named_object_resolves_to_its_own_cluster_before_and_after_a_switch(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = super::tests::fixture(cx, 1280., 820.);
    let core = pod_of("core-fra", SAME_NAME);
    let dev = pod_of("dev-fra", SAME_NAME);
    assert_eq!(core.name, dev.name);
    assert_ne!(core.uid, dev.uid, "two objects, not one");
    assert_ne!(core.connection, dev.connection);

    // Both have been open in this window, so each has a session to find.
    switch_to(cx, handle, &view, "dev-fra");
    switch_to(cx, handle, &view, "core-fra");
    open_in_window(cx, handle, &view, link_to(core.clone()));
    assert_eq!(
        holding(cx, &view),
        Some((
            core.connection.clone(),
            core.uid.clone(),
            Some(core.connection.clone()),
            SessionKey::Entry("core-fra".into())
        )),
        "before a switch: core's own object, read from core"
    );

    // dev's link, made in dev, opens dev first and then its own object.
    open_in_window(cx, handle, &view, link_to(dev.clone()));
    assert_eq!(
        holding(cx, &view),
        Some((
            dev.connection.clone(),
            dev.uid.clone(),
            Some(dev.connection.clone()),
            SessionKey::Entry("dev-fra".into())
        )),
        "after a switch: dev's object, never core's of that name"
    );

    // And back: core's link opens core's object, not dev's.
    open_in_window(cx, handle, &view, link_to(core.clone()));
    assert_eq!(
        holding(cx, &view),
        Some((
            core.connection.clone(),
            core.uid.clone(),
            Some(core.connection.clone()),
            SessionKey::Entry("core-fra".into())
        ))
    );

    // A plain switch leaves nothing of the other cluster's object open.
    switch_to(cx, handle, &view, "dev-fra");
    let after = holding(cx, &view);
    assert!(
        after.is_none_or(|(object, ..)| object == dev.connection),
        "core's object stayed open in dev's window"
    );
}

#[gpui_kit::test]
fn each_clusters_rows_carry_their_own_connection_after_a_switch(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = super::tests::fixture(cx, 1280., 820.);
    for id in ["core-fra", "dev-fra", "core-fra"] {
        switch_to(cx, handle, &view, id);
        let (context, source) = cx.read(|cx| {
            let pilot = view.read(cx);
            (
                pilot.applied.context.clone().unwrap(),
                pilot.kube_source().unwrap().id,
            )
        });
        assert_eq!(context, id);
        assert_eq!(source, example::connection(id));
        let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|row| row.identity.connection == example::connection(id)),
            "{id}: a row names another cluster"
        );
    }
}

// Late answers ---------------------------------------------------------------

fn settle(cx: &mut TestAppContext) {
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
async fn an_object_answer_from_the_cluster_just_left_is_dropped(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_runtime, handle, view) = super::tests::fixture(cx, 1280., 820.);
    switch_to(cx, handle, &view, "core-fra");
    let applied = std::rc::Rc::new(std::cell::Cell::new(0));
    let (open, gate) = tokio::sync::oneshot::channel::<()>();
    let seen = applied.clone();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            // The shell's own epoch check, as every object read uses it.
            let epoch = pilot.epoch;
            pilot.cancel_object_open();
            pilot.start_object_open(
                "late",
                async move { gate.await.map(|_| 1u32).map_err(|e| e.to_string()) },
                move |pilot, _| pilot.epoch != epoch,
                move |_, _, _, _| seen.set(seen.get() + 1),
                window,
                cx,
            )
        })
    })
    .unwrap();
    // The window moves to another cluster while the answer is out.
    switch_to(cx, handle, &view, "dev-fra");
    // The switch cancelled the read itself, so there is nothing to answer;
    // a send that finds its receiver gone is the proof.
    assert!(open.send(()).is_err(), "the read outlived the switch");
    settle(cx);
    assert_eq!(applied.get(), 0, "core's answer landed in dev's window");
    cx.read(|cx| assert!(view.read(cx).object_open_job.is_none()));
}

// Forwards -------------------------------------------------------------------

#[gpui_kit::test]
async fn a_forward_keeps_running_on_its_own_connection_through_a_switch(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, view) = super::tests::fixture(cx, 1280., 820.);
    switch_to(cx, handle, &view, "core-fra");
    let (_, rows) = example::read("core-fra", "pods", None, live::now()).unwrap();
    let pod = rows
        .into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity;
    let resource = builtin("pods").unwrap();
    let spec = crate::forwards::ForwardSpec {
        runtime: runtime.handle().clone(),
        access: resources::KubeAccess::Example,
        target: ForwardTarget::of(&resource, &pod.name, &pod.uid).unwrap(),
        identity: pod.clone(),
        context: "core-fra".into(),
        port: 8080,
        local_port: None,
    };
    let list = cx.update(crate::forwards::list);
    let forward = cx.update(|cx| list.update(cx, |list, cx| list.start(spec, cx)));
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        forward.read(cx).local_port.is_some()
    })
    .await;
    assert_eq!(cx.read(crate::forwards::running), 1);

    // No question about forwards on a switch, and the switch happens.
    switch_to(cx, handle, &view, "dev-fra");
    assert!(
        cx.pending_prompt().is_none(),
        "a forward is not asked about"
    );
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).registry.active_key(),
            &SessionKey::Entry("dev-fra".into())
        )
    });
    // It runs on, under the connection it started with.
    settle(cx);
    assert_eq!(cx.read(crate::forwards::running), 1);
    cx.read(|cx| {
        let forward = forward.read(cx);
        assert!(forward.running());
        // Still core's pod, by the identity it started with, never dev's
        // same-named one.
        assert_eq!(forward.port_of(&pod), Some(8080));
        assert!(forward.local_port.is_some());
        assert!(forward.port_of(&pod_of("dev-fra", &pod.name)).is_none());
    });
}
