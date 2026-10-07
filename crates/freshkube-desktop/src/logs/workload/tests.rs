use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, TestAppContext, px, size};
use tokio::runtime::Runtime;

use freshkube_core::logs::ServiceId;
use freshkube_core::resources::{
    Container, ContainerRole, ContainerState, PodLogUpdate, WorkloadPod, WorkloadPods,
};

use super::{
    EXAMPLE_INTERVAL, Fed, Jitter, MAX_STREAMS, PodsState, RETRY_FIRST, RETRY_JITTER, RETRY_MAX,
    StreamKey, StreamState, Streams, WorkloadLogPanel, WorkloadLogView,
};
use crate::desktop::probe;
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example, live};

const CONTEXT: &str = "production";

/// The view on its own, active and on screen, as the Logs tab shows it.
fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<WorkloadLogView>, AnyWindowHandle) {
    mount_sized(cx, 900., 820.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (Runtime, Entity<WorkloadLogView>, AnyWindowHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut view = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
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
        // Its chip says so, in the row or behind "+N".
        let chips = view.read(cx).source().chips.clone();
        let chip = chips
            .iter()
            .find(|chip| chip.service.as_str() == refused_tag)
            .unwrap();
        assert!(
            chip.tooltip.starts_with(&format!("{refused_tag}: Failed")),
            "{}",
            chip.tooltip
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
        // The markers' pod tags name no container, so the labels stay the
        // pods' short names.
        let labels = view.read(cx).source().labels.clone();
        assert!(labels.contains_key(&ServiceId::new(leaving.name.clone())));
        for (tag, label) in &labels {
            let tag = tag.as_str();
            assert!(!label.contains('/'), "{tag}: {label}");
            assert!(tag.starts_with("api-"), "{tag}");
            assert!(label.len() < tag.len(), "{tag}: {label}");
        }
        let chips = view.read(cx).source().chips.clone();
        assert!(chips.iter().all(|chip| !chip.label.contains('/')));
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
    let now = cx.update(|cx| view.read(cx).source().clock.now(cx));
    let line = |minutes_ago: i64, text: &str| {
        let at = now - chrono::TimeDelta::minutes(minutes_ago);
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

/// Showing again asks each container for its live log from where it
/// stopped, not its tail again.
#[gpui_kit::test]
fn showing_again_reads_on_from_each_streams_saved_position(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let saved = cx
        .update_window(handle, |_, window, cx| {
            show(&view, &api, cx);
            window.render_frame(cx);
            let source = view.read(cx).source();
            // The first read asks for the tail.
            assert!(source.streams.values().all(|stream| {
                stream.request.resume.is_none() && stream.request.tail.is_some()
            }));
            let saved = source.positions.clone();
            view.update(cx, |view, cx| view.set_active(false, cx));
            saved
        })
        .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.set_active(true, cx));
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert!(!source.streams.is_empty());
        let mut resumed = 0;
        for (key, stream) in &source.streams {
            assert!(!stream.request.previous);
            assert_eq!(stream.request.pod, key.pod);
            assert_eq!(stream.request.container, key.container);
            match saved.get(key).filter(|position| position.time().is_some()) {
                Some(position) => {
                    assert_eq!(stream.request.resume, Some(*position), "{key:?}");
                    resumed += 1;
                }
                None => assert_eq!(stream.request.resume, None, "{key:?}"),
            }
        }
        // Every container that wrote reads on.
        assert_eq!(resumed, tags(&view, cx).len());
    })
    .unwrap();
}

/// However many containers write, one delivery hands their lines to the
/// view: one apply, one ingest, one notify a frame.
#[gpui_kit::test]
fn busy_streams_are_applied_once_a_delivery(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let mut all = pods(&api, "app=api");
    for pod in &mut all {
        let mut sidecar: Container = pod.containers.containers[0].clone();
        sidecar.name = "proxy".into();
        sidecar.role = ContainerRole::App;
        sidecar.state = ContainerState::Running(None);
        pod.containers.containers.push(sidecar);
    }
    let now = cx.update(|cx| view.read(cx).source().clock.now(cx));
    for busy in [13, MAX_STREAMS] {
        cx.update_window(handle, |_, window, cx| {
            show_fed(&view, &api, cx);
            feed(&view, all.clone(), cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let streams: Vec<(StreamKey, u64)> = cx.update(|cx| {
            view.read(cx)
                .source()
                .streams
                .iter()
                .map(|(key, stream)| (key.clone(), stream.generation))
                .take(busy)
                .collect()
        });
        assert_eq!(streams.len(), busy);
        let sender = cx.update(|cx| view.update(cx, |view, cx| view.feed(cx)));
        let before = (
            probe::count("workload-logs.apply"),
            cx.update(|cx| lines(&view, cx)),
        );
        // Each container writes ten lines before the next delivery.
        for round in 0..10 {
            for (ix, (key, generation)) in streams.iter().enumerate() {
                let at = now + chrono::TimeDelta::milliseconds((round * 100 + ix) as i64);
                let line = format!(
                    "{} level=info msg=\"busy\"",
                    at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
                );
                sender
                    .try_send(Fed {
                        key: key.clone(),
                        generation: *generation,
                        update: PodLogUpdate::Line(line),
                    })
                    .unwrap();
            }
        }
        cx.run_until_parked();
        assert_eq!(probe::count("workload-logs.apply"), before.0 + 1, "{busy}");
        assert_eq!(cx.update(|cx| lines(&view, cx)), before.1 + busy * 10);
        // Another workload starts afresh.
        cx.update(|cx| view.update(cx, |view, cx| view.show_workload(None, None, cx)));
    }
}

/// A refused stream isn't read again on a timer: only when the pod list
/// changes (a pod added or removed), or on Retry.
#[gpui_kit::test]
fn a_refused_stream_waits_for_the_pods_to_change_or_retry(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let metrics = deployment("metrics-server");
    // One pod of the example's many, so one stream is refused.
    let refused = vec![pods(&metrics, "app=metrics-server").remove(0)];
    let refused_key = key(&refused[0]);
    let generation = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().streams[&refused_key].generation)
    };
    let still_refused = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let stream = &view.read(cx).source().streams[&refused_key];
            stream.state.refused() && stream.retry.is_none()
        })
    };
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &metrics, cx);
        feed(&view, refused.clone(), cx);
        window.render_frame(cx);
        let tag = refused_key.service();
        let label = window
            .find(format!("workload-logs-stream-{}", tag.as_str()))
            .label()
            .unwrap()
            .to_owned();
        assert!(label.contains("Failed"), "{label}");
        assert!(label.contains("read again when the pods change"), "{label}");
        assert_eq!(
            window.find("workload-logs-refused").label(),
            Some("Logs refused for 1 container: read again when the pods change.")
        );
    })
    .unwrap();
    assert!(still_refused(cx));
    let first = generation(cx);
    cx.executor().advance_clock(RETRY_MAX * 4);
    cx.run_until_parked();
    assert_eq!(generation(cx), first);

    // The same pods listed again change nothing.
    cx.update(|cx| feed(&view, refused.clone(), cx));
    assert_eq!(generation(cx), first);

    // A pod joins: the refused stream is read again, and refused again.
    let mut joined = refused.clone();
    joined.push(pods(&deployment("coredns"), "app=coredns").remove(0));
    cx.update(|cx| feed(&view, joined, cx));
    let second = generation(cx);
    assert!(second > first);
    assert!(still_refused(cx));

    // It leaves: read again.
    cx.update(|cx| feed(&view, refused.clone(), cx));
    let third = generation(cx);
    assert!(third > second);
    cx.executor().advance_clock(RETRY_MAX * 4);
    cx.run_until_parked();
    assert_eq!(generation(cx), third);

    // Retry reads it again at once.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workload-logs-retry-refused", cx);
    })
    .unwrap();
    assert!(generation(cx) > third);
    assert!(still_refused(cx));
}

/// A pod replaced under the same name, with a new uid, is a change too: a
/// refused stream is read again, its backoff from the start.
#[gpui_kit::test]
fn a_pod_replaced_under_its_name_wakes_a_refused_stream(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let metrics = deployment("metrics-server");
    let mut both = pods(&metrics, "app=metrics-server");
    both.truncate(2);
    let refused_key = key(&both[0]);
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &metrics, cx);
        feed(&view, both.clone(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let first = cx.update(|cx| {
        let stream = &view.read(cx).source().streams[&refused_key];
        assert!(stream.state.refused());
        stream.generation
    });
    // As if it had failed for a while before it was refused.
    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.source_mut().failures.insert(refused_key.clone(), 3);
        })
    });

    let mut replaced = both.clone();
    replaced[1].uid = format!("{}-replaced", replaced[1].uid);
    assert_eq!(replaced[1].name, both[1].name);
    cx.update(|cx| feed(&view, replaced.clone(), cx));
    cx.update(|cx| {
        let source = view.read(cx).source();
        assert!(source.streams[&refused_key].generation > first);
        assert!(!source.failures.contains_key(&refused_key));
        // The old incarnation's stream is gone; the new one's is read.
        assert!(!source.streams.contains_key(&key(&both[1])));
        assert!(source.streams.contains_key(&key(&replaced[1])));
    });
}

/// Retry reads every refused stream at once and starts its backoff over;
/// once none is refused, the notice goes.
#[gpui_kit::test]
fn retry_reads_refused_streams_and_starts_their_backoff_over(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Failure, FailureKind};
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let find = |cx: &mut TestAppContext, waiting: bool| {
        cx.update(|cx| {
            view.read(cx)
                .source()
                .streams
                .iter()
                .find(|(_, stream)| {
                    if waiting {
                        matches!(stream.state, StreamState::Waiting(_))
                    } else {
                        stream.state == StreamState::Streaming
                    }
                })
                .map(|(key, _)| key.clone())
                .unwrap()
        })
    };
    // The container still being created waits again when read again, so
    // its failures add up; a running one streams.
    let waiting = find(cx, true);
    let streaming = find(cx, false);
    let generation = |key: &StreamKey, cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().streams[key].generation)
    };
    let failures = |key: &StreamKey, cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().failures.get(key).copied())
    };
    let fail = |key: &StreamKey, kind: FailureKind, cx: &mut TestAppContext| {
        let generation = generation(key, cx);
        let failure = Failure::new(kind, "an invented failure".to_owned());
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(view.apply_updates(
                    key,
                    generation,
                    vec![PodLogUpdate::Failed(failure)],
                    cx,
                ));
            })
        });
    };
    let mut jitter = cx.update(|cx| view.read(cx).source().jitter.clone());
    let tick = Duration::from_millis(1);

    // A transient failure first: the creating container backs off once.
    let delay = jitter.spread(RETRY_FIRST);
    fail(&waiting, FailureKind::Unreachable, cx);
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    assert_eq!(failures(&waiting, cx), Some(1));

    // Then both are refused, and Retry reads them again.
    fail(&waiting, FailureKind::Forbidden, cx);
    fail(&streaming, FailureKind::Forbidden, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("workload-logs-refused").label(),
            Some("Logs refused for 2 containers: read again when the pods change.")
        );
        window.click("workload-logs-retry-refused", cx);
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert_eq!(source.streams[&streaming].state, StreamState::Streaming);
        assert!(matches!(
            source.streams[&waiting].state,
            StreamState::Waiting(_)
        ));
        assert!(source.refused_note.is_none());
        assert!(window.try_find("workload-logs-refused").is_none());
    })
    .unwrap();
    assert_eq!(failures(&waiting, cx), None);

    // Its next failure waits the first step again, not the second.
    let delay = jitter.spread(RETRY_FIRST);
    let before = generation(&waiting, cx);
    fail(&waiting, FailureKind::Unreachable, cx);
    cx.executor().advance_clock(delay - tick);
    cx.run_until_parked();
    assert_eq!(generation(&waiting, cx), before);
    cx.executor().advance_clock(tick);
    cx.run_until_parked();
    assert!(generation(&waiting, cx) > before);
}

/// A stream that fails for a while reads again with the watcher's backoff,
/// each wait spread by the jitter, and stops once its pod goes.
#[gpui_kit::test]
fn a_failed_stream_reads_again_with_a_jittered_backoff(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Failure, FailureKind};
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The container still being created: read again, it waits again, so
    // its failures add up.
    let waiting = cx.update(|cx| {
        view.read(cx)
            .source()
            .streams
            .iter()
            .find(|(_, stream)| matches!(stream.state, StreamState::Waiting(_)))
            .map(|(key, _)| key.clone())
            .unwrap()
    });
    let generation = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().streams[&waiting].generation)
    };
    let fail = |cx: &mut TestAppContext| {
        let generation = generation(cx);
        let unreachable = Failure::new(FailureKind::Unreachable, "connection refused".to_owned());
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(view.apply_updates(
                    &waiting,
                    generation,
                    vec![PodLogUpdate::Failed(unreachable)],
                    cx,
                ));
            })
        });
    };
    // The waits the view will draw, from the same sequence.
    let mut jitter = cx.update(|cx| view.read(cx).source().jitter.clone());
    let tick = Duration::from_millis(1);
    for wait in [RETRY_FIRST, RETRY_FIRST * 2] {
        let delay = jitter.spread(wait);
        assert_ne!(delay, wait);
        let before = generation(cx);
        fail(cx);
        cx.executor().advance_clock(delay - tick);
        cx.run_until_parked();
        assert_eq!(generation(cx), before, "{wait:?} spread to {delay:?}");
        cx.executor().advance_clock(tick);
        cx.run_until_parked();
        assert!(generation(cx) > before, "{wait:?} spread to {delay:?}");
    }

    // Once the pod goes, nothing reads it again.
    fail(cx);
    let rest: Vec<_> = all
        .iter()
        .filter(|pod| pod.name != waiting.pod)
        .cloned()
        .collect();
    cx.update(|cx| feed(&view, rest, cx));
    cx.executor().advance_clock(RETRY_MAX * 4);
    cx.run_until_parked();
    assert!(cx.update(|cx| !view.read(cx).source().streams.contains_key(&waiting)));
}

#[test]
fn retry_waits_spread_across_the_jitter() {
    let mut jitter = Jitter::new();
    let wait = Duration::from_secs(10);
    let (least, most) = (
        wait.mul_f64(1. - RETRY_JITTER),
        wait.mul_f64(1. + RETRY_JITTER),
    );
    let draws: Vec<Duration> = (0..1_000).map(|_| jitter.spread(wait)).collect();
    assert!(draws.iter().all(|&delay| delay >= least && delay <= most));
    // Spread over the whole range, not bunched at the plain wait.
    assert!(draws.iter().any(|&delay| delay < wait.mul_f64(0.85)));
    assert!(draws.iter().any(|&delay| delay > wait.mul_f64(1.15)));
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

/// Chips past the rows' room end in "+N", which lists every container
/// and shows or hides its lines like a chip.
#[gpui_kit::test]
fn chips_past_the_rows_end_in_more_which_lists_them_all(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount_sized(cx, 520., 820.);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        // A narrow panel: the chips fit its width on the next frame.
        window.render_frame(cx);
        window.render_frame(cx);
        let total = view.read(cx).source().chips.len();
        assert_eq!(total, all.len());
        let fits = view.read(cx).source().fits;
        assert!(fits < total, "{fits} of {total}");
        let shown = (0..total)
            .filter(|&ix| {
                let id = view.read(cx).source().chips[ix].id.clone();
                window.try_find(id).is_some()
            })
            .count();
        assert_eq!(shown, fits);
        let more = window
            .find("workload-logs-more")
            .label()
            .unwrap()
            .to_owned();
        assert!(more.contains(&format!("+{}", total - fits)), "{more}");

        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(window.find("workload-logs-list").visible());
        let last = view.read(cx).source().chips[total - 1].clone();
        window.click(last.list_id.clone(), cx);
        window.render_frame(cx);
        assert!(view.read(cx).source().hidden.contains(&last.service));
        assert_eq!(window.find(last.list_id.clone()).checked(), Some(false));
    })
    .unwrap();
}

/// The list behind "+N" stays closed once dismissed, through new frames,
/// new chips and another workload; it opens only when asked.
#[gpui_kit::test]
fn the_list_of_containers_stays_closed_once_dismissed(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount_sized(cx, 520., 820.);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let open = |view: &Entity<WorkloadLogView>, cx: &App| view.read(cx).source().more_open;
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        window.render_frame(cx);
        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(open(&view, cx));
        assert!(window.find("workload-logs-list").visible());

        // Dismissed by a click outside it.
        window.click("workload-logs-status", cx);
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());
    })
    .unwrap();
    // New frames and new lines leave it closed.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 2);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());

        // Open again, then a pod joins: another set of chips closes it.
        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(open(&view, cx));
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());

        // Open again, then another workload: closed.
        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(open(&view, cx));
        view.update(cx, |view, cx| {
            view.show_workload(Some(deployment("ledger")), None, cx)
        });
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());
    })
    .unwrap();
}

#[test]
fn chips_fit_their_rows_and_leave_room_for_more() {
    use gpui_kit::px;
    let fit = |widths: &[f32], rows| {
        let widths: Vec<_> = widths.iter().map(|&width| px(width)).collect();
        crate::logs::chips_that_fit(&widths, px(30.), px(6.), px(100.), rows)
    };
    // All fit: no "+N" needed.
    assert_eq!(fit(&[40., 40.], 1), 2);
    assert_eq!(fit(&[40., 40., 40., 40.], 2), 4);
    // The third doesn't fit one row; "+N" (30) after the first leaves 76.
    assert_eq!(fit(&[40., 40., 40.], 1), 1);
    // Two rows of two, then "+N" takes the second's place in the last row.
    assert_eq!(fit(&[40., 40., 40., 40., 40.], 2), 3);
    // No rows: only "+N".
    assert_eq!(fit(&[40.], 0), 0);
    // A chip too wide for any row still takes one alone.
    assert_eq!(fit(&[150.], 1), 1);
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
    // A pod's own tag, from its markers, doesn't make the container show.
    assert_eq!(
        labels(&[
            "api-6c4f8d9f-bbbbg",
            "api-6c4f8d9f-bbbbg/api",
            "api-6c4f8d9f-bbbch/api"
        ]),
        ["bbbbg", "bbbbg", "bbbch"]
    );
    // StatefulSet ordinals, and a lone pod.
    assert_eq!(
        labels(&["db-0/db", "db-1/db", "db-10/db"]),
        ["0", "1", "10"]
    );
    assert_eq!(labels(&["report-28411/report"]), ["28411"]);
    assert_eq!(labels(&["solo/app"]), ["solo"]);
}

/// The tags of the lines the list shows.
fn shown_tags(view: &Entity<WorkloadLogView>, cx: &App) -> std::collections::BTreeSet<String> {
    let view = view.read(cx);
    view.visible_rows()
        .iter()
        .map(|&ix| view.retained()[ix].service.as_str().to_owned())
        .collect()
}

/// Opens the Pod select and clicks its `item`th entry: All pods is 0, and
/// the pods follow a separator, from 2.
fn pick_pod(handle: AnyWindowHandle, item: usize, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workload-logs-pod", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(item, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_pod_select_narrows_the_lines_and_chips_to_one_pod(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some("Pod: All pods")
        );
    })
    .unwrap();
    let before = cx.update(|cx| shown_tags(&view, cx));
    assert!(before.len() > 1, "{before:?}");
    // Newest first, as the menu lists them.
    let picked = view.read_with(cx, |view, _| view.source().pod_choices[1].clone());
    pick_pod(handle, 3, cx);
    cx.update_window(handle, |_, window, cx| {
        let source = view.read(cx).source();
        assert_eq!(source.pod.as_deref(), Some(picked.name.as_str()));
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some(format!("Pod: {}", picked.label).as_str())
        );
        let tags = shown_tags(&view, cx);
        assert!(!tags.is_empty());
        assert!(
            tags.iter()
                .all(|tag| tag.split('/').next() == Some(picked.name.as_str())),
            "{tags:?}"
        );
        // The chips are the picked pod's containers.
        assert!(source.chips.iter().all(|chip| {
            chip.service
                .as_str()
                .starts_with(&format!("{}/", picked.name))
        }));
        assert_eq!(source.chips.len(), 1);
        // Every stream reads on.
        assert_eq!(source.streams.len(), all.len());
    })
    .unwrap();
    pick_pod(handle, 0, cx);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).source().pod, None);
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some("Pod: All pods")
        );
        assert_eq!(view.read(cx).source().chips.len(), all.len());
        assert_eq!(shown_tags(&view, cx), before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_picked_pod_that_leaves_keeps_its_pick_and_its_lines(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let leaving = all[0].name.clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        view.update(cx, |view, cx| view.pick_pod(Some(leaving.clone()), cx));
        window.render_frame(cx);
        let before = shown_tags(&view, cx);
        assert!(before.contains(&format!("{leaving}/api")));

        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert_eq!(source.pod.as_deref(), Some(leaving.as_str()));
        let gone = source.pod_choices.last().unwrap();
        assert_eq!(gone.name, leaving);
        assert!(gone.label.ends_with(" (gone)"), "{}", gone.label);
        assert!(
            window
                .find("workload-logs-pod")
                .label()
                .unwrap()
                .ends_with(" (gone)")
        );
        // What it wrote stays shown.
        assert!(shown_tags(&view, cx).contains(&format!("{leaving}/api")));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pod_past_the_cap_says_it_is_not_read(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
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
        let source = view.read(cx).source();
        let choice = source
            .pod_choices
            .iter()
            .find(|choice| choice.name == oldest)
            .unwrap();
        assert!(choice.label.ends_with(" · not read"), "{}", choice.label);
        assert!(
            source
                .pod_choices
                .iter()
                .filter(|choice| choice.label.ends_with(" · not read"))
                .all(|choice| !source.streams.keys().any(|key| key.pod == choice.name))
        );
        view.update(cx, |view, cx| view.pick_pod(Some(oldest.clone()), cx));
        window.render_frame(cx);
        // The list says why it is empty, in the cap note's words.
        assert_eq!(
            view.read(cx).source().empty.as_ref(),
            format!(
                "{oldest} isn't read. Reading {MAX_STREAMS} of {containers} containers, the newest pods first."
            )
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn timestamps_hide_and_show_each_lines_time(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, pods(&api, "app=api"), cx);
        window.render_frame(cx);
        assert!(view.read(cx).columns().time);
        assert_eq!(
            window.find("workload-logs-timestamps").checked(),
            Some(true)
        );
        window.click("workload-logs-timestamps", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).columns().time);
        assert_eq!(
            window.find("workload-logs-timestamps").checked(),
            Some(false)
        );
        window.click("workload-logs-timestamps", cx);
        window.render_frame(cx);
        assert!(view.read(cx).columns().time);
    })
    .unwrap();
}
