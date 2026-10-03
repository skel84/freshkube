use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, TestAppContext, px, size};
use tokio::runtime::Runtime;

use freshkube_core::resources::{ContainerRole, PodContainers};

use super::{EXAMPLE_INTERVAL, PodLogView, StreamState};
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example, live};

/// The view on its own, active and on screen, as the Logs tab shows it.
fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<PodLogView>, AnyWindowHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut view = None;
    let handle = cx.open_window(size(px(620.), px(820.)), |window, cx| {
        let logs = cx.new(|cx| {
            let mut logs = PodLogView::for_pods(runtime.handle().clone(), window, cx);
            logs.set_active(true, cx);
            logs
        });
        view = Some(logs.clone());
        Root::new(logs, window, cx)
    });
    cx.run_until_parked();
    (runtime, view.unwrap(), handle.into())
}

/// The first example pod whose status and containers `pick` accepts, from
/// a cluster large enough to have one of each.
fn pod(pick: impl Fn(&str, &PodContainers) -> bool) -> ResourceIdentity {
    let (_, rows) = example::read("production", "pods", None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| pick(&row.cells[2], &containers(&row.identity)))
        .unwrap()
        .identity
}

fn containers(identity: &ResourceIdentity) -> PodContainers {
    example::document(identity, live::now())
        .unwrap()
        .overview
        .pod
        .unwrap()
}

fn has_init(containers: &PodContainers) -> bool {
    containers
        .containers
        .iter()
        .any(|container| container.role == ContainerRole::Init)
}

fn restarted(containers: &PodContainers) -> bool {
    containers
        .containers
        .iter()
        .any(|container| container.has_previous())
}

/// A running pod that never restarted, so it has no previous instance.
/// The metrics server refuses its logs.
fn running_pod() -> ResourceIdentity {
    pod(|status, containers| {
        status == "Running"
            && !restarted(containers)
            && containers.default.as_deref() != Some("metrics-server")
    })
}

/// Opens the Logs tab on `identity`: the pane passes the pod, then its
/// containers once it has read them, and asks for the log.
fn show(view: &Entity<PodLogView>, identity: &ResourceIdentity, cx: &mut App) {
    let identity = identity.clone();
    let pod = containers(&identity);
    view.update(cx, |view, cx| {
        view.show_pod(Some(identity), Some(KubeAccess::Example), cx);
        view.want(cx);
        view.set_containers(pod, cx);
    });
}

/// Log lines held, not counting markers.
fn lines(view: &Entity<PodLogView>, cx: &App) -> usize {
    let entries = view.read(cx).review.logs.buffer().entries();
    entries.iter().filter(|entry| !entry.is_marker()).count()
}

fn markers(view: &Entity<PodLogView>, cx: &App) -> Vec<String> {
    let entries = view.read(cx).review.logs.buffer().entries();
    entries
        .iter()
        .filter(|entry| entry.is_marker())
        .map(|entry| entry.message.to_string())
        .collect()
}

fn state(view: &Entity<PodLogView>, cx: &App) -> StreamState {
    view.read(cx).source.state.clone()
}

/// The id of the newest row, which a following view shows.
fn last_row(view: &Entity<PodLogView>, cx: &App) -> String {
    let view = view.read(cx);
    let id = view.review.id(view.review.visible.len() - 1);
    format!("log-line-{}-{id}", view.generation)
}

#[gpui_kit::test]
fn the_log_waits_for_the_pod_then_reads_its_tail_and_follows_it(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let identity = running_pod();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.show_pod(Some(identity.clone()), Some(KubeAccess::Example), cx);
            view.want(cx);
        });
        window.render_frame(cx);
        // Nothing streams until the pane has read the pod's containers.
        assert_eq!(window.find("pod-logs-status").label(), Some("Reading"));
        assert_eq!(window.find("logs-empty").label(), Some("Reading the pod"));
        assert_eq!(state(&view, cx), StreamState::Idle);

        let pod = containers(&identity);
        let app = pod.default.clone().unwrap();
        view.update(cx, |view, cx| view.set_containers(pod, cx));
        window.render_frame(cx);
        assert_eq!(state(&view, cx), StreamState::Streaming);
        assert_eq!(window.find("pod-logs-status").label(), Some("Streaming"));
        assert_eq!(
            view.read(cx).source.container.as_deref(),
            Some(app.as_str())
        );
        assert_eq!(lines(&view, cx), 40);
        assert!(window.try_find("logs-empty").is_none());
        assert!(window.find(last_row(&view, cx)).visible());
        assert!(window.try_find("pod-logs-failed").is_none());
        assert!(window.try_find("pod-logs-hint").is_none());
    })
    .unwrap();

    // A running container writes on.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 2);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(lines(&view, cx), 42);
        assert!(window.find(last_row(&view, cx)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_container_that_has_not_started_says_so_with_nothing_to_show(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let identity = pod(|status, _| status == "Pending");
    cx.update_window(handle, |_, window, cx| {
        show(&view, &identity, cx);
        window.render_frame(cx);
        assert_eq!(state(&view, cx), StreamState::Waiting("Not started".into()));
        let app = view.read(cx).source.container.clone().unwrap();
        assert_eq!(
            window.find("logs-empty").label(),
            Some(format!("{app} hasn't started yet.").as_str())
        );
        let status = window.find("pod-logs-status").label().unwrap().to_owned();
        assert!(status.starts_with("Waiting: "), "{status}");
        assert_eq!(lines(&view, cx), 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_log_fails_with_retry_and_no_lines(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let identity = pod(|_, containers| containers.default.as_deref() == Some("metrics-server"));
    cx.update_window(handle, |_, window, cx| {
        show(&view, &identity, cx);
        window.render_frame(cx);
        assert!(matches!(state(&view, cx), StreamState::Failed(_)));
        assert_eq!(
            window
                .find("pod-logs-status")
                .label()
                .map(|label| label.starts_with("Failed: ")),
            Some(true)
        );
        let failed = window.find("pod-logs-failed").label().unwrap().to_owned();
        assert!(failed.contains("forbidden"), "{failed}");
        assert_eq!(window.find("logs-empty").label(), Some("Nothing was read."));

        // Retry reads again, and is refused again.
        let stream = view.read(cx).source.stream;
        window.click("pod-logs-retry", cx);
        assert!(view.read(cx).source.stream > stream);
        assert!(matches!(state(&view, cx), StreamState::Failed(_)));

        // Stop and Resume have nothing to do; Retry is the way on.
        let stream = view.read(cx).source.stream;
        window.click("pod-logs-stream", cx);
        assert_eq!(view.read(cx).source.stream, stream);
        assert_eq!(lines(&view, cx), 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_crash_loop_marks_the_restart_and_offers_the_previous_instance(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let identity = pod(|status, _| status == "CrashLoopBackOff");
    cx.update_window(handle, |_, window, cx| {
        show(&view, &identity, cx);
        window.render_frame(cx);
        let app = view.read(cx).source.container.clone().unwrap();
        // The instance that just ended, a marker, then the wait to start
        // again.
        assert_eq!(lines(&view, cx), 12);
        assert_eq!(
            markers(&view, cx),
            vec![format!(
                "{app} restarted (exit 1 (Error)), showing the new instance"
            )]
        );
        assert_eq!(
            state(&view, cx),
            StreamState::Waiting("CrashLoopBackOff".into())
        );
        let hint = window.find("pod-logs-hint").label().unwrap().to_owned();
        assert!(
            hint.starts_with(&format!(
                "{app} restarted 14 times, last exited 1 (Error) at "
            )),
            "{hint}"
        );
        // The marker row isn't a log line: copy and search pass over it.
        let marker = view.read(cx).review.id(12);
        let generation = view.read(cx).generation;
        assert!(
            window
                .find(format!("log-marker-{generation}-{marker}"))
                .visible()
        );

        // The app never switches by itself; the hint asks first.
        assert!(!view.read(cx).source.previous);
        window.click("pod-logs-show-previous", cx);
        window.render_frame(cx);
        assert!(view.read(cx).source.previous);
        assert_eq!(lines(&view, cx), 24);
        assert!(markers(&view, cx).is_empty());
        assert_eq!(state(&view, cx), StreamState::Ended(None));
        let status = window.find("pod-logs-status").label().unwrap().to_owned();
        assert!(
            status.starts_with("Previous instance: Previous instance · exited 1 (Error) at ")
                && status.ends_with(" · complete"),
            "{status}"
        );
        assert!(window.try_find("pod-logs-hint").is_none());

        // A log read to its end has nothing to stop or resume.
        let stream = view.read(cx).source.stream;
        window.click("pod-logs-stream", cx);
        assert_eq!(view.read(cx).source.stream, stream);

        // Unchecking goes back to the current instance, read afresh.
        window.click("pod-logs-previous", cx);
        assert!(!view.read(cx).source.previous);
        assert_eq!(lines(&view, cx), 12);
        assert_eq!(markers(&view, cx).len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn without_a_restart_there_is_no_previous_instance(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        show(&view, &running_pod(), cx);
        window.render_frame(cx);
        let stream = view.read(cx).source.stream;
        window.click("pod-logs-previous", cx);
        assert!(!view.read(cx).source.previous);
        assert_eq!(view.read(cx).source.stream, stream);
    })
    .unwrap();
}

#[gpui_kit::test]
fn stop_keeps_what_was_read_and_resume_reads_on_without_repeating(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        show(&view, &running_pod(), cx);
        window.render_frame(cx);
        window.click("pod-logs-stream", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, cx), StreamState::Stopped);
        assert_eq!(
            window.find("pod-logs-status").label(),
            Some("Stopped: Resume reads on from the last line.")
        );
        assert_eq!(lines(&view, cx), 40);
    })
    .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    // Example history is dated from the wall clock, so let a second pass:
    // reading on must not take it again for new lines.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(lines(&view, cx), 40);
        let generation = view.read(cx).generation;
        window.click("pod-logs-stream", cx);
        assert_eq!(state(&view, cx), StreamState::Streaming);
        // The same review, with nothing read twice.
        assert_eq!(view.read(cx).generation, generation);
        assert_eq!(lines(&view, cx), 40);
    })
    .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL);
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| assert_eq!(lines(&view, cx), 41))
        .unwrap();
}

#[gpui_kit::test]
fn the_time_column_hides_and_copy_copies_what_shows(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        show(&view, &running_pod(), cx);
        window.render_frame(cx);
        window.click(last_row(&view, cx), cx);
        window.press("secondary-c", cx);
        let copied = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        let (time, rest) = copied.split_once(' ').unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(time).is_ok(), "{time}");

        window.click("pod-logs-timestamps", cx);
        assert!(!view.read(cx).columns.time);
        window.render_frame(cx);
        // The selection stays; the copy leaves the time out.
        window.press("secondary-c", cx);
        let copied = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        assert_eq!(copied, rest);
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_init_container_reads_to_its_end_and_a_new_tail_starts_over(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let identity = pod(|status, containers| {
        status == "Running"
            && has_init(containers)
            && containers.default.as_deref() != Some("metrics-server")
    });
    cx.update_window(handle, |_, window, cx| {
        show(&view, &identity, cx);
        window.render_frame(cx);
        let choices = view.read(cx).source.choices.clone();
        assert_eq!(choices[0].role, ContainerRole::Init);
        assert_eq!(choices[0].label.as_ref(), "init-config · Completed");
        assert!(choices[0].enabled);
        assert_eq!(choices[1].role, ContainerRole::App);
        // The app container is the default.
        assert_eq!(
            view.read(cx).source.container.as_deref(),
            Some(choices[1].name.as_str())
        );

        let generation = view.read(cx).generation;
        view.update(cx, |view, cx| {
            view.choose_container("init-config".into(), cx)
        });
        window.render_frame(cx);
        assert!(view.read(cx).generation > generation);
        assert_eq!(lines(&view, cx), 3);
        assert!(matches!(state(&view, cx), StreamState::Ended(Some(_))));
        let status = window.find("pod-logs-status").label().unwrap().to_owned();
        assert!(
            status.starts_with("Ended: Stream ended: init-config exited 0 (Completed) at "),
            "{status}"
        );

        // Another tail reads the log again into a fresh review.
        let generation = view.read(cx).generation;
        view.update(cx, |view, cx| view.set_tail(Some(100), cx));
        assert!(view.read(cx).generation > generation);
        assert_eq!(view.read(cx).source.tail, Some(100));
        assert_eq!(lines(&view, cx), 3);
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_the_page_stops_the_stream_and_showing_it_reads_on(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    cx.update_window(handle, |_, _, cx| {
        show(&view, &running_pod(), cx);
        view.update(cx, |view, cx| view.set_active(false, cx));
        let source = &view.read(cx).source;
        assert!(source.job.is_none() && source.delivery.is_none());
        assert!(source.suspended && !source.running());
    })
    .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(lines(&view, cx), 40);
        let generation = view.read(cx).generation;
        view.update(cx, |view, cx| view.set_active(true, cx));
        assert_eq!(state(&view, cx), StreamState::Streaming);
        assert_eq!(view.read(cx).generation, generation);
        assert!(view.read(cx).source.delivery.is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_pod_or_none_cancels_the_stream_and_starts_over(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let first = running_pod();
    let crashing = pod(|status, _| status == "CrashLoopBackOff");
    cx.update_window(handle, |_, _, cx| {
        show(&view, &first, cx);
        view.update(cx, |view, cx| {
            view.set_tail(Some(1_000), cx);
            view.set_timestamps(false, cx);
        });
        let stream = view.read(cx).source.stream;

        view.update(cx, |view, cx| {
            view.show_pod(Some(crashing.clone()), None, cx)
        });
        let source = &view.read(cx).source;
        assert!(source.stream > stream);
        assert!(source.delivery.is_none());
        assert_eq!(source.state, StreamState::Idle);
        // Nothing streams for the new pod until it is wanted, and the
        // tail and time column carry over.
        assert!(!source.wanted);
        assert_eq!(source.tail, Some(1_000));
        assert!(!view.read(cx).columns.time);
        assert_eq!(lines(&view, cx), 0);

        view.update(cx, |view, cx| view.show_pod(None, None, cx));
        assert!(view.read(cx).source.pod.is_none());
    })
    .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| assert_eq!(lines(&view, cx), 0))
        .unwrap();
}
