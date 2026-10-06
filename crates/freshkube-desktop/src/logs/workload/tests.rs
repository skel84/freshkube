use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, TestAppContext, px, size};
use tokio::runtime::Runtime;

use freshkube_core::logs::ServiceId;
use freshkube_core::resources::{
    Container, ContainerRole, ContainerState, PodLogUpdate, WorkloadPod, WorkloadPods,
};

use super::{
    EXAMPLE_INTERVAL, MAX_STREAMS, PodsState, StreamKey, StreamState, Streams, WorkloadLogPanel,
    WorkloadLogView,
};
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example, live};

const CONTEXT: &str = "production";

/// The view on its own, active and on screen, as the Logs tab shows it.
fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<WorkloadLogView>, AnyWindowHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut view = None;
    let handle = cx.open_window(size(px(900.), px(820.)), |window, cx| {
        let logs = cx.new(|cx| {
            let mut logs = WorkloadLogView::for_workloads(runtime.handle().clone(), window, cx);
            logs.set_active(true, cx);
            logs
        });
        view = Some(logs.clone());
        Root::new(logs, window, cx)
    });
    cx.run_until_parked();
    (runtime, view.unwrap(), handle.into())
}

/// The example Deployment of `app`.
fn deployment(app: &str) -> ResourceIdentity {
    let (_, rows) = example::read(CONTEXT, "deployments.apps", None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| row.identity.name == app)
        .unwrap()
        .identity
}

/// Its selector, as the pane reads it from the document.
fn selector(workload: &ResourceIdentity) -> Option<String> {
    example::document(workload, live::now())
        .unwrap()
        .overview
        .selector
        .unwrap()
}

fn pods(workload: &ResourceIdentity, selector: &str) -> Vec<WorkloadPod> {
    example::workload_pods(workload, selector, live::now())
}

/// Opens the Logs tab on `workload` with its selector, as the pane does.
fn show(view: &Entity<WorkloadLogView>, workload: &ResourceIdentity, cx: &mut App) {
    let selector = selector(workload);
    view.update(cx, |view, cx| {
        view.show_workload(Some(workload.clone()), Some(KubeAccess::Example), cx);
        view.want(cx);
        view.set_selector(selector, cx);
    });
}

/// Opens the Logs tab with no selector known, so the test feeds the pods.
fn show_fed(view: &Entity<WorkloadLogView>, workload: &ResourceIdentity, cx: &mut App) {
    view.update(cx, |view, cx| {
        view.show_workload(Some(workload.clone()), Some(KubeAccess::Example), cx);
        view.want(cx);
    });
}

fn feed(view: &Entity<WorkloadLogView>, pods: Vec<WorkloadPod>, cx: &mut App) {
    view.update(cx, |view, cx| {
        let epoch = view.source().epoch;
        assert!(view.apply_pods(
            epoch,
            WorkloadPods {
                listed: true,
                pods,
                failure: None,
            },
            cx,
        ));
    });
}

fn status(window: &mut gpui_kit::Window) -> String {
    window
        .find("workload-logs-status")
        .label()
        .unwrap()
        .to_owned()
}

/// Log lines held, not counting markers.
fn lines(view: &Entity<WorkloadLogView>, cx: &App) -> usize {
    let entries = view.read(cx).retained();
    entries.iter().filter(|entry| !entry.is_marker()).count()
}

fn markers(view: &Entity<WorkloadLogView>, cx: &App) -> Vec<String> {
    let entries = view.read(cx).retained();
    entries
        .iter()
        .filter(|entry| entry.is_marker())
        .map(|entry| entry.message.to_string())
        .collect()
}

/// Every tag a line carries.
fn tags(view: &Entity<WorkloadLogView>, cx: &App) -> Vec<String> {
    let mut tags: Vec<String> = view
        .read(cx)
        .retained()
        .iter()
        .filter(|entry| !entry.is_marker())
        .map(|entry| entry.service.as_str().to_owned())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

fn key(pod: &WorkloadPod) -> StreamKey {
    StreamKey {
        pod: pod.name.clone(),
        uid: pod.uid.clone(),
        container: pod.containers.default.clone().unwrap(),
    }
}

#[gpui_kit::test]
fn nothing_is_read_until_the_tab_shows_then_every_pod_is_followed(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let selector = selector(&api).unwrap();
    assert_eq!(selector, "app=api");
    let expected = pods(&api, &selector);
    assert!(expected.len() > 3, "{}", expected.len());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.show_workload(Some(api.clone()), Some(KubeAccess::Example), cx);
            view.set_selector(Some(selector.clone()), cx);
        });
        window.render_frame(cx);
        // The pane passed the workload and its selector, but the tab hasn't
        // shown yet.
        assert!(!view.read(cx).reading());
        assert_eq!(status(window), "Idle");
        assert_eq!(lines(&view, cx), 0);

        view.update(cx, |view, cx| view.want(cx));
        window.render_frame(cx);
        assert!(view.read(cx).reading());
        let count = expected.len();
        assert_eq!(
            status(window),
            format!("Following: {count} pods · {count} containers")
        );
        // One chip per container, tagged pod/container.
        let mut creating = 0;
        for pod in &expected {
            let tag = format!("{}/api", pod.name);
            let label = window
                .find(format!("workload-logs-stream-{tag}"))
                .label()
                .unwrap()
                .to_owned();
            if !label.starts_with(&format!("{tag}: Streaming")) {
                assert!(
                    label.starts_with(&format!("{tag}: Waiting (ContainerCreating)")),
                    "{label}"
                );
                creating += 1;
            }
        }
        assert_eq!(creating, 1);
        // A container still being created has written nothing yet.
        assert_eq!(tags(&view, cx).len(), count - 1);
        assert!(window.try_find("logs-empty").is_none());
        assert!(window.try_find("workload-logs-capped").is_none());
        // Lines from every pod interleave by time.
        let keys: Vec<i64> = view
            .read(cx)
            .retained()
            .iter()
            .map(|entry| entry.timestamp.as_ref().map_or(0, |time| time.sort_key))
            .collect();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
    })
    .unwrap();

    // Running containers write on.
    let before = cx.update(|cx| lines(&view, cx));
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert!(cx.update(|cx| lines(&view, cx)) > before);
}

#[gpui_kit::test]
fn finding_pods_no_selector_and_no_pods_each_say_so(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    cx.update_window(handle, |_, window, cx| {
        // Wanted before the document is read.
        show_fed(&view, &api, cx);
        window.render_frame(cx);
        assert_eq!(status(window), "Finding pods");
        assert_eq!(
            window.find("logs-empty").label(),
            Some("Finding the workload's pods")
        );

        view.update(cx, |view, cx| view.set_selector(None, cx));
        window.render_frame(cx);
        assert_eq!(status(window), "No selector");
        assert_eq!(
            window.find("logs-empty").label(),
            Some("This workload selects no pods.")
        );

        view.update(cx, |view, cx| {
            view.set_selector(Some("app=nothing-runs".into()), cx)
        });
        window.render_frame(cx);
        assert_eq!(status(window), "No pods: Pods show here as they start.");
        assert_eq!(
            window.find("logs-empty").label(),
            Some("The workload runs no pods.")
        );
        assert!(window.try_find("workload-logs-streams").is_none());
    })
    .unwrap();
}

/// The metrics server refuses its logs, as a viewer without `pods/log`
/// would see; the other pods read on beside it.
#[gpui_kit::test]
fn a_failing_stream_says_why_while_the_others_read_on(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let coredns = deployment("coredns");
    let metrics = deployment("metrics-server");
    let mut both = pods(&coredns, "app=coredns");
    let refused = pods(&metrics, "app=metrics-server");
    both.extend(refused.clone());
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &coredns, cx);
        feed(&view, both, cx);
        window.render_frame(cx);
        let refused_tag = format!("{}/metrics-server", refused[0].name);
        let errors = view.read(cx).source().errors.clone();
        let error = errors.get(&ServiceId::new(refused_tag.clone())).unwrap();
        assert!(error.contains("forbidden"), "{error}");
        assert!(
            window
                .find(format!("workload-logs-stream-{refused_tag}"))
                .label()
                .unwrap()
                .starts_with(&format!("{refused_tag}: Failed"))
        );
        // The coredns pods stream.
        assert!(tags(&view, cx).iter().all(|tag| tag.ends_with("/coredns")));
        assert!(lines(&view, cx) > 0);
        assert!(status(window).starts_with("Following: "));
        assert!(window.try_find("workload-logs-failed").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn pods_joining_and_leaving_start_and_stop_their_streams(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let (joining, leaving) = (all[0].clone(), all[1].clone());
    let without_joining: Vec<_> = all[1..].to_vec();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, without_joining.clone(), cx);
        window.render_frame(cx);
        // The first listing marks nobody as joining.
        assert!(markers(&view, cx).is_empty());
        let tag = format!("{}/api", joining.name);
        assert!(
            window
                .try_find(format!("workload-logs-stream-{tag}"))
                .is_none()
        );

        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert_eq!(markers(&view, cx), [format!("Pod {} joined", joining.name)]);
        assert!(window.find(format!("workload-logs-stream-{tag}")).visible());
        assert!(tags(&view, cx).contains(&tag));

        let left_tag = format!("{}/api", leaving.name);
        let left_lines = view
            .read(cx)
            .retained()
            .iter()
            .filter(|entry| entry.service.as_str() == left_tag)
            .count();
        assert!(left_lines > 0);
        let rest: Vec<_> = all
            .iter()
            .filter(|pod| pod.name != leaving.name)
            .cloned()
            .collect();
        feed(&view, rest, cx);
        window.render_frame(cx);
        assert!(
            markers(&view, cx).contains(&format!("Pod {} left", leaving.name)),
            "{:?}",
            markers(&view, cx)
        );
        // Its stream stops; what it wrote stays.
        assert!(!view.read(cx).source().streams.contains_key(&key(&leaving)));
        assert!(
            window
                .try_find(format!("workload-logs-stream-{left_tag}"))
                .is_none()
        );
        let kept = view
            .read(cx)
            .retained()
            .iter()
            .filter(|entry| entry.service.as_str() == left_tag)
            .count();
        assert_eq!(kept, left_lines);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chip_hides_its_streams_lines_and_new_streams_show(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        let hidden = format!("{}/api", all[1].name);
        window.click(format!("workload-logs-stream-{hidden}"), cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("workload-logs-stream-{hidden}"))
                .checked(),
            Some(false)
        );
        let shown_tags = |view: &Entity<WorkloadLogView>, cx: &App| {
            let view = view.read(cx);
            view.visible_rows()
                .iter()
                .map(|&ix| view.retained()[ix].service.as_str().to_owned())
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert!(!shown_tags(&view, cx).contains(&hidden));
        assert!(!shown_tags(&view, cx).is_empty());

        // A pod that joins shows at once.
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        let joined = format!("{}/api", all[0].name);
        assert!(shown_tags(&view, cx).contains(&joined));
        assert!(!shown_tags(&view, cx).contains(&hidden));

        window.click(format!("workload-logs-stream-{hidden}"), cx);
        window.render_frame(cx);
        assert!(shown_tags(&view, cx).contains(&hidden));
    })
    .unwrap();
}

/// A stream behind the others delivers lines older than ones already
/// shown: they take their place by time, a paused review stays on the line
/// it showed, and the selection keeps its line.
#[gpui_kit::test]
fn a_lagging_streams_lines_take_their_place_without_moving_the_review(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let lagging = all[0].clone();
    let line = |minutes_ago: i64, text: &str| {
        let at = chrono::Utc::now() - chrono::TimeDelta::minutes(minutes_ago);
        PodLogUpdate::Line(format!(
            "{} level=info msg=\"{text}\"",
            at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
        ))
    };
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);

        // Following: a late line lands among the old ones, and the newest
        // row stays in view.
        let generation = view.read(cx).source().streams[&key(&lagging)].generation;
        let newest = view.read(cx).retained().last().unwrap().raw.clone();
        view.update(cx, |view, cx| {
            assert!(view.apply_updates(
                &key(&lagging),
                generation,
                vec![line(30, "late while following")],
                cx,
            ));
        });
        window.render_frame(cx);
        let retained = view.read(cx).retained();
        let late = retained
            .iter()
            .position(|entry| entry.raw.contains("late while following"))
            .unwrap();
        assert!(late + 1 < retained.len(), "placed by time, not appended");
        assert_eq!(retained.last().unwrap().raw, newest);
        assert!(view.read(cx).following());

        // Paused on a selected line in the middle of the review.
        window.click("logs-follow", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).following());
        let view_ref = view.read(cx);
        let rows = view_ref.visible_rows().len();
        let middle = view_ref.row_id(rows - 5);
        let generation_id = view_ref.generation();
        let row = format!("log-line-{generation_id}-{middle}");
        window.click(row.clone(), cx);
        window.render_frame(cx);
        assert_eq!(window.find(row.clone()).selected(), Some(true));
        let top_before = window.find(row.clone()).bounds().origin.y;

        let generation = view.read(cx).source().streams[&key(&lagging)].generation;
        view.update(cx, |view, cx| {
            assert!(
                view.apply_updates(
                    &key(&lagging),
                    generation,
                    (0..12)
                        .map(|ix| line(50 - ix, "late while paused"))
                        .collect(),
                    cx,
                )
            );
        });
        window.render_frame(cx);
        window.render_frame(cx);
        // The new lines sit above the selection, which stays put.
        let first_late = view
            .read(cx)
            .retained()
            .iter()
            .position(|entry| entry.raw.contains("late while paused"))
            .unwrap();
        let view_ref = view.read(cx);
        let selected_row = (0..view_ref.visible_rows().len())
            .find(|&ix| view_ref.row_id(ix) == middle)
            .unwrap();
        assert!(first_late < view_ref.visible_rows()[selected_row]);
        assert!(!view_ref.following());
        assert!(
            view_ref
                .review_status(true)
                .contains(&"1 selected".to_owned())
        );
        assert_eq!(window.find(row.clone()).selected(), Some(true));
        assert_eq!(window.find(row).bounds().origin.y, top_before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_stops_everything_and_showing_reads_on_without_repeats(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let read = cx
        .update_window(handle, |_, window, cx| {
            show(&view, &api, cx);
            window.render_frame(cx);
            assert!(view.read(cx).reading());
            view.update(cx, |view, cx| view.set_active(false, cx));
            window.render_frame(cx);
            assert!(!view.read(cx).reading());
            assert!(view.read(cx).source().streams.is_empty());
            assert_eq!(status(window), "Idle");
            lines(&view, cx)
        })
        .unwrap();
    // Nothing is written while hidden.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| lines(&view, cx)), read);

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.set_active(true, cx));
        window.render_frame(cx);
        assert!(view.read(cx).reading());
        // Each container reads on from its last line: nothing repeats, and
        // nobody counts as joining.
        assert_eq!(lines(&view, cx), read);
        assert!(markers(&view, cx).is_empty());
    })
    .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert!(cx.update(|cx| lines(&view, cx)) > read);
}

#[gpui_kit::test]
fn another_workload_or_none_drops_every_stream(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        show(&view, &deployment("api"), cx);
        window.render_frame(cx);
        assert!(lines(&view, cx) > 0);

        view.update(cx, |view, cx| {
            view.show_workload(Some(deployment("ledger")), None, cx)
        });
        window.render_frame(cx);
        assert!(!view.read(cx).reading());
        assert!(view.read(cx).source().streams.is_empty());
        assert_eq!(lines(&view, cx), 0);
        assert!(window.try_find("workload-logs-streams").is_none());

        show(&view, &deployment("ledger"), cx);
        window.render_frame(cx);
        assert!(tags(&view, cx).iter().all(|tag| tag.starts_with("ledger-")));
        view.update(cx, |view, cx| view.show_workload(None, None, cx));
        assert!(!view.read(cx).reading());
        assert!(view.read(cx).source().streams.is_empty());
    })
    .unwrap();
    // Nothing writes on once dropped.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| lines(&view, cx)), 0);
}

#[gpui_kit::test]
fn past_the_cap_the_newest_pods_are_read_and_the_rest_counted(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    // Every pod gains a sidecar, which makes more containers than the cap.
    let mut all = pods(&api, "app=api");
    for pod in &mut all {
        let mut sidecar: Container = pod.containers.containers[0].clone();
        sidecar.name = "proxy".into();
        sidecar.role = ContainerRole::App;
        sidecar.state = ContainerState::Running(None);
        pod.containers.containers.push(sidecar);
    }
    let containers: usize = all
        .iter()
        .map(|pod| {
            pod.containers
                .containers
                .iter()
                .filter(|container| container.role == ContainerRole::App)
                .count()
        })
        .sum();
    assert!(containers > MAX_STREAMS, "{containers}");
    let newest = all
        .iter()
        .max_by_key(|pod| pod.created)
        .unwrap()
        .name
        .clone();
    let oldest = all
        .iter()
        .min_by_key(|pod| pod.created)
        .unwrap()
        .name
        .clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().streams.len(), MAX_STREAMS);
        assert_eq!(
            window.find("workload-logs-capped").label(),
            Some(
                format!("Reading {MAX_STREAMS} of {containers} containers, the newest pods first.")
                    .as_str()
            )
        );
        let streams = &view.read(cx).source().streams;
        assert!(streams.keys().any(|key| key.pod == newest));
        assert!(streams.keys().all(|key| key.pod != oldest));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_pod_watch_says_why_and_retry_watches_again(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Failure, FailureKind};
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    cx.update_window(handle, |_, window, cx| {
        show(&view, &api, cx);
        let epoch = view.read(cx).source().epoch;
        view.update(cx, |view, cx| {
            view.apply_pods(
                epoch,
                WorkloadPods {
                    listed: false,
                    pods: Vec::new(),
                    failure: Some(Failure::new(
                        FailureKind::Forbidden,
                        "pods is forbidden".to_owned(),
                    )),
                },
                cx,
            );
        });
        window.render_frame(cx);
        assert!(matches!(
            view.read(cx).source().pods_state,
            PodsState::Failed(_)
        ));
        let failed = window
            .find("workload-logs-failed")
            .label()
            .unwrap()
            .to_owned();
        assert!(failed.contains("pods is forbidden"), "{failed}");

        window.click("workload-logs-retry", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().pods_state, PodsState::Watching);
        assert!(window.try_find("workload-logs-failed").is_none());
        // An older watch's answer is dropped.
        view.update(cx, |view, cx| {
            assert!(!view.apply_pods(epoch, WorkloadPods::default(), cx));
        });
        assert!(
            view.read(cx)
                .source()
                .streams
                .values()
                .any(|stream| stream.state == StreamState::Streaming)
        );
    })
    .unwrap();
}

#[test]
fn short_labels_drop_the_shared_prefix_up_to_a_dash() {
    let labels = |tags: &[&str]| {
        super::short_labels(tags.iter().map(|tag| ServiceId::new(*tag)))
            .into_values()
            .map(|label| label.to_string())
            .collect::<Vec<_>>()
    };
    // One ReplicaSet: the random suffix; one container name: no container.
    assert_eq!(
        labels(&["api-6c4f8d9f-bbbbg/api", "api-6c4f8d9f-bbbch/api"]),
        ["bbbbg", "bbbch"]
    );
    // A rollout keeps the template hash; a sidecar names the container.
    assert_eq!(
        labels(&["api-6c4f8d9f-bbbbg/api", "api-7d5e9a0b-cxk2p/proxy"]),
        ["6c4f8d9f-bbbbg/api", "7d5e9a0b-cxk2p/proxy"]
    );
    // StatefulSet ordinals, and a lone pod.
    assert_eq!(
        labels(&["db-0/db", "db-1/db", "db-10/db"]),
        ["0", "1", "10"]
    );
    assert_eq!(labels(&["report-28411/report"]), ["28411"]);
    assert_eq!(labels(&["solo/app"]), ["solo"]);
}
