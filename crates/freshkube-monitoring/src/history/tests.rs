use std::time::Duration;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, px, size};

use super::*;

const NOW: i64 = 1_700_003_600;

fn example() -> Option<HistorySource> {
    Some(HistorySource {
        id: "example".into(),
        kind: HistoryKind::Example,
        client: 0,
        owner: None,
    })
}

fn pod(name: &str) -> Option<Subject> {
    Some(Subject::Pod {
        namespace: "payments".into(),
        name: name.into(),
    })
}

/// The history alone in a window, its clock fixed.
fn mount(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<HistoryView>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut history = None;
    let window = cx.open_window(size(px(720.), px(600.)), |window, cx| {
        let view = cx.new(|_| {
            let mut view = HistoryView::new(runtime.handle().clone(), "pod");
            view.set_now(|| NOW);
            view
        });
        history = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, window.into(), history.unwrap())
}

fn update(
    cx: &mut TestAppContext,
    history: &Entity<HistoryView>,
    change: impl FnOnce(&mut HistoryView, &mut Context<HistoryView>),
) {
    cx.update(|cx| history.update(cx, change));
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = SharedString::from(id.to_owned());
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

/// How many of the panels show an answer.
fn answered(cx: &mut TestAppContext, history: &Entity<HistoryView>) -> usize {
    cx.read(|cx| {
        history
            .read(cx)
            .panels()
            .iter()
            .filter(|panel| panel.read(cx).is_ready())
            .count()
    })
}

#[gpui_kit::test]
fn without_a_prometheus_nothing_draws_or_reads(cx: &mut TestAppContext) {
    let (_runtime, handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let view = history.read(cx);
        assert!(!view.shows());
        assert!(!view.reading());
        assert!(view.panels().is_empty());
    });
    assert!(!shown(cx, handle, "pod-history"));
    assert!(!shown(cx, handle, "monitoring-panel-pod-cpu"));
}

#[gpui_kit::test]
fn the_history_reads_only_while_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| history.read(cx).shows()));
    assert_eq!(answered(cx, &history), 0, "a hidden history read");

    update(cx, &history, |view, cx| view.set_visible(true, cx));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
    assert!(shown(cx, handle, "pod-history"));
    assert!(shown(cx, handle, "monitoring-panel-pod-cpu"));
    assert!(shown(cx, handle, "monitoring-panel-pod-memory"));
}

#[gpui_kit::test]
fn hiding_mid_read_drops_it_and_showing_reads_again(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
        assert!(view.reading());
        view.set_visible(false, cx);
        assert!(!view.reading());
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 0, "a dropped read answered");

    // Hidden, the minute passes without a read.
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 0);

    update(cx, &history, |view, cx| view.set_visible(true, cx));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
}

#[gpui_kit::test]
fn another_pod_or_cluster_starts_over(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);

    // Another pod: new panels, read at once since they show.
    update(cx, &history, |view, cx| {
        view.set_subject(pod("checkout-5d8b7-lwz8r"), cx);
        assert!(view.reading());
    });
    assert_eq!(answered(cx, &history), 0);
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);

    // Another cluster: nothing of the last one stays.
    update(cx, &history, |view, cx| {
        view.set_source(
            Some(HistorySource {
                id: "other".into(),
                kind: HistoryKind::Example,
                client: 0,
                owner: None,
            }),
            cx,
        )
    });
    assert_eq!(answered(cx, &history), 0);
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);

    // No Prometheus: nothing draws.
    update(cx, &history, |view, cx| view.set_source(None, cx));
    cx.read(|cx| {
        assert!(!history.read(cx).shows());
        assert!(!history.read(cx).reading());
    });

    // No pod: nothing draws either.
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(None, cx);
    });
    cx.read(|cx| assert!(!history.read(cx).shows()));
}

#[gpui_kit::test]
fn a_shown_history_reads_again_each_minute(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    let before = freshkube_probe::probe::count("monitoring.history.ask");
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    let asks = || freshkube_probe::probe::count("monitoring.history.ask") - before;
    assert_eq!(asks(), 1);

    // Showing again within the minute keeps the answer; Refresh reads again.
    update(cx, &history, |view, cx| {
        view.set_visible(false, cx);
        view.set_visible(true, cx);
    });
    assert_eq!(asks(), 1);
    update(cx, &history, |view, cx| view.refresh(cx));
    cx.run_until_parked();
    assert_eq!(asks(), 2);

    // The timer fires the next read.
    cx.executor().advance_clock(EVERY);
    cx.run_until_parked();
    assert_eq!(asks(), 3);
    assert_eq!(answered(cx, &history), 2);
}

#[gpui_kit::test]
fn the_cursor_on_one_chart_shows_on_the_other(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    let panels = cx.read(|cx| history.read(cx).panels().to_vec());
    cx.update(|cx| {
        panels[0].update(cx, |_, cx| {
            cx.emit(PanelEvent::Cursor(Some((NOW - 600) as f64)))
        })
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let linked = &history.read(cx).linked;
        assert!(linked.crosshair(1).is_some());
        assert!(linked.crosshair(0).is_none());
    });
}

#[gpui_kit::test]
fn a_rebuilt_client_reads_again_through_it(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    let before = freshkube_probe::probe::count("monitoring.history.ask");
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    let asks = || freshkube_probe::probe::count("monitoring.history.ask") - before;
    assert_eq!(asks(), 1);

    // The same source again keeps the answer; one whose client the page
    // built again reads at once, without waiting for the minute.
    update(cx, &history, |view, cx| view.set_source(example(), cx));
    assert_eq!(asks(), 1);
    update(cx, &history, |view, cx| {
        let mut source = example();
        source.as_mut().unwrap().client = 1;
        view.set_source(source, cx)
    });
    cx.run_until_parked();
    assert_eq!(asks(), 2);
    assert_eq!(answered(cx, &history), 2);
}

fn cpu_identity(view: &HistoryView) -> HistoryIdentity {
    HistoryIdentity {
        source: view.source.as_ref().unwrap().key(),
        subject: view.subject.clone().unwrap(),
        panel: "cpu",
    }
}

fn cpu_result(subject: &Subject, window: TimeWindow) -> PanelResult {
    let source = ExampleSource::new(&subject.seed());
    let spec = history::dashboard(subject).panels.remove(0);
    let context = QueryContext {
        window,
        variables: Default::default(),
    };
    source
        .query_panel(&spec, &context, &Variables::default())
        .unwrap()
}

#[gpui_kit::test]
fn returning_to_the_same_subject_rejects_its_earlier_answer(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api"), cx);
        view.set_visible(true, cx);
        let identity = cpu_identity(view);
        let old = view.reads[0].start(identity.clone());
        view.set_subject(pod("checkout"), cx);
        view.set_subject(pod("api"), cx);
        let window = TimeWindow::new(NOW, SPAN, POINTS);
        view.land_panel(
            0,
            &old,
            window,
            Ok(cpu_result(&identity.subject, window)),
            cx,
        );
        assert!(
            view.reads[0].is_loading(),
            "an old answer ended the current read"
        );
        assert!(
            !view.panels[0].read(cx).is_ready(),
            "an old answer reached the new panel"
        );
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
}

#[gpui_kit::test]
fn showing_again_rejects_an_answer_from_before_the_hide(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api"), cx);
        view.set_visible(true, cx);
        let identity = cpu_identity(view);
        let old = view.reads[0].start(identity.clone());
        view.set_visible(false, cx);
        view.set_visible(true, cx);
        let window = TimeWindow::new(NOW, SPAN, POINTS);
        view.land_panel(
            0,
            &old,
            window,
            Ok(cpu_result(&identity.subject, window)),
            cx,
        );
        assert!(
            view.reads[0].is_loading(),
            "an old answer ended the resumed read"
        );
        assert!(
            !view.panels[0].read(cx).is_ready(),
            "a cut read answered after showing"
        );
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
}

#[gpui_kit::test]
fn a_recent_failed_answer_keeps_the_ask_time_on_showing(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    let before = freshkube_probe::probe::count("monitoring.history.ask");
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api"), cx);
        view.set_visible(true, cx);
        view.run_panel(
            0,
            cpu_identity(view),
            TimeWindow::new(NOW, SPAN, POINTS),
            true,
            async { Err(QueryError::refused()) },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 1);
    update(cx, &history, |view, cx| {
        assert!(!view.reading());
        assert_eq!(
            view.reads[0].error(),
            Some(QueryError::refused().message.as_str())
        );
        view.set_visible(false, cx);
        view.set_visible(true, cx);
        assert_eq!(view.asked, Some(NOW));
    });
    cx.run_until_parked();
    assert_eq!(
        freshkube_probe::probe::count("monitoring.history.ask") - before,
        1
    );
    assert_eq!(answered(cx, &history), 1);

    update(cx, &history, |view, cx| view.refresh(cx));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
}

/// A loopback server that answers every request as a Prometheus with no
/// series and records the path and query it was asked for.
fn recording_server(
    runtime: &tokio::runtime::Runtime,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let log = seen.clone();
    runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]);
                let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                log.lock().unwrap().push(path);
                let body = r#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#;
                let answer = format!(
                    "HTTP/1.1 200 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    (format!("http://{address}/"), seen)
}

#[gpui_kit::test]
async fn the_history_of_a_scoped_store_sends_the_entrys_matcher(cx: &mut TestAppContext) {
    use freshkube_ui::real_io::RealIo;
    cx.executor().allow_parking();
    let (runtime, handle, history) = mount(cx);
    let (url, seen) = recording_server(&runtime);
    let prometheus = Prometheus::direct(url, None).unwrap().with_matcher(
        freshkube_core::monitoring::Matcher::new("cluster", "cluster-b"),
    );
    update(cx, &history, |view, cx| {
        view.set_source(
            Some(HistorySource {
                id: "open".into(),
                kind: HistoryKind::Ready(prometheus),
                client: 0,
                owner: None,
            }),
            cx,
        );
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    let observed = seen.clone();
    cx.wait_for_io(handle, move |_, _| observed.lock().unwrap().len() >= 2)
        .await;
    let asked = seen.lock().unwrap().clone();
    assert!(
        asked
            .iter()
            .all(|path| path.contains("cluster%3D%22cluster-b%22")),
        "an unscoped history read: {asked:?}"
    );
}
