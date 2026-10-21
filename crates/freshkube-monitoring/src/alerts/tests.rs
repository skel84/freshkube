use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering::SeqCst};

use chrono::{TimeZone, Utc};
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, Hold, KubeClientSource};
use freshkube_core::failure::Failure;
use freshkube_core::fake_api::{self, Recorder};
use freshkube_core::monitoring::Matcher;
use freshkube_ui::layout_check;
use freshkube_ui::real_io::until_io;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
use serde_json::{Value, json};

// Named, not globbed: a glob would bring GPUI's `test` macro in, over Rust's.
use super::display::Body;
use super::{AlertObject, AlertsEvent, AlertsPage, Scope, Shape, Which, object_of};
use chrono::DateTime;
use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap()
}

fn example_source(id: &str) -> ClusterSource {
    ClusterSource {
        id: id.into(),
        context: "prod-fra".into(),
        access: ClusterAccess::EXAMPLE,
    }
}

/// The page alone in a window, its clock fixed.
fn mount(
    cx: &mut TestAppContext,
    which: Which,
    source: Option<ClusterSource>,
    shape: Shape,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<AlertsPage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Light, None, cx);
        cx.set_reduce_motion(true);
        cx.bind_keys(super::key_bindings());
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut page = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = AlertsPage::new(which, runtime.handle().clone(), cx);
            view.set_now(now);
            view.shape = shape;
            view.set_source(source, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, window.into(), page.unwrap())
}

fn show(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<AlertsPage>) {
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.run_until_parked();
    frame(cx, handle);
}

fn frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn lines(cx: &mut TestAppContext, page: &Entity<AlertsPage>) -> Vec<String> {
    cx.read(|cx| page.read(cx).lines_text())
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = SharedString::from(id.to_owned());
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

/// Waits for a read on a real runtime to land.
fn landed(cx: &mut TestAppContext, page: &Entity<AlertsPage>) {
    until_io(cx, "the alerts read", |cx| {
        cx.read(|cx| {
            let page = page.read(cx);
            !page.is_reading() && !matches!(page.display().body, Body::Loading)
        })
    });
}

/// The example's active alerts as the table lists them: critical first, each
/// name's alerts together; silenced and inhibited ones are hidden.
fn by_severity_then_name() -> Vec<String> {
    let mut expected = Vec::new();
    for name in [
        "KubePodCrashLooping",
        "NodeFilesystemSpaceFillingUp",
        "CPUThrottlingHigh",
        "KubeDeploymentReplicasMismatch",
        "TargetDown",
    ] {
        expected.push(format!("# {name}"));
        expected.push(name.to_owned());
    }
    expected
}

#[gpui_kit::test]
fn the_alerts_page_groups_by_severity_then_name_and_hides_what_is_muted(cx: &mut TestAppContext) {
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Firing);
    show(cx, handle, &page);
    assert_eq!(lines(cx, &page), by_severity_then_name());
    // Pending is a state of its own, never firing.
    let pending = cx.read(|cx| {
        page.read(cx)
            .display()
            .rows
            .iter()
            .find(|row| row.cells[0].text == "CPUThrottlingHigh")
            .map(|row| (row.tone, row.cells[1].text.to_string()))
    });
    assert_eq!(pending, Some((Tone::Waiting, "Pending".to_owned())));

    cx.update_window(handle, |_, window, cx| window.click("alerts-muted", cx))
        .unwrap();
    frame(cx, handle);
    let all = lines(cx, &page);
    assert!(all.contains(&"KubeNodeNotReady".to_owned()), "{all:?}");
    assert!(all.contains(&"InfoInhibitor".to_owned()), "{all:?}");
    cx.update_window(handle, |_, window, cx| window.click("alerts-muted", cx))
        .unwrap();
    frame(cx, handle);
    assert_eq!(lines(cx, &page).len(), 10, "the toggle clears");
}

#[gpui_kit::test]
fn a_filter_keeps_the_rows_that_match(cx: &mut TestAppContext) {
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Firing);
    show(cx, handle, &page);
    cx.update_window(handle, |_, window, cx| {
        window.click("alerts-filter", cx);
        window.input("payments", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        lines(cx, &page),
        ["# KubePodCrashLooping", "KubePodCrashLooping"],
        "the filter matches labels, not only names"
    );
    cx.update_window(handle, |_, window, cx| window.input("zzz-nothing", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(lines(cx, &page).is_empty());
    assert!(
        shown(cx, handle, "alerts-filtered"),
        "it says the filter hides them"
    );
}

#[gpui_kit::test]
fn selecting_an_alert_shows_its_runbook_and_its_labels_open_their_objects(cx: &mut TestAppContext) {
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Firing);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let seen = opened.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, event: &AlertsEvent, _| {
            seen.borrow_mut().push(event.clone())
        })
    });
    show(cx, handle, &page);
    let key = cx.read(|cx| page.read(cx).display().rows[0].key.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("alerts-row-{key}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(cx.read(|cx| page.read(cx).selected().cloned()), Some(key));
    assert!(shown(cx, handle, "alerts-detail-title"));
    assert!(
        shown(cx, handle, "alerts-detail-runbook"),
        "a web runbook opens"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("alerts-detail-label-pod", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *opened.borrow(),
        [AlertsEvent::Open(AlertObject {
            kind: "pods",
            namespace: "payments".into(),
            name: "payments-api-6d8f9c7b5-x2k4q".into(),
        })]
    );
    // A label that names no object is plain text, never a link.
    assert!(shown(cx, handle, "alerts-detail-label-container"));
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| page.read(cx).selected().cloned()), None);
}

#[test]
fn labels_name_an_object_only_when_they_can() {
    let labels = |pairs: &[(&str, &str)]| -> Vec<(SharedString, SharedString)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned().into(), (*b).to_owned().into()))
            .collect()
    };
    let with_namespace = labels(&[("namespace", "shop"), ("pod", "web-1")]);
    assert_eq!(
        object_of("pod", "web-1", &with_namespace),
        Some(AlertObject {
            kind: "pods",
            namespace: "shop".into(),
            name: "web-1".into()
        })
    );
    // A namespaced kind with no namespace on the alert opens nothing.
    assert_eq!(
        object_of("pod", "web-1", &labels(&[("pod", "web-1")])),
        None
    );
    assert_eq!(
        object_of("node", "worker-1", &[]),
        Some(AlertObject {
            kind: "nodes",
            namespace: String::new(),
            name: "worker-1".into()
        })
    );
    assert_eq!(object_of("severity", "critical", &with_namespace), None);
    assert_eq!(object_of("pod", "", &with_namespace), None);
}

#[gpui_kit::test]
fn what_was_not_read_is_never_no_alerts(cx: &mut TestAppContext) {
    for (shape, id) in [
        (Shape::NotFound, "alerts-not-found"),
        (Shape::Refused, "alerts-refused"),
        (Shape::NoLabel, "alerts-no-label"),
        (Shape::RulesOnly, "alerts-rules-only"),
    ] {
        let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(example_source("a")), shape);
        show(cx, handle, &page);
        assert!(shown(cx, handle, id), "{shape:?} says {id}");
        assert!(!shown(cx, handle, "alerts-quiet"), "{shape:?} isn't quiet");
        assert!(lines(cx, &page).is_empty(), "{shape:?} shows no rows");
        // Retrying is offered, since asking again can help.
        assert!(shown(cx, handle, "alerts-retry"), "{shape:?}");
    }
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Quiet);
    show(cx, handle, &page);
    assert!(
        shown(cx, handle, "alerts-quiet"),
        "only a current empty read is quiet"
    );
    let words = cx.read(|cx| match &page.read(cx).display().body {
        Body::State(state) => state.description.to_string(),
        other => format!("{other:?}"),
    });
    assert!(
        words.contains("Alertmanager 0.27.0 (3 replicas)") && words.contains("vmalert"),
        "it names what was read: {words}"
    );
}

#[gpui_kit::test]
fn a_stale_read_keeps_its_rows_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Stale);
    show(cx, handle, &page);
    assert!(!lines(cx, &page).is_empty(), "the last read stays");
    assert!(shown(cx, handle, "alerts-note-stale"));
    let status = cx.read(|cx| page.read(cx).status.clone());
    assert!(
        status.text(None).contains("last read"),
        "the status bar marks it too: {}",
        status.text(None)
    );
}

#[gpui_kit::test]
fn the_several_alertmanagers_say_which_was_read(cx: &mut TestAppContext) {
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Several);
    show(cx, handle, &page);
    assert!(shown(cx, handle, "alerts-note-alerts-several"));
}

#[gpui_kit::test]
fn the_rules_page_shows_a_broken_rule_as_a_problem_and_unevaluated_ones_as_unknown(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) =
        mount(cx, Which::Rules, Some(example_source("a")), Shape::Firing);
    show(cx, handle, &page);
    assert_eq!(lines(cx, &page)[0], "# kubernetes-apps");
    let tones = cx.read(|cx| {
        page.read(cx)
            .display()
            .rows
            .iter()
            .map(|row| (row.cells[0].text.to_string(), row.tone))
            .collect::<Vec<_>>()
    });
    assert!(
        tones.contains(&("TargetDown".to_owned(), Tone::Crit)),
        "{tones:?}"
    );
    assert!(tones.contains(&("KubePodCrashLooping".to_owned(), Tone::Good)));
    let groups = cx.read(|cx| page.read(cx).display().groups.clone());
    assert!(
        groups[0].detail.iter().any(|d| d == "1 failing"),
        "{groups:?}"
    );
    assert_eq!(groups[0].tone, Tone::Crit);

    // Definitions nobody evaluates are Unknown, never OK.
    let (_runtime, handle, page) = mount(
        cx,
        Which::Rules,
        Some(example_source("b")),
        Shape::RulesOnly,
    );
    show(cx, handle, &page);
    let tones = cx.read(|cx| {
        page.read(cx)
            .display()
            .rows
            .iter()
            .map(|row| row.tone)
            .collect::<Vec<_>>()
    });
    assert!(!tones.is_empty() && tones.iter().all(|tone| *tone == Tone::Unknown));
    assert!(shown(cx, handle, "rules-note-rules-unevaluated"));
}

#[gpui_kit::test]
fn the_silences_page_lists_each_state_with_the_alerts_it_mutes_now(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(
        cx,
        Which::Silences,
        Some(example_source("a")),
        Shape::Firing,
    );
    show(cx, handle, &page);
    let lines = lines(cx, &page);
    assert_eq!(
        lines.iter().filter(|line| line.starts_with('#')).count(),
        3,
        "{lines:?}"
    );
    let muting = cx.read(|cx| {
        page.read(cx)
            .display()
            .rows
            .iter()
            .find(|row| row.key == "silence-worker-2")
            .map(|row| row.cells[4].text.to_string())
    });
    assert_eq!(muting.as_deref(), Some("1 alert"));
    // Read-only: nothing here silences or expires.
    for id in ["silences-silence", "silences-expire", "alerts-silence"] {
        assert!(!shown(cx, handle, id), "{id}");
    }
}

#[gpui_kit::test]
fn a_hold_leaves_the_page_on_its_loading_rows(cx: &mut TestAppContext) {
    let source = ClusterSource {
        access: ClusterAccess::Example(Hold {
            monitoring: true,
            ..Hold::default()
        }),
        ..example_source("held")
    };
    let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(source), Shape::Firing);
    // Real I/O answers from the runtime's threads.
    cx.executor().allow_parking();
    show(cx, handle, &page);
    assert!(matches!(
        cx.read(|cx| page.read(cx).display().body.clone()),
        Body::Loading
    ));
    assert!(cx.read(|cx| page.read(cx).loading_motion().is_some()));
}

#[gpui_kit::test]
fn another_source_drops_what_was_read(cx: &mut TestAppContext) {
    let (_runtime, handle, page) =
        mount(cx, Which::Alerts, Some(example_source("a")), Shape::Firing);
    show(cx, handle, &page);
    assert!(!lines(cx, &page).is_empty());
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_source(Some(example_source("b")), cx);
            // Before anything answers, nothing of the old source shows.
            assert!(page.lines_text().is_empty());
            assert!(matches!(page.display().body, Body::Loading));
        })
    });
    cx.run_until_parked();
    assert!(!lines(cx, &page).is_empty(), "and the new one answers");
}

struct InMemory(kube::Client);

impl KubeClientSource for InMemory {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

fn service(name: &str, port: Value) -> Value {
    json!({"metadata": {"name": name, "namespace": "monitoring"}, "spec": {"ports": [port]}})
}

/// A cluster with an Alertmanager behind the proxy. `status` is what its
/// alerts answer with; the alerts carry `cluster` when `labelled`.
fn cluster(
    status: Arc<AtomicU16>,
    labelled: bool,
) -> (ClusterSource, Recorder, tokio::runtime::Runtime) {
    // The client's worker runs on the runtime it is made in.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _enter = runtime.enter();
    let (client, recorder) = fake_api::client(move |request| {
        let path = request.path.as_str();
        if path == "/api/v1/services" || path == "/api/v1/namespaces/monitoring/services" {
            return Some(fake_api::json(
                200,
                json!({"kind": "ServiceList", "apiVersion": "v1", "metadata": {},
                       "items": [service("alertmanager-operated", json!({"name": "web", "port": 9093}))]}),
            ));
        }
        let rest = path.strip_prefix(
            "/api/v1/namespaces/monitoring/services/alertmanager-operated:9093/proxy/",
        )?;
        match rest {
            "api/v2/status" => Some(fake_api::json(
                200,
                json!({"versionInfo": {"version": "0.27.0"}, "cluster": {"peers": [{}]}}),
            )),
            "api/v2/alerts" => {
                let code = status.load(SeqCst);
                if code != 200 {
                    return Some(fake_api::json(code, json!({"message": "down"})));
                }
                let mut labels = json!({"alertname": "KubePodCrashLooping", "severity": "critical",
                                        "namespace": "payments", "pod": "payments-api-1"});
                if labelled {
                    labels["cluster"] = json!("prod-fra");
                }
                Some(fake_api::json(
                    200,
                    json!([{"fingerprint": "f1", "startsAt": "2026-10-21T11:00:00Z",
                            "labels": labels, "annotations": {"summary": "crash looping"},
                            "status": {"state": "active", "silencedBy": [], "inhibitedBy": []}}]),
                ))
            }
            "api/v2/silences" => Some(fake_api::json(200, json!([]))),
            _ => None,
        }
    });
    let source = ClusterSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: ClusterAccess::Live(Arc::new(InMemory(client))),
    };
    (source, recorder, runtime)
}

#[gpui_kit::test]
fn a_hidden_page_reads_nothing_and_a_shown_one_only_reads(cx: &mut TestAppContext) {
    let (source, recorder, _cluster) = cluster(Arc::new(AtomicU16::new(200)), true);
    let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(source), Shape::Firing);
    // Real I/O answers from the runtime's threads.
    cx.executor().allow_parking();
    cx.run_until_parked();
    assert!(recorder.requests().is_empty(), "hidden: nothing is asked");
    show(cx, handle, &page);
    landed(cx, &page);
    assert!(!recorder.requests().is_empty());
    recorder.assert_all("alerts are read, never written", |request| {
        request.method == "GET"
    });
    assert_eq!(
        lines(cx, &page),
        ["# KubePodCrashLooping", "KubePodCrashLooping"]
    );

    // Hiding drops the page's reads: a refresh while hidden asks nothing.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    let asked = recorder.requests().len();
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(recorder.requests().len(), asked);
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_last_read_as_stale_and_a_first_failure_is_not_quiet(
    cx: &mut TestAppContext,
) {
    let status = Arc::new(AtomicU16::new(200));
    let (source, _recorder, _cluster) = cluster(status.clone(), true);
    let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(source), Shape::Firing);
    // Real I/O answers from the runtime's threads.
    cx.executor().allow_parking();
    show(cx, handle, &page);
    landed(cx, &page);
    assert!(!shown(cx, handle, "alerts-note-stale"));

    status.store(502, SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    until_io(cx, "the failed refresh", |cx| {
        cx.read(|cx| page.read(cx).failure.is_some())
    });
    frame(cx, handle);
    assert!(
        shown(cx, handle, "alerts-note-stale"),
        "marked as the last read"
    );
    assert_eq!(lines(cx, &page).len(), 2, "the rows stay");

    // Another source that never read: a failure says so, never "all quiet".
    let (source, _recorder, _cluster) = cluster(Arc::new(AtomicU16::new(502)), true);
    let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(source), Shape::Firing);
    // Real I/O answers from the runtime's threads.
    cx.executor().allow_parking();
    show(cx, handle, &page);
    until_io(cx, "the failed read", |cx| {
        cx.read(|cx| page.read(cx).failure.is_some())
    });
    frame(cx, handle);
    assert!(shown(cx, handle, "alerts-failed"));
    assert!(!shown(cx, handle, "alerts-quiet"));
}

#[gpui_kit::test]
fn a_central_store_without_the_cluster_label_shows_nothing(cx: &mut TestAppContext) {
    let scope = || Scope::Central {
        namespace: "monitoring".into(),
        matcher: Matcher::new("cluster", "prod-fra"),
    };
    let (source, _recorder, _cluster) = cluster(Arc::new(AtomicU16::new(200)), false);
    let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(source), Shape::Firing);
    // Real I/O answers from the runtime's threads.
    cx.executor().allow_parking();
    cx.update(|cx| page.update(cx, |page, cx| page.set_scope(scope(), None, cx)));
    show(cx, handle, &page);
    landed(cx, &page);
    assert!(lines(cx, &page).is_empty());
    assert!(shown(cx, handle, "alerts-no-label"));

    // With the label, the cluster's alerts show, in a cluster column.
    let (source, _recorder, _cluster) = cluster(Arc::new(AtomicU16::new(200)), true);
    let (_runtime, handle, page) = mount(cx, Which::Alerts, Some(source), Shape::Firing);
    // Real I/O answers from the runtime's threads.
    cx.executor().allow_parking();
    cx.update(|cx| page.update(cx, |page, cx| page.set_scope(scope(), None, cx)));
    show(cx, handle, &page);
    landed(cx, &page);
    assert_eq!(lines(cx, &page).len(), 2);
    assert!(cx.read(|cx| page.read(cx).display().central));
}

#[gpui_kit::test]
fn each_page_keeps_the_table_page_frame(cx: &mut TestAppContext) {
    for (which, page_id, title, text, table, list) in [
        (
            Which::Alerts,
            "alerts-page",
            "alerts-title",
            "Alerts",
            "alerts-table-scroll",
            "alerts-list",
        ),
        (
            Which::Rules,
            "rules-page",
            "rules-title",
            "Rules",
            "rules-table-scroll",
            "rules-list",
        ),
        (
            Which::Silences,
            "silences-page",
            "silences-title",
            "Silences",
            "silences-table-scroll",
            "silences-list",
        ),
    ] {
        let (_runtime, handle, page) = mount(cx, which, Some(example_source("a")), Shape::Firing);
        show(cx, handle, &page);
        cx.update_window(handle, |_, window, cx| {
            layout_check::settle_header(window, cx);
            layout_check::assert_table_page(
                window,
                cx,
                &layout_check::TablePage {
                    page: page_id,
                    title,
                    title_text: text,
                    table,
                    list,
                },
            );
        })
        .unwrap();
    }
}
