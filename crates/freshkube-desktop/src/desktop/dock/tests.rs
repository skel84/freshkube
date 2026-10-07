use freshkube_core::resources::builtin;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px};

use super::saved::{SavedDock, SavedTab};
use super::{Dock, MAX_LOG_TABS, MAX_SHELL_TABS, TabKind};
use crate::desktop::tests::{fixture, start_shell};
use crate::desktop::{Page, Pilot};
use crate::logs::PodLogPanel;
use crate::logs::PodLogView;
use crate::resources::model::ResourceIdentity;
use crate::resources::shell::{self, ShellState, ShellView};
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
        TabKind::Workload(_) | TabKind::Shell(_) => panic!("not a pod's log tab"),
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

    // The status bar minimizes it to its tabs, which keep reading, and
    // opens it again.
    cx.update_window(handle, |_, window, cx| window.click("dock-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    cx.update_window(handle, |_, window, cx| {
        assert!(
            window.find("dock-tab-0").visible(),
            "minimized, its tabs show"
        );
        assert!(window.try_find("dock-tab-body").is_none());
        window.click("dock-toggle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    // A middle-click closes a tab and drops its stream.
    let weak = view.downgrade();
    drop(view);
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::InputEvent as _;
        let position = window.find("dock-tab-0").bounds().center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Middle,
                position,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
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
    assert_whole_without_scrolling(handle, &view, "pod-logs-note", cx);
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
        shell: false,
    };
    let saved = SavedDock {
        height: 260.,
        open: false,
        maximized: false,
        // The second of this context's tabs, counted among all of them.
        selected: Some(2),
        tabs: vec![
            saved_tab(&pods[2], "another-context"),
            saved_tab(&pods[0], &context),
            saved_tab(&pods[1], &context),
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

#[gpui_kit::test]
fn a_pod_tab_asked_for_by_name_keeps_the_first_pod_it_finds(cx: &mut TestAppContext) {
    use freshkube_core::resources::{WorkloadPod, WorkloadPods};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let mut pod = running_pods(&pilot, cx).remove(0);
    let uid = std::mem::take(&mut pod.uid);
    open_logs(handle, &pilot, "pods", &pod, cx);
    let listed = |uid: &str| WorkloadPods {
        listed: true,
        pods: vec![WorkloadPod {
            name: pod.name.clone(),
            uid: uid.to_owned(),
            ..WorkloadPod::default()
        }],
        ..WorkloadPods::default()
    };
    cx.update(|cx| {
        dock.update(cx, |dock, cx| {
            let id = dock.tabs[0].id;
            assert!(dock.apply_pod(id, listed(&uid), cx));
            assert_eq!(dock.tabs[0].target.identity.uid, uid);
            // The pod is made again with its name: the tab's pod is gone.
            assert!(!dock.apply_pod(id, listed("00000000-0000-4000-8000-000000000099"), cx));
            assert_eq!(dock.tabs[0].feed.state, super::feed::FeedState::Gone);
        })
    });
    // The new pod takes a tab of its own.
    pod.uid = "00000000-0000-4000-8000-000000000099".into();
    open_logs(handle, &pilot, "pods", &pod, cx);
    assert_eq!(titles(&dock, cx).len(), 2);
}

/// A pod asked for by name that is already gone at its first listing
/// never takes a later pod of the name.
#[gpui_kit::test]
fn a_pod_tab_gone_at_its_first_listing_keeps_no_later_pod(cx: &mut TestAppContext) {
    use crate::resources::LogsRequest;
    use crate::resources::detail::DetailTarget;
    use freshkube_core::resources::WorkloadPods;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let mut pod = running_pods(&pilot, cx).remove(0);
    let open = |identity: ResourceIdentity, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            dock.update(cx, |dock, cx| {
                let target = DetailTarget {
                    identity,
                    kind: builtin("pods").unwrap(),
                };
                dock.open_logs(LogsRequest { target, at: None }, window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
    };
    // By name alone, as a restored tab is.
    pod.uid.clear();
    open(pod.clone(), cx);
    cx.update(|cx| {
        dock.update(cx, |dock, cx| {
            assert!(dock.tabs[0].key.wildcard);
            let id = dock.tabs[0].id;
            let none = WorkloadPods {
                listed: true,
                ..WorkloadPods::default()
            };
            assert!(!dock.apply_pod(id, none, cx));
            assert_eq!(dock.tabs[0].feed.state, super::feed::FeedState::Gone);
        })
    });
    // A pod made with the name later takes a tab of its own.
    pod.uid = "00000000-0000-4000-8000-000000000099".into();
    open(pod, cx);
    assert_eq!(titles(&dock, cx).len(), 2);
}

#[gpui_kit::test]
fn the_chromes_close_closes_every_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    open_logs(handle, &pilot, "pods", &pods[1], cx);
    let views = [
        pod_view(&dock, 0, cx).downgrade(),
        pod_view(&dock, 1, cx).downgrade(),
    ];
    cx.update_window(handle, |_, window, cx| window.click("dock-close-all", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(views.iter().all(|view| view.upgrade().is_none()));
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dock").is_none());
        assert!(window.try_find("dock-toggle").is_none());
        // The keyboard is back on the page.
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn without_tabs_the_docks_keys_go_on(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let handled = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            ["ctrl-.", "ctrl-,", "shift-escape"]
                .map(|key| window.dispatch_keystroke(gpui_kit::Keystroke::parse(key).unwrap(), cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(handled, [false; 3], "nothing took them");
    assert!(
        cx.update(|cx| dock.read(cx).is_open()),
        "nothing was minimized"
    );
}

#[gpui_kit::test]
fn leaving_the_dock_after_another_page_puts_the_keyboard_on_that_page(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    // The list had the keyboard when the dock took it.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| pilot.focus_page(window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(true));
        let target = crate::resources::detail::DetailTarget {
            identity: pods[0].clone(),
            kind: builtin("pods").unwrap(),
        };
        dock.update(cx, |dock, cx| {
            dock.open_logs(
                crate::resources::LogsRequest { target, at: None },
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(dock.read(cx).has_focus(window, cx));
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        // A click in the lines takes the keyboard without the dock
        // remembering anew: it still holds the list.
        dock.update(cx, |dock, cx| dock.tabs[0].focus_lines(window, cx));
        window.render_frame(cx);
        assert!(dock.read(cx).has_focus(window, cx));
        dock.update(cx, |dock, cx| dock.leave(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Not the list, which Overview doesn't draw: the page.
        assert!(window.try_find("resource-body").is_none());
        assert!(pilot.read(cx).focus.contains_focused(window, cx));
        assert!(!dock.read(cx).has_focus(window, cx));
    })
    .unwrap();
}

#[test]
fn the_saved_dock_reads_back_what_it_wrote_and_fills_in_what_is_missing() {
    let saved = SavedDock {
        height: 260.,
        open: false,
        maximized: true,
        selected: Some(1),
        tabs: vec![SavedTab {
            kind: "pods".into(),
            context: "lab".into(),
            namespace: "payments".into(),
            name: "api-0".into(),
            container: Some("api".into()),
            previous: true,
            shell: false,
        }],
    };
    let json = serde_json::to_string(&saved).unwrap();
    assert_eq!(serde_json::from_str::<SavedDock>(&json).unwrap(), saved);
    // An empty dock, or one from a build that saved more, still reads.
    let empty: SavedDock = serde_json::from_str("{}").unwrap();
    assert_eq!(
        (empty.height, empty.open, empty.tabs.len()),
        (DEFAULT_HEIGHT, true, 0)
    );
    let older: SavedDock = serde_json::from_str(r#"{"hidden":true,"height":120}"#).unwrap();
    assert_eq!(older.height, 120.);
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

#[gpui_kit::test]
fn l_in_the_smallest_window_keeps_a_running_pods_row_above_three_lines(cx: &mut TestAppContext) {
    let (_runtime, _, pilot) = fixture(cx, 760., 560.);
    // Low in the list, among the healthy pods.
    let pod = running_pods(&pilot, cx).remove(0);
    l_keeps_the_row_above_three_lines(&pod, 13., true, cx);
}

#[gpui_kit::test]
fn l_in_the_smallest_window_keeps_a_crash_looping_pods_row_above_three_lines(
    cx: &mut TestAppContext,
) {
    let (_runtime, _, pilot) = fixture(cx, 760., 560.);
    let pod = lowest_crash_looping_pod(&pilot, cx);
    l_keeps_the_row_above_three_lines(&pod, 13., true, cx);
}

#[gpui_kit::test]
fn l_at_the_largest_text_size_opens_a_log_whose_panel_doesnt_scroll(cx: &mut TestAppContext) {
    let (_runtime, _, pilot) = fixture(cx, 760., 560.);
    let pod = lowest_crash_looping_pod(&pilot, cx);
    l_keeps_the_row_above_three_lines(&pod, 20., false, cx);
}

/// How many rows the toolbar's tools take in the dock: the distinct
/// middles of its controls.
fn tool_rows(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    cx.update_window(handle, |_, window, _| {
        let mut middles: Vec<f32> = Vec::new();
        for id in [
            "pod-logs-container",
            "pod-logs-tail",
            "pod-logs-previous",
            "pod-logs-timestamps",
            "pod-logs-stream",
            "pod-logs-status",
            "logs-levels",
            "logs-search",
            "logs-wrap",
            "logs-follow",
            "logs-copy",
        ] {
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

#[gpui_kit::test]
fn the_toolbar_is_one_row_wide_and_two_narrow_or_large(cx: &mut TestAppContext) {
    for (width, height, text_size, most) in [
        (1280., 880., 13., 1),
        (760., 560., 13., 2),
        (760., 560., 20., 2),
    ] {
        let (_runtime, handle, pilot) = fixture(cx, width, height);
        while cx
            .update_window(handle, |_, window, _| window.rem_size())
            .unwrap()
            < px(text_size)
        {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.press("secondary-=", cx)
            })
            .unwrap();
        }
        let pod = lowest_crash_looping_pod(&pilot, cx);
        open_logs(handle, &pilot, "pods", &pod, cx);
        settle(handle, cx);
        let rows = tool_rows(handle, cx);
        assert!(rows <= most, "{width}×{height} at {text_size}: {rows} rows");
        // Previous keeps its label in a compact toolbar too, where the
        // crash-loop note points to it: wider than an icon alone.
        cx.update_window(handle, |_, window, _| {
            let mut width = |id| window.within("dock").find(id).bounds().size.width;
            let (previous, icon) = (width("pod-logs-previous"), width("pod-logs-timestamps"));
            assert!(
                previous > icon * 2.,
                "{text_size}: Previous {previous:?} beside an icon {icon:?}"
            );
            // Levels keeps room for its icon and its chevron.
            let levels = width("logs-levels");
            assert!(
                levels > icon * 1.5,
                "{text_size}: Levels {levels:?} beside an icon {icon:?}"
            );
        })
        .unwrap();
    }
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

#[gpui_kit::test]
fn a_pick_from_the_shell_menu_starts_a_shell_in_a_tab_of_its_own(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                pods[0].clone().into(),
                Tab::Overview,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    // Opening the menu runs nothing.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-open-shell", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(cx.read(shell::running_anywhere).is_empty());
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();

    pick_shell(handle, &pilot, &pods[0], cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pods[0].name)]);
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("terminal").focused(), Some(true));
        assert_eq!(window.find("dock-toggle").label(), Some("1 shell"));
    })
    .unwrap();

    // Another pod's shell takes another tab, and a log tab counts apart.
    pick_shell(handle, &pilot, &pods[1], cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    assert_eq!(
        titles(&dock, cx),
        [
            format!("Shell {}", pods[0].name),
            format!("Shell {}", pods[1].name),
            format!("Pod {}", pods[0].name),
        ]
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("dock-toggle").label(), Some("1 log · 2 shells"));
    })
    .unwrap();
    assert_eq!(cx.read(shell::running_anywhere).len(), 2);

    // The same container again selects its tab, and starts again a shell
    // that ended.
    let first = shell_view(&dock, 0, cx);
    cx.update(|cx| first.update(cx, |view, cx| view.end(cx)));
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Ended);
    pick_shell(handle, &pilot, &pods[0], cx);
    assert_eq!(titles(&dock, cx).len(), 3);
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(0));
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
}

#[gpui_kit::test]
fn closing_a_running_shells_tab_asks_first(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    pick_shell(handle, &pilot, &pods[1], cx);
    let click = |id: &'static str, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    // The tab's ×: Cancel keeps it and its shell.
    click("dock-tab-1-close", cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pods[1].name));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(cx.read(shell::running_anywhere).len(), 2);

    // The chrome's × asks once for both.
    click("dock-close-all", cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, "End 2 shells?");
    cx.simulate_prompt_answer("End the shells");
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(cx.read(shell::running_anywhere).is_empty());

    // A shell that has ended closes without asking.
    pick_shell(handle, &pilot, &pods[0], cx);
    let view = shell_view(&dock, 0, cx);
    cx.update(|cx| view.update(cx, |view, cx| view.end(cx)));
    let id = cx.update(|cx| dock.read(cx).tabs[0].id);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("dock-tab-{id}-close"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(titles(&dock, cx).is_empty());
}

#[gpui_kit::test]
fn a_ninth_shell_tab_asks_to_close_the_oldest(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    assert!(pods.len() > MAX_SHELL_TABS);
    // Log tabs don't count toward the shells' cap.
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    for pod in &pods[..MAX_SHELL_TABS] {
        start_shell(handle, &pilot, pod, cx);
    }
    assert_eq!(titles(&dock, cx).len(), MAX_SHELL_TABS + 1);
    assert_eq!(cx.read(shell::running_anywhere).len(), MAX_SHELL_TABS);
    assert!(!cx.has_pending_prompt());

    // Every shell runs: the oldest asks first, and Cancel opens nothing.
    start_shell(handle, &pilot, &pods[MAX_SHELL_TABS], cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pods[0].name));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx).len(), MAX_SHELL_TABS + 1);
    assert_eq!(cx.read(shell::running_anywhere).len(), MAX_SHELL_TABS);

    // Agreeing ends the oldest shell and starts the new one.
    start_shell(handle, &pilot, &pods[MAX_SHELL_TABS], cx);
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    let shown = titles(&dock, cx);
    assert_eq!(shown.len(), MAX_SHELL_TABS + 1);
    assert_eq!(shown[0], format!("Pod {}", pods[0].name));
    assert_eq!(shown[1], format!("Shell {}", pods[1].name));
    assert_eq!(
        shown.last().unwrap(),
        &format!("Shell {}", pods[MAX_SHELL_TABS].name)
    );
    let running = cx.read(shell::running_anywhere);
    assert_eq!(running.len(), MAX_SHELL_TABS);
    assert!(!running.iter().any(|pod| pod.as_ref() == pods[0].name));

    // A shell that ended goes first, before older running ones, unasked.
    let ended = shell_view(&dock, 4, cx);
    let ended_pod = cx.read(|cx| ended.read(cx).pod().unwrap().name.clone());
    cx.update(|cx| ended.update(cx, |view, cx| view.end(cx)));
    cx.run_until_parked();
    start_shell(handle, &pilot, &pods[0], cx);
    assert!(!cx.has_pending_prompt());
    let shown = titles(&dock, cx);
    assert_eq!(shown.len(), MAX_SHELL_TABS + 1);
    assert!(!shown.contains(&format!("Shell {ended_pod}")));
    assert_eq!(shown[1], format!("Shell {}", pods[1].name));
    assert_eq!(shown.last().unwrap(), &format!("Shell {}", pods[0].name));
}

#[gpui_kit::test]
fn the_terminal_keeps_control_period_and_comma(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    pick_shell(handle, &pilot, &pods[1], cx);
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(1));
    for key in ["ctrl-.", "ctrl-,"] {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        // The shell has them; the tab stays.
        assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(1));
    }
    // Command-Escape hands the keyboard back, and the keys step the tabs.
    let leave = if cfg!(target_os = "macos") {
        "secondary-escape"
    } else {
        "ctrl-shift-q"
    };
    cx.update_window(handle, |_, window, cx| window.press(leave, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("ctrl-.", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(0));
    assert_eq!(shell_state(&dock, 1, cx), ShellState::Running);
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

#[gpui_kit::test]
fn a_shell_tab_asked_for_by_name_starts_on_the_pod_its_watch_finds(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let pod = pods[0].clone();
    assert!(!pod.uid.is_empty());
    // As a restore on a cluster: the pod by name alone, no UID yet.
    let by_name = ResourceIdentity {
        uid: String::new(),
        ..pod.clone()
    };
    let target = crate::resources::detail::DetailTarget {
        identity: by_name.clone(),
        kind: builtin("pods").unwrap(),
    };
    let container = watched(&pod).pods[0].containers.default.clone().unwrap();
    let id = cx
        .update_window(handle, |_, window, cx| {
            dock.update(cx, |dock, cx| {
                let id = dock.add_shell(target, container.clone(), window, cx);
                let view = shell_view_of(dock, 0);
                let access = dock.source.as_ref().unwrap().access.clone();
                view.update(cx, |view, cx| {
                    view.show_pod(Some(by_name.clone()), Some(access), cx);
                    view.choose_container(container.clone(), cx);
                });
                id
            })
        })
        .unwrap();
    let view = cx.update(|cx| shell_view_of(&dock.read(cx), 0));
    assert!(!cx.read(|cx| view.read(cx).can_start()), "no UID, no start");

    // The watch finds the pod: the view takes its UID and keeps its container.
    cx.update(|cx| dock.update(cx, |dock, cx| dock.apply_pod(id, watched(&pod), cx)));
    cx.read(|cx| {
        let view = view.read(cx);
        assert_eq!(view.pod().unwrap().uid, pod.uid);
        assert_eq!(view.container(), Some(container.as_str()));
        assert!(view.can_start());
    });
    // The menu picks the same pod: the tab, not another.
    pick_shell(handle, &pilot, &pod, cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pod.name)]);
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
}

#[gpui_kit::test]
fn a_shell_tab_asked_for_by_name_takes_the_picked_pods_uid(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pod = running_pods(&pilot, cx)[0].clone();
    let by_name = ResourceIdentity {
        uid: String::new(),
        ..pod.clone()
    };
    let container = watched(&pod).pods[0].containers.default.clone().unwrap();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            let target = crate::resources::detail::DetailTarget {
                identity: by_name.clone(),
                kind: builtin("pods").unwrap(),
            };
            dock.add_shell(target, container.clone(), window, cx);
            let view = shell_view_of(dock, 0);
            let access = dock.source.as_ref().unwrap().access.clone();
            view.update(cx, |view, cx| {
                view.show_pod(Some(by_name.clone()), Some(access), cx);
                view.choose_container(container.clone(), cx);
            });
        })
    })
    .unwrap();
    // Before its watch answers, the menu's pick settles the tab's pod.
    pick_shell(handle, &pilot, &pod, cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pod.name)]);
    let view = cx.update(|cx| shell_view_of(&dock.read(cx), 0));
    assert_eq!(
        cx.read(|cx| view.read(cx).pod().unwrap().uid.clone()),
        pod.uid
    );
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
}

#[gpui_kit::test]
fn a_restored_shell_tab_comes_back_idle(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    let container = {
        let view = shell_view(&dock, 0, cx);
        cx.update(|cx| view.read(cx).container().unwrap().to_owned())
    };
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved = SavedDock {
        height: 260.,
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![SavedTab {
            kind: "pods".into(),
            context,
            namespace: pods[0].namespace.clone(),
            name: pods[0].name.clone(),
            container: Some(container.clone()),
            previous: false,
            shell: true,
        }],
    };
    // What the running tab saves is what comes back.
    let written = cx.update(|cx| dock.read(cx).saved(cx));
    assert_eq!(written.tabs, saved.tabs);
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).kube_source();
        dock.update(cx, |dock, cx| {
            dock.set_source(None, window, cx);
            dock.restore = Some(saved);
            dock.set_source(source, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pods[0].name)]);
    // Shown and selected, it runs nothing until Start.
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Idle);
    assert!(cx.read(shell::running_anywhere).is_empty());
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.find("pod-shell-start").label(), Some("Start"));
        window.click("pod-shell-start", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
    let view = shell_view(&dock, 0, cx);
    assert_eq!(
        cx.update(|cx| view.read(cx).container().map(str::to_owned)),
        Some(container)
    );
}
