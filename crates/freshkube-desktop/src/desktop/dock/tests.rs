use freshkube_core::resources::builtin;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px};

use super::saved::{SavedDock, SavedTab};
use super::{Dock, MAX_LOG_TABS, TabKind};
use crate::desktop::tests::fixture;
use crate::desktop::{Page, Pilot};
use crate::logs::PodLogPanel;
use crate::logs::PodLogView;
use crate::resources::model::ResourceIdentity;
use crate::resources::{Tab, example, live};
use freshkube_ui::dock::{DEFAULT_HEIGHT, MIN_HEIGHT};

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
        TabKind::Workload(_) => panic!("a workload's tab"),
    })
}

#[gpui_kit::test]
fn logs_open_in_a_tab_and_opening_them_again_selects_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let api = objects(&pilot, "deployments.apps", |_, name| name == "api", cx);
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dock").is_none(), "no tabs, no dock");
    })
    .unwrap();

    open_logs(handle, &pilot, "pods", &pods[0], cx);
    assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[0].name)]);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.find("dock-tab-0").selected(), Some(true));
        assert_eq!(window.find("pod-logs-status").label(), Some("Streaming"));
        // The tab takes the keyboard, and the status bar counts it.
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
        assert_eq!(window.find("dock-toggle").label(), Some("1 log"));
        let _ = cx;
    })
    .unwrap();

    // A workload follows every pod it runs, in a tab of its own.
    open_logs(handle, &pilot, "deployments.apps", &api[0], cx);
    assert_eq!(
        titles(&dock, cx),
        [format!("Pod {}", pods[0].name), "Deployment api".into()]
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("dock-tab-1").selected(), Some(true));
        let status = window.find("workload-logs-status");
        let status = status.label().unwrap();
        assert!(status.starts_with("Following: "), "{status}");
    })
    .unwrap();

    // The same pod again selects its tab.
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(0));
}

#[gpui_kit::test]
fn a_tab_reads_across_pages_and_objects_until_it_closes(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));

    // Another object in the pane, and another page, keep it.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                pods[1].clone().into(),
                Tab::Overview,
                window,
                cx,
            );
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("dock").visible(), "the dock spans every page");
    })
    .unwrap();

    // Hiding keeps it; closing the tab drops it.
    cx.update_window(handle, |_, window, cx| window.click("dock-hide", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("dock").is_none());
        window.click("dock-toggle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    let weak = view.downgrade();
    drop(view);
    cx.update_window(handle, |_, window, cx| window.click("dock-tab-0-close", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the closed tab's view is dropped");
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dock").is_none());
        assert!(window.try_find("dock-toggle").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_ninth_log_tab_asks_to_close_the_oldest(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = objects(&pilot, "pods", |_, _| true, cx);
    assert!(pods.len() > MAX_LOG_TABS + 1);
    for pod in &pods[..MAX_LOG_TABS] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    assert_eq!(titles(&dock, cx).len(), MAX_LOG_TABS);
    assert!(!cx.has_pending_prompt());

    // Cancel opens nothing.
    open_logs(handle, &pilot, "pods", &pods[MAX_LOG_TABS], cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx)[0], format!("Pod {}", pods[0].name));

    // Agreeing closes the oldest and opens the new one.
    open_logs(handle, &pilot, "pods", &pods[MAX_LOG_TABS], cx);
    cx.simulate_prompt_answer("Close it");
    cx.run_until_parked();
    let titles = titles(&dock, cx);
    assert_eq!(titles.len(), MAX_LOG_TABS);
    assert_eq!(titles[0], format!("Pod {}", pods[1].name));
    assert_eq!(
        titles.last().unwrap(),
        &format!("Pod {}", pods[MAX_LOG_TABS].name)
    );
}

#[gpui_kit::test]
fn keys_switch_close_and_minimize_tabs_and_escape_hands_the_keyboard_back(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    for pod in &pods[..3] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    let press = |key: &str, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        draw(handle, cx);
    };
    let selected = |cx: &mut TestAppContext| cx.update(|cx| dock.read(cx).selected());
    assert_eq!(selected(cx), Some(2));
    press("ctrl-.", cx);
    assert_eq!(selected(cx), Some(0), "wraps round");
    press("ctrl-,", cx);
    assert_eq!(selected(cx), Some(2));
    press("ctrl-,", cx);
    assert_eq!(selected(cx), Some(1));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
    })
    .unwrap();

    // Command-W closes the selected tab; the next one takes its place.
    press(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-shift-w"
        },
        cx,
    );
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(selected(cx), Some(2));

    // Escape with nothing to clear hands the keyboard back to the list.
    press("escape", cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(!dock.read(cx).has_focus(window, cx));
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();

    // Shift-Escape minimizes to the tabs, from anywhere.
    press("shift-escape", cx);
    assert!(cx.update(|cx| !dock.read(cx).is_open()));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("dock-bar").visible());
        assert!(window.try_find("dock-body").is_none());
    })
    .unwrap();
    // Control-. brings it back on the tab it had.
    press("ctrl-.", cx);
    assert!(cx.update(|cx| dock.read(cx).is_open()));
    assert_eq!(selected(cx), Some(2));
}

#[gpui_kit::test]
fn command_f_searches_the_logs_in_the_dock_and_the_yaml_in_the_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert!(window.find("logs-search").focused().unwrap_or(false));
        assert_eq!(
            pilot.read(cx).resources.read(cx).detail_tab(cx),
            Tab::Overview
        );
        // From the pane, it finds in the YAML.
        window.click("detail-tab-overview", cx);
        window.render_frame(cx);
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).resources.read(cx).detail_tab(cx), Tab::Yaml);
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_in_the_dock_clears_the_search_leaves_it_then_returns_to_the_list(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let step = |cx: &mut TestAppContext, key: &str, typed: Option<&str>| {
        cx.update_window(handle, |_, window, cx| {
            window.press(key, cx);
            if let Some(text) = typed {
                window.input(text, cx);
            }
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let focused = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused().unwrap_or(false)
        })
        .unwrap()
    };
    step(cx, "secondary-f", Some("GET"));
    assert!(focused(cx, "logs-search"));
    // The first Escape clears the search and stays in it.
    step(cx, "escape", None);
    assert!(focused(cx, "logs-search"));
    // The next leaves it for the lines.
    step(cx, "escape", None);
    assert!(focused(cx, "logs-viewport"));
    // With nothing selected, the keyboard goes back to the list.
    step(cx, "escape", None);
    assert!(focused(cx, "resource-body"));
    assert!(cx.update(|cx| pilot.read(cx).dock.read(cx).has_tabs()));
}

#[gpui_kit::test]
fn the_dock_resizes_minimizes_below_its_least_and_fits_the_window(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    // The dock spans the page cell under the page.
    cx.update_window(handle, |_, window, _| {
        let cell = window.find("page-cell").bounds();
        let bounds = window.find("dock").bounds();
        assert!((bounds.left() - cell.left()).abs() <= px(1.), "{bounds:?}");
        assert!((bounds.right() - cell.right()).abs() <= px(1.));
        assert!((bounds.bottom() - cell.bottom()).abs() <= px(1.));
        let tall = crate::ui::dp_px(DEFAULT_HEIGHT, window);
        assert!((bounds.size.height - tall).abs() <= px(1.), "{bounds:?}");
    })
    .unwrap();

    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(420., window, cx))
    })
    .unwrap();
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
    // Below the least height it minimizes, keeping the dragged height.
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(MIN_HEIGHT - 10., window, cx))
    })
    .unwrap();
    assert!(cx.update(|cx| !dock.read(cx).is_open()));
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        let bounds = window.find("dock").bounds();
        let bar = crate::ui::dp_px(freshkube_ui::dock::BAR_HEIGHT, window);
        assert!(bounds.size.height <= bar + px(2.), "{bounds:?}");
    })
    .unwrap();

    // Fit to window fills the page cell; Restore brings the height back.
    cx.update_window(handle, |_, window, cx| window.click("dock-maximize", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| dock.read(cx).is_maximized()));
    cx.update_window(handle, |_, window, cx| {
        let cell = window.find("page-cell").bounds();
        let bounds = window.find("dock").bounds();
        assert!(
            (bounds.size.height - cell.size.height).abs() <= px(1.),
            "{bounds:?}"
        );
        window.click("dock-maximize", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| !dock.read(cx).is_maximized()));
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
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

#[gpui_kit::test]
fn a_crash_looping_pod_shows_its_notice_and_three_lines_in_a_small_window(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let dock = dock(&pilot, cx);
    let crashing = objects(
        &pilot,
        "pods",
        |cells, _| cells[2] == "CrashLoopBackOff",
        cx,
    );
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(MIN_HEIGHT, window, cx))
    })
    .unwrap();
    open_logs(handle, &pilot, "pods", &crashing[0], cx);
    settle(handle, cx);
    let view = pod_view(&dock, 0, cx);
    assert_whole_without_scrolling(handle, &view, "pod-logs-hint", cx);
}

#[gpui_kit::test]
fn a_gone_pods_notice_counts_in_the_docks_least_height(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let dock = dock(&pilot, cx);
    let mut gone = running_pods(&pilot, cx).remove(0);
    gone.name = format!("{}-gone", gone.name);
    gone.uid = String::new();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(MIN_HEIGHT, window, cx))
    })
    .unwrap();
    // No pane opens an object that isn't there; ask the dock as a link
    // to a pod that has since gone would.
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            let target = crate::resources::detail::DetailTarget {
                identity: gone,
                kind: builtin("pods").unwrap(),
            };
            dock.open_logs(
                crate::resources::LogsRequest { target, at: None },
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    settle(handle, cx);
    let view = pod_view(&dock, 0, cx);
    assert_whole_without_scrolling(handle, &view, "dock-tab-notice", cx);
    // Settled: another frame changes nothing.
    let height = cx
        .update_window(handle, |_, window, _| window.find("dock").bounds())
        .unwrap();
    settle(handle, cx);
    let again = cx
        .update_window(handle, |_, window, _| window.find("dock").bounds())
        .unwrap();
    assert_eq!(height, again);
}

#[gpui_kit::test]
fn another_context_closes_every_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx).downgrade();
    let other = cx.update(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|context| Some(*context) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_context(other, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(view.upgrade().is_none());
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
            pilot.fixture = false;
            pilot.config_loading = true;
            change(pilot, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn another_talos_context_closes_every_tab_while_it_loads(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx).downgrade();
    let other = cx.update(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|context| Some(*context) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    });
    while_loading(handle, &pilot, cx, |pilot, window, cx| {
        pilot.select_context(other, window, cx);
        assert!(pilot.kube_source().is_none());
    });
    assert!(titles(&dock, cx).is_empty());
    assert!(view.upgrade().is_none());
}

#[gpui_kit::test]
fn another_kubeconfig_closes_every_tab_while_it_loads(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    std::fs::write(&path, "apiVersion: v1\nkind: Config\n").unwrap();
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx).downgrade();
    while_loading(handle, &pilot, cx, |pilot, window, cx| {
        let selection = KubeconfigSelection::File {
            path: path.clone(),
            context: None,
        };
        pilot.apply_kubeconfig(selection, window, cx);
        assert!(pilot.kube_source().is_none());
    });
    assert!(titles(&dock, cx).is_empty());
    assert!(view.upgrade().is_none());
}

#[gpui_kit::test]
fn losing_the_source_closes_every_tab_and_its_return_reopens_none(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let source = cx.update(|cx| pilot.read(cx).kube_source());
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            dock.set_source(None, window, cx);
            assert!(dock.tabs.is_empty());
            // The same connection back opens nothing it had.
            dock.set_source(source.clone(), window, cx);
            assert!(dock.tabs.is_empty());
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn saved_tabs_come_back_for_their_context_and_read_once_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved_tab = |pod: &ResourceIdentity, context: &str| SavedTab {
        kind: "pods".into(),
        context: context.into(),
        namespace: pod.namespace.clone(),
        name: pod.name.clone(),
        container: None,
        previous: false,
    };
    let saved = SavedDock {
        height: 260.,
        open: false,
        maximized: false,
        hidden: false,
        selected: Some(1),
        tabs: vec![
            saved_tab(&pods[0], &context),
            saved_tab(&pods[1], &context),
            saved_tab(&pods[2], "another-context"),
        ],
    };
    // As the app opens: the saved dock waits for the first connection.
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).kube_source();
        dock.update(cx, |dock, cx| {
            dock.source = None;
            dock.height = saved.height;
            dock.open = saved.open;
            dock.restore = Some(saved);
            dock.set_source(source, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(
        titles(&dock, cx),
        [
            format!("Pod {}", pods[0].name),
            format!("Pod {}", pods[1].name)
        ]
    );
    // Minimized, nothing reads.
    let started = |cx: &mut TestAppContext| -> Vec<bool> {
        cx.update(|cx| dock.read(cx).tabs.iter().map(|tab| tab.started).collect())
    };
    assert_eq!(started(cx), [false, false]);
    let first = pod_view(&dock, 0, cx);
    assert!(!cx.update(|cx| first.read(cx).streaming()));

    // Opening shows the selected tab, which reads; the other waits.
    cx.update_window(handle, |_, window, cx| window.click("dock-minimize", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(started(cx), [false, true]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("dock-tab-0", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started(cx), [true, true]);
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 260.);
}

#[gpui_kit::test]
fn a_gone_pod_stops_reading_and_one_made_again_with_its_name_takes_a_new_tab(
    cx: &mut TestAppContext,
) {
    use crate::logs::PodLogPanel as _;
    use freshkube_core::resources::WorkloadPods;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pod = running_pods(&pilot, cx).remove(0);
    open_logs(handle, &pilot, "pods", &pod, cx);
    let view = pod_view(&dock, 0, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    // The watch's next list no longer has it.
    cx.update(|cx| {
        dock.update(cx, |dock, cx| {
            let id = dock.tabs[0].id;
            let pods = WorkloadPods {
                listed: true,
                ..WorkloadPods::default()
            };
            assert!(!dock.apply_pod(id, pods, cx), "the delivery ends");
        })
    });
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).streaming()));
    // Showing it again reads nothing by its name.
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            dock.set_open(false, window, cx);
            dock.set_open(true, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).streaming()));
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.within("dock").find("dock-tab-notice").visible());
    })
    .unwrap();
    // The same name with another UID is another pod.
    let mut again = pod.clone();
    again.uid = "00000000-0000-4000-8000-000000000099".into();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            let target = crate::resources::detail::DetailTarget {
                identity: again,
                kind: builtin("pods").unwrap(),
            };
            dock.open_logs(
                crate::resources::LogsRequest { target, at: None },
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        titles(&dock, cx),
        [format!("Pod {}", pod.name), format!("Pod {} (2)", pod.name)]
    );
}
