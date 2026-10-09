//! Proof for the session registry (#44). Nothing here changes behaviour: it
//! pins what a parked, hidden or replaced session may and may not do.
use super::session::SessionKey;
use super::switch_tests::live_launch;
use super::{Page, Pilot};
use gpui_kit::test::TestAppContextExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A Kubernetes API on loopback that remembers every request. It answers
/// `/version`, then lists empty (a printer table when asked for one), holds
/// every watch open until the client closes it, and can be told to refuse
/// every list or to hold them unanswered. Anything that contacts a cluster
/// shows here, whatever it does with the answer.
#[derive(Clone, Default)]
struct FakeApi {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    /// Watch connections the client still holds open: summary, and the
    /// Resources page's (printer table) apart.
    watching: Arc<AtomicUsize>,
    table_watching: Arc<AtomicUsize>,
    refuse_lists: Arc<AtomicBool>,
    hold_lists: Arc<AtomicBool>,
}

struct Request {
    line: String,
    table: bool,
    watch: bool,
}

impl FakeApi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Self {
            port: listener.local_addr().unwrap().port(),
            ..Self::default()
        };
        let served = api.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let api = served.clone();
                std::thread::spawn(move || api.serve(stream));
            }
        });
        api
    }

    fn serve(&self, mut stream: TcpStream) {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => return,
            }
        }
        let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
        let line = text.lines().next().unwrap_or_default().to_owned();
        let request = Request {
            table: text.contains("as=table"),
            watch: line.contains("watch="),
            line,
        };
        let (version, watch, table) = (
            request.line.starts_with("get /version "),
            request.watch,
            request.table,
        );
        self.requests.lock().unwrap().push(request);
        let json = |body: &str| {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        if version {
            let body =
                r#"{"major":"1","minor":"30","gitVersion":"v1.30.0","platform":"linux/amd64"}"#;
            _ = stream.write_all(json(body).as_bytes());
            return;
        }
        if self.refuse_lists.load(Ordering::SeqCst) {
            _ = stream.write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return;
        }
        if watch {
            self.hold_watch(stream, table);
            return;
        }
        while self.hold_lists.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let body = if table {
            r#"{"kind":"Table","apiVersion":"meta.k8s.io/v1","metadata":{"resourceVersion":"1"},"columnDefinitions":[{"name":"Name","type":"string","format":"name","description":"","priority":0}],"rows":[]}"#
        } else {
            r#"{"kind":"List","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[]}"#
        };
        _ = stream.write_all(json(body).as_bytes());
    }

    /// Answers a watch's headers and sends nothing more until the client
    /// closes its end.
    fn hold_watch(&self, mut stream: TcpStream, table: bool) {
        let counter = if table {
            &self.table_watching
        } else {
            &self.watching
        };
        _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n",
        );
        counter.fetch_add(1, Ordering::SeqCst);
        _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
        counter.fetch_sub(1, Ordering::SeqCst);
    }

    fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Requests that are not the connection check: the reads themselves.
    fn reads(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| !request.line.starts_with("get /version "))
            .count()
    }

    /// Requests for a printer table: what the Resources lists send.
    fn table_reads(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.table)
            .count()
    }

    /// Whether any request has been made twice: a read that was retried.
    fn repeated(&self) -> bool {
        let requests = self.requests.lock().unwrap();
        requests.iter().enumerate().any(|(ix, request)| {
            requests[..ix]
                .iter()
                .any(|earlier| earlier.line == request.line)
        })
    }

    fn watching(&self) -> usize {
        self.watching.load(Ordering::SeqCst)
    }

    fn table_watching(&self) -> usize {
        self.table_watching.load(Ordering::SeqCst)
    }
}

/// Polls `until` against a real-time deadline, letting the app run between
/// looks. True when it held in time.
async fn within(
    cx: &mut TestAppContext,
    limit: Duration,
    until: impl Fn(&mut TestAppContext) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Waits until `count` has held steady for `quiet` of real time, within
/// `limit`, and returns it: whatever was in flight has arrived.
async fn steady(
    cx: &mut TestAppContext,
    limit: Duration,
    quiet: Duration,
    count: impl Fn() -> usize,
) -> usize {
    let deadline = Instant::now() + limit;
    let (mut last, mut since) = (count(), Instant::now());
    loop {
        cx.run_until_parked();
        let now = count();
        if now != last {
            (last, since) = (now, Instant::now());
        } else if since.elapsed() >= quiet {
            return now;
        }
        assert!(Instant::now() < deadline, "the count never settled");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// True when `holds` stays true for the whole of `span` of real time.
async fn throughout(
    cx: &mut TestAppContext,
    span: Duration,
    holds: impl Fn(&mut TestAppContext) -> bool,
) -> bool {
    let deadline = Instant::now() + span;
    while Instant::now() < deadline {
        cx.run_until_parked();
        if !holds(cx) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    true
}

/// A folder whose workspace lists `one` and `two`, Kubernetes-only entries
/// whose kubeconfig contexts point at the two fake APIs. The kubeconfig is
/// written by `write_kubeconfig`, so a test can replace it.
fn folder(one: &FakeApi, two: &FakeApi) -> tempfile::TempDir {
    let guard = tempfile::tempdir().unwrap();
    write_kubeconfig(guard.path(), one, two, "not-a-real-token");
    std::fs::write(
        guard.path().join("workspace.json"),
        format!(
            r#"{{"version":1,"kubeconfig":{},"clusters":[{{"id":"one","role":"core","context":"one"}},{{"id":"two","role":"core","context":"two"}}]}}"#,
            serde_json::to_string(&guard.path().join("kubeconfig")).unwrap()
        ),
    )
    .unwrap();
    guard
}

fn write_kubeconfig(directory: &std::path::Path, one: &FakeApi, two: &FakeApi, token: &str) {
    let server = |name: &str, api: &FakeApi| {
        format!(
            "- name: {name}\n  cluster:\n    server: http://127.0.0.1:{}\n",
            api.port
        )
    };
    std::fs::write(
        directory.join("kubeconfig"),
        format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: one\nclusters:\n{}{}contexts:\n- name: one\n  context:\n    cluster: one\n    user: u\n- name: two\n  context:\n    cluster: two\n    user: u\nusers:\n- name: u\n  user:\n    token: {token}\n",
            server("one", one),
            server("two", two),
        ),
    )
    .unwrap();
}

fn key(id: &str) -> SessionKey {
    SessionKey::Entry(id.into())
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

fn open_entry(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

async fn settled(cx: &mut TestAppContext, view: &Entity<Pilot>) {
    assert!(
        within(cx, Duration::from_secs(10), |cx| !view
            .read_with(cx, |pilot, _| pilot.config_loading))
        .await,
        "the startup read never finished"
    );
}

#[gpui_kit::test]
async fn a_parked_session_stops_retrying_though_the_open_one_retries(cx: &mut TestAppContext) {
    let (one, two) = (FakeApi::start(), FakeApi::start());
    // `one` refuses every list, so its reads go on retrying while it is open.
    one.refuse_lists.store(true, Ordering::SeqCst);
    let guard = folder(&one, &two);
    let (_runtime, handle, view) = launched(cx, &guard, &one).await;
    assert_eq!(two.hits(), 0, "an entry not opened is not contacted");
    // The detector sees a retry while the entry is open: a read is made
    // again, after the 1 s of backoff, which a first round cannot satisfy.
    assert!(
        within(cx, Duration::from_secs(15), |_| one.repeated()).await,
        "the open session's retry was never seen, so the check below proves nothing"
    );
    open_entry(cx, handle, &view, "two");
    assert!(within(cx, Duration::from_secs(10), |_| two.hits() > 0).await);
    cx.read(|cx| assert_eq!(view.read(cx).registry.active_key(), &key("two")));
    // A switch can land mid-round: wait for what was in flight to arrive.
    let parked = steady(
        cx,
        Duration::from_secs(10),
        Duration::from_millis(500),
        || one.reads(),
    )
    .await;
    // Longer than the next backoff steps, so a retry still alive would show.
    assert!(
        throughout(cx, Duration::from_secs(5), |_| one.reads() == parked).await,
        "the parked entry was contacted again"
    );
    assert!(two.hits() > 0);
}

#[gpui_kit::test]
async fn a_parked_sessions_watches_are_closed_and_not_reopened(cx: &mut TestAppContext) {
    let (one, two) = (FakeApi::start(), FakeApi::start());
    let guard = folder(&one, &two);
    let (_runtime, handle, view) = launched(cx, &guard, &one).await;
    assert!(
        within(cx, Duration::from_secs(10), |_| one.watching() > 0).await,
        "the open session holds watches"
    );
    open_entry(cx, handle, &view, "two");
    assert!(
        within(cx, Duration::from_secs(10), |_| two.watching() > 0).await,
        "the new session holds its own"
    );
    assert!(
        within(cx, Duration::from_secs(10), |_| one.watching() == 0).await,
        "the parked session still holds a watch open"
    );
    let parked = one.reads();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.refresh_summary(window, cx))
    })
    .unwrap();
    assert!(
        throughout(cx, Duration::from_secs(4), |_| one.watching() == 0
            && one.reads() == parked)
        .await,
        "the parked session read or watched again"
    );
}

#[gpui_kit::test]
async fn a_hidden_resources_page_reads_nothing_and_a_shown_one_does(cx: &mut TestAppContext) {
    let (one, two) = (FakeApi::start(), FakeApi::start());
    let guard = folder(&one, &two);
    let (_runtime, handle, view) = launched(cx, &guard, &one).await;
    assert!(within(cx, Duration::from_secs(10), |_| one.watching() > 0).await);
    assert_eq!(
        one.table_reads(),
        0,
        "no list is read while Resources is hidden"
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(Page::Resources, window, cx))
    })
    .unwrap();
    assert!(
        within(cx, Duration::from_secs(10), |_| one.table_watching() > 0).await,
        "a shown Resources page lists and watches"
    );
    let shown = one.table_reads();
    assert!(shown > 0);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx))
    })
    .unwrap();
    assert!(
        within(cx, Duration::from_secs(10), |_| one.table_watching() == 0).await,
        "hiding the page left its watch open"
    );
    // Past the first retry step (1 s) and the next (2 s).
    assert!(
        throughout(cx, Duration::from_secs(3), |_| one.table_reads() == shown
            && one.table_watching() == 0)
        .await,
        "a hidden page read again"
    );
}

#[gpui_kit::test]
async fn a_parked_summary_comes_back_as_last_known_only_if_its_kubeconfig_is_unchanged(
    cx: &mut TestAppContext,
) {
    let (one, two) = (FakeApi::start(), FakeApi::start());
    let guard = folder(&one, &two);
    let (_runtime, handle, view) = launched(cx, &guard, &one).await;
    let has_data = |cx: &mut TestAppContext| {
        view.read_with(cx, |pilot, _| {
            pilot.registry.active().kubernetes_summary.data().is_some()
        })
    };
    assert!(
        within(cx, Duration::from_secs(10), &has_data).await,
        "one's summary never loaded"
    );

    // Control: away and back with the kubeconfig as it was. The returning
    // reads are held, so the restored summary stays what is on show.
    open_entry(cx, handle, &view, "two");
    settled(cx, &view).await;
    one.hold_lists.store(true, Ordering::SeqCst);
    open_entry(cx, handle, &view, "one");
    settled(cx, &view).await;
    assert!(
        within(cx, Duration::from_secs(10), |cx| view
            .read_with(cx, |pilot, _| pilot.last_known_since().is_some()))
        .await,
        "an unchanged kubeconfig gets its last summary back"
    );
    one.hold_lists.store(false, Ordering::SeqCst);
    assert!(
        within(cx, Duration::from_secs(10), |cx| view
            .read_with(cx, |pilot, _| pilot.last_known_since().is_none())
            && has_data(cx))
        .await,
        "the reads that follow replace it"
    );

    // Away again, the kubeconfig replaced, and back: nothing comes back.
    open_entry(cx, handle, &view, "two");
    settled(cx, &view).await;
    write_kubeconfig(guard.path(), &one, &two, "a-replaced-token");
    one.hold_lists.store(true, Ordering::SeqCst);
    open_entry(cx, handle, &view, "one");
    settled(cx, &view).await;
    // The revision is what a restore is decided by: look only once known,
    // or the quiet window below would pass before it could have mattered.
    assert!(
        within(cx, Duration::from_secs(10), |cx| view
            .read_with(cx, |pilot, _| pilot.current_revision().is_some()))
        .await,
        "the kubeconfig's revision never became known"
    );
    assert!(
        throughout(cx, Duration::from_secs(1), |cx| view
            .read_with(cx, |pilot, _| {
                pilot.last_known_since().is_none() && pilot.restored_taken.is_none()
            }))
        .await,
        "a replaced kubeconfig still got the old summary"
    );
    one.hold_lists.store(false, Ordering::SeqCst);
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
    let mut uids: Vec<(String, std::collections::BTreeSet<String>)> = Vec::new();
    for id in ["core-fra", "dev-fra", "core-fra"] {
        switch_to(cx, handle, &view, id);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| pilot.navigate(Page::Resources, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        // The Resources list's own rows, not the example data they came from.
        let rows = cx.read(|cx| view.read(cx).resources.read(cx).store_identities());
        assert!(!rows.is_empty(), "{id}: the list shows no rows");
        assert!(
            rows.iter()
                .all(|row| row.connection == example::connection(id)),
            "{id}: a row names another cluster"
        );
        assert_eq!(
            cx.read(|cx| view.read(cx).kube_source().unwrap().id),
            example::connection(id)
        );
        uids.push((id.into(), rows.into_iter().map(|row| row.uid).collect()));
    }
    // The same names, other objects: no UID is shared between the clusters,
    // and coming back shows the first cluster's own again.
    assert!(uids[0].1.is_disjoint(&uids[1].1));
    assert_eq!(uids[0].1, uids[2].1);
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
    let epoch = cx.read(|cx| view.read(cx).epoch);
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
    cx.read(|cx| assert_ne!(view.read(cx).epoch, epoch, "the epoch did not move"));
    // The switch cancels the read itself: its abort lands on a Tokio worker,
    // so look for the receiver closing until a real-time deadline.
    assert!(
        within(cx, Duration::from_secs(5), |_| open.is_closed()).await,
        "the read outlived the switch"
    );
    _ = open.send(());
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
    let pod = pod_of("core-fra", SAME_NAME);
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
        assert!(forward.port_of(&pod_of("dev-fra", SAME_NAME)).is_none());
    });
}
