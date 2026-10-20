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
    StreamKey, StreamReads, StreamState, Streams, WorkloadLogPanel, WorkloadLogView, WorkloadLogs,
    forward_listings, short_labels,
};
use crate::desktop::probe;
use crate::resources::model::ResourceIdentity;
use crate::resources::{ClusterAccess, example, live};

mod chips;
mod export;
mod lifecycle;
mod pods;
mod reads;
mod retry;
mod selection;
mod streams;

const CONTEXT: &str = "production";

/// The view on its own, active and on screen, as the Logs tab shows it.
pub(super) fn mount(
    cx: &mut TestAppContext,
) -> (Runtime, Entity<WorkloadLogView>, AnyWindowHandle) {
    mount_sized(cx, 900., 820.)
}

pub(super) fn mount_sized(
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
pub(super) fn deployment(app: &str) -> ResourceIdentity {
    let (_, rows) = example::read(CONTEXT, "deployments.apps", None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| row.identity.name == app)
        .unwrap()
        .identity
}

/// Its selector, as the pane reads it from the document.
pub(super) fn selector(workload: &ResourceIdentity) -> Option<String> {
    example::document(workload, live::now())
        .unwrap()
        .overview
        .selector
        .unwrap()
}

pub(super) fn pods(workload: &ResourceIdentity, selector: &str) -> Vec<WorkloadPod> {
    example::workload_pods(workload, selector, live::now())
}

/// Opens the Logs tab on `workload` with its selector, as the pane does.
pub(super) fn show(view: &Entity<WorkloadLogView>, workload: &ResourceIdentity, cx: &mut App) {
    let selector = selector(workload);
    view.update(cx, |view, cx| {
        view.show_workload(Some(workload.clone()), Some(ClusterAccess::EXAMPLE), cx);
        view.want(cx);
        view.set_selector(selector, cx);
    });
}

/// Opens the Logs tab with no selector known, so the test feeds the pods.
pub(super) fn show_fed(view: &Entity<WorkloadLogView>, workload: &ResourceIdentity, cx: &mut App) {
    view.update(cx, |view, cx| {
        view.show_workload(Some(workload.clone()), Some(ClusterAccess::EXAMPLE), cx);
        view.want(cx);
    });
}

pub(super) fn feed(view: &Entity<WorkloadLogView>, pods: Vec<WorkloadPod>, cx: &mut App) {
    view.update(cx, |view, cx| {
        view.apply_pods(
            WorkloadPods {
                listed: true,
                pods,
                failure: None,
            },
            cx,
        );
    });
}

pub(super) fn status(window: &mut gpui_kit::Window) -> String {
    window
        .find("workload-logs-status")
        .label()
        .unwrap()
        .to_owned()
}

/// Log lines held, not counting markers.
pub(super) fn lines(view: &Entity<WorkloadLogView>, cx: &App) -> usize {
    let entries = view.read(cx).retained();
    entries.iter().filter(|entry| !entry.is_marker()).count()
}

pub(super) fn markers(view: &Entity<WorkloadLogView>, cx: &App) -> Vec<String> {
    let entries = view.read(cx).retained();
    entries
        .iter()
        .filter(|entry| entry.is_marker())
        .map(|entry| entry.message.to_string())
        .collect()
}

/// Every tag a line carries.
pub(super) fn tags(view: &Entity<WorkloadLogView>, cx: &App) -> Vec<String> {
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

pub(super) fn key(pod: &WorkloadPod) -> StreamKey {
    StreamKey {
        pod: pod.name.clone(),
        uid: pod.uid.clone(),
        container: pod.containers.default.clone().unwrap(),
    }
}

/// The tags of the lines the list shows.
pub(super) fn shown_tags(
    view: &Entity<WorkloadLogView>,
    cx: &App,
) -> std::collections::BTreeSet<String> {
    let view = view.read(cx);
    view.visible_rows()
        .iter()
        .map(|&ix| view.retained()[ix].service.as_str().to_owned())
        .collect()
}
