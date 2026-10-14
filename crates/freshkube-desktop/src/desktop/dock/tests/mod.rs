use freshkube_core::resources::builtin;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px};

use super::saved::{SavedDock, SavedTab};
use super::{Dock, MAX_LOG_TABS, MAX_SHELL_TABS, TabKind};
use crate::desktop::tests::{fixture, start_shell};
use crate::desktop::{Page, Pilot};
use crate::logs::PodLogPanel;
use crate::logs::{PodLogView, WorkloadLogView};
use crate::resources::model::ResourceIdentity;
use crate::resources::shell::{self, ShellState, ShellView};
use crate::resources::{Tab, example, live};
use freshkube_ui::dock::{DEFAULT_HEIGHT, MIN_HEIGHT};

mod drawer;
mod keys;
mod layout;
mod logs;
mod saved;
mod shells;
mod source;
mod tabs;

/// The applied example context's objects of `key` that `pick` accepts.
fn objects(
    pilot: &Entity<Pilot>,
    key: &str,
    pick: impl Fn(&[String], &str) -> bool,
    cx: &mut TestAppContext,
) -> Vec<ResourceIdentity> {
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, key, None, live::now()).unwrap();
    rows.into_iter()
        .filter(|row| pick(&row.cells, &row.identity.name))
        .map(|row| row.identity)
        .collect()
}

fn running_pods(pilot: &Entity<Pilot>, cx: &mut TestAppContext) -> Vec<ResourceIdentity> {
    objects(pilot, "pods", |cells, _| cells[2] == "Running", cx)
}

/// Asks for `identity`'s logs as a link does, and lets them open.
fn open_logs(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    key: &str,
    identity: &ResourceIdentity,
    cx: &mut TestAppContext,
) {
    let identity = identity.clone();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin(key).unwrap(),
                identity.into(),
                Tab::Logs,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

fn draw(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn dock(pilot: &Entity<Pilot>, cx: &mut TestAppContext) -> Entity<Dock> {
    cx.update(|cx| pilot.read(cx).dock.clone())
}

fn titles(dock: &Entity<Dock>, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        dock.read(cx)
            .tabs
            .iter()
            .map(|tab| tab.title.to_string())
            .collect()
    })
}

fn pod_view(dock: &Entity<Dock>, ix: usize, cx: &mut TestAppContext) -> Entity<PodLogView> {
    cx.update(|cx| match &dock.read(cx).tabs[ix].kind {
        TabKind::Pod(view) => view.clone(),
        TabKind::Workload(_) | TabKind::Shell(_) => panic!("not a pod's log tab"),
    })
}

fn workload_view(
    dock: &Entity<Dock>,
    ix: usize,
    cx: &mut TestAppContext,
) -> Entity<WorkloadLogView> {
    cx.update(|cx| match &dock.read(cx).tabs[ix].kind {
        TabKind::Workload(view) => view.clone(),
        TabKind::Pod(_) | TabKind::Shell(_) => panic!("not a workload's log tab"),
    })
}

/// Lets the dock settle at its least height: the log and the dock measure
/// themselves as they draw and ask for a frame when that changes.
fn settle(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    for _ in 0..4 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

/// In a small window, the selected tab's notices and its log's controls
/// show whole above at least the list's least height, with no wheel: the
/// log's panel has nowhere to scroll.
fn assert_whole_without_scrolling(
    handle: AnyWindowHandle,
    view: &Entity<PodLogView>,
    notice: &'static str,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        let dock = window.find("dock").bounds();
        let panel = window.within("dock").find("logs-panel").bounds();
        let notice = window.within("dock").find(notice).bounds();
        assert!(
            notice.top() >= dock.top() && notice.bottom() <= panel.bottom(),
            "{notice:?} {dock:?}"
        );
        assert_eq!(view.read(cx).panel_max_offset(), px(0.));
        let lines = window.within("dock").find("logs-viewport").bounds();
        let least = window.rem_size() * freshkube_logs::LIST_LEAST_REMS;
        assert!(lines.size.height + px(1.) >= least, "{lines:?}");
        assert!(lines.top() >= notice.bottom(), "{lines:?} {notice:?}");
        assert!(
            lines.bottom() <= panel.bottom() + px(1.),
            "{lines:?} {panel:?}"
        );
        assert!(
            panel.bottom() <= dock.bottom() + px(1.),
            "{panel:?} {dock:?}"
        );
    })
    .unwrap();
}

/// Runs `change` on the shell as a live Talos session would, with the new
/// configuration still loading so nothing is contacted: the Kubernetes
/// source is gone until the new cluster answers.
fn while_loading(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    cx: &mut TestAppContext,
    change: impl FnOnce(&mut Pilot, &mut gpui_kit::Window, &mut gpui_kit::Context<Pilot>),
) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.mode.set_example(false);
            pilot.config_loading = true;
            change(pilot, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
}

/// The log lines drawn whole inside the dock's viewport.
fn whole_lines(
    handle: AnyWindowHandle,
    view: &Entity<PodLogView>,
    cx: &mut TestAppContext,
) -> usize {
    cx.update_window(handle, |_, window, cx| {
        let viewport = window.within("dock").find("logs-viewport").bounds();
        let view = view.read(cx);
        let generation = view.generation();
        (0..view.visible_rows().len())
            .filter(|&ix| {
                let id = format!("log-line-{generation}-{}", view.row_id(ix));
                window.try_find(id).is_some_and(|line| {
                    let line = line.bounds();
                    line.top() >= viewport.top() - px(0.5)
                        && line.bottom() <= viewport.bottom() + px(0.5)
                })
            })
            .count()
    })
    .unwrap()
}

/// In the smallest window, at `text_size`, L on `pod` in the pods list
/// opens its logs in the dock, and nothing in the log's panel scrolls to
/// reach its lines. When `row`, the row stays in sight above the dock and
/// at least three whole log lines show. At the largest text size the page
/// keeps only its least height, about its header, so no row can show, and
/// the log's long lines wrap: there the list keeps five rems.
fn l_keeps_the_row_above_three_lines(
    pod: &ResourceIdentity,
    text_size: f32,
    row: bool,
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let dock = dock(&pilot, cx);
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    while cx
        .update_window(handle, |_, window, _| window.rem_size())
        .unwrap()
        < px(text_size)
    {
        step(cx, &|window, cx| window.press("secondary-=", cx));
    }
    step(cx, &|window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
    });
    // The pane opens on the pod, and the keyboard goes back to the list,
    // where L opens the pod's logs in the dock.
    step(cx, &|window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                pod.clone().into(),
                Tab::Overview,
                window,
                cx,
            )
        });
    });
    step(cx, &|window, cx| window.press("escape", cx));
    step(cx, &|window, cx| {
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("l", cx);
    });
    settle(handle, cx);
    assert_eq!(titles(&dock, cx).len(), 1);
    assert_eq!(
        cx.update(|cx| dock.read(cx).tabs[0].target.identity.name.clone()),
        pod.name
    );
    let view = pod_view(&dock, 0, cx);
    cx.update_window(handle, |_, window, cx| {
        let cell = window.find("page-cell").bounds();
        let dock_bounds = window.find("dock").bounds();
        // An undragged dock takes at most half the page cell, unless the
        // tab's notes and three lines need more.
        let least = crate::ui::dp_px(dock.read(cx).least_height(window, cx), window);
        assert!(
            dock_bounds.size.height <= (cell.size.height / 2.).max(least) + px(1.),
            "{cell:?} {dock_bounds:?} {least:?}"
        );
        assert!(dock_bounds.bottom() <= cell.bottom() + px(1.));
        // Nothing in the log's panel scrolls to reach its lines.
        assert_eq!(view.read(cx).panel_max_offset(), px(0.));
        let panel = window.within("dock").find("logs-panel").bounds();
        assert!(panel.bottom() <= dock_bounds.bottom() + px(1.));
        if !row {
            let viewport = window.within("dock").find("logs-viewport").bounds();
            assert!(
                viewport.size.height >= window.rem_size() * 5.,
                "{viewport:?}"
            );
            return;
        }
        // The row the logs came from still shows whole above the dock.
        let row = window.find(format!(
            "resource-row:{}/{}/{}",
            pod.namespace, pod.name, pod.uid
        ));
        assert!(row.visible(), "{row:?}");
        let row = row.bounds();
        let body = window.find("resource-body").bounds();
        assert!(row.top() >= body.top().max(cell.top()), "{row:?} {body:?}");
        assert!(
            row.bottom() <= body.bottom().min(dock_bounds.top()),
            "{row:?} {dock_bounds:?}"
        );
    })
    .unwrap();
    if row {
        let lines = whole_lines(handle, &view, cx);
        assert!(lines >= 3, "{lines} whole lines");
    }
}

/// The crash-looping pod lowest in the list, so the list scrolls to keep
/// it in sight above the dock.
fn lowest_crash_looping_pod(pilot: &Entity<Pilot>, cx: &mut TestAppContext) -> ResourceIdentity {
    objects(pilot, "pods", |cells, _| cells[2] == "CrashLoopBackOff", cx)
        .pop()
        .unwrap()
}

/// A pod tab's own tools.
const POD_TOOLS: &[&str] = &[
    "pod-logs-container",
    "pod-logs-tail",
    "pod-logs-previous",
    "pod-logs-timestamps",
    "pod-logs-stream",
    "pod-logs-status",
];

/// A workload tab's own tools.
const WORKLOAD_TOOLS: &[&str] = &["workload-logs-pod", "workload-logs-timestamps"];

/// How many rows the toolbar takes in the dock, with the source's `tools`
/// first: the distinct middles of its controls.
fn tool_rows(handle: AnyWindowHandle, tools: &[&'static str], cx: &mut TestAppContext) -> usize {
    cx.update_window(handle, |_, window, _| {
        let mut middles: Vec<f32> = Vec::new();
        for &id in tools.iter().chain(&[
            "logs-levels",
            "logs-search",
            "logs-wrap",
            "logs-follow",
            "logs-copy",
            "logs-download",
        ]) {
            let bounds = window.within("dock").find(id).bounds();
            let middle = f32::from(bounds.center().y);
            if !middles.iter().any(|seen| (seen - middle).abs() < 8.) {
                middles.push(middle);
            }
        }
        middles.len()
    })
    .unwrap()
}

fn shell_view_of(dock: &Dock, ix: usize) -> Entity<ShellView> {
    match &dock.tabs[ix].kind {
        TabKind::Shell(view) => view.clone(),
        _ => panic!("not a shell tab"),
    }
}

fn shell_view(dock: &Entity<Dock>, ix: usize, cx: &mut TestAppContext) -> Entity<ShellView> {
    cx.update(|cx| match &dock.read(cx).tabs[ix].kind {
        TabKind::Shell(view) => view.clone(),
        _ => panic!("not a shell tab"),
    })
}

fn shell_state(dock: &Entity<Dock>, ix: usize, cx: &mut TestAppContext) -> ShellState {
    let view = shell_view(dock, ix, cx);
    cx.update(|cx| view.read(cx).state.clone())
}

/// Opens `pod` in the pane and picks its first running container from
/// the Shell menu, as the user does.
fn pick_shell(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    pod: &ResourceIdentity,
    cx: &mut TestAppContext,
) {
    let target = pod.clone();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                target.into(),
                Tab::Overview,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    // Kit's first Down lands on the first entry, a heading when the pod's
    // containers have several roles.
    let containers = example::document(pod, live::now())
        .unwrap()
        .overview
        .pod
        .unwrap();
    let choices = shell::choices(&containers);
    let mixed = choices.iter().any(|choice| choice.role != choices[0].role);
    let keys: &[&str] = if mixed {
        &["", "down", "down", "enter"]
    } else {
        &["", "down", "enter"]
    };
    for key in keys {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            if key.is_empty() {
                window.click("detail-open-shell", cx);
            } else {
                window.press(key, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }
    draw(handle, cx);
}

/// A pod's identity and containers as its watch reports them.
fn watched(pod: &ResourceIdentity) -> freshkube_core::resources::WorkloadPods {
    let containers = example::document(pod, live::now())
        .unwrap()
        .overview
        .pod
        .unwrap();
    freshkube_core::resources::WorkloadPods {
        listed: true,
        pods: vec![freshkube_core::resources::WorkloadPod {
            name: pod.name.clone(),
            uid: pod.uid.clone(),
            created: None,
            terminating: false,
            containers,
        }],
        failure: None,
    }
}

/// Whether the Resources drawer is drawn, and the row the list selects.
fn drawer(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    cx: &mut TestAppContext,
) -> (bool, Option<ResourceIdentity>) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let open = window.try_find("resource-drawer").is_some();
        let selected = pilot.read(cx).resources.read(cx).selected_row().cloned();
        (open, selected)
    })
    .unwrap()
}
