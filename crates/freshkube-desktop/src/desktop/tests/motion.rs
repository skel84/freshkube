use super::*;

#[gpui_kit::test]
fn a_cold_nodes_table_draws_its_header_over_the_loading_rows(cx: &mut TestAppContext) {
    // Nothing has answered: neither Talos nor the summary built the rows.
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-2", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Nodes);
        assert!(view.read(cx).nodes_wait(cx));
        assert!(window.find("nodes-loading").visible());
        let header: Vec<_> = (0usize..12)
            .filter_map(|ix| window.try_find(("nodes-sort", ix)))
            .filter_map(|cell| cell.label().map(str::to_owned))
            .collect();
        assert!(
            header.iter().any(|label| label == "Name") && header.len() > 4,
            "{header:?}"
        );
    })
    .unwrap();
}

/// The loading motion sits beside the page and the cached chrome: its
/// frames redraw the shell's frame, but neither the header, the rail, the
/// column, nor the page and its table.
#[gpui_kit::test]
fn the_loading_motion_redraws_neither_the_chrome_nor_the_page(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = loading_pods(cx);
    let motion = probe::count("table.loading-motion");
    assert!(frames_redrawing(cx, handle, "resources") <= 1);
    assert!(probe::count("table.loading-motion") >= motion + 60);
}

/// A watch's change flashes its row from the layer beside the page: its
/// frames redraw neither the chrome nor the page, and stop once it fades.
#[gpui_kit::test]
fn the_change_flash_redraws_neither_the_chrome_nor_the_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| open_kind(window, cx, "pods"))
        .unwrap();
    cx.run_until_parked();
    // Whatever the first rows asked for draws first.
    while motion_frame(cx, handle) > 0 {}
    let resources = cx.update(|cx| view.read(cx).resources.clone());
    resources.update(cx, |resources, cx| {
        let changed = resources.restarted(0);
        resources.deliver(
            vec![crate::resources::store::ResourceEvent::Upsert(changed)],
            cx,
        );
    });
    cx.run_until_parked();
    // The change itself draws the page once.
    motion_frame(cx, handle);
    assert_eq!(motion_frame(cx, handle), 0, "the fade asks for no frames");
    // A timer steps the fade: each step draws the layer, not the page.
    let flash = probe::count("table.flash-layer");
    let page = probe::count("resources");
    let rail = probe::count("chrome.rail");
    // The steps, and one past the end that clears the last.
    for _ in 0..=freshkube_ui::motion::FLASH_STEPS {
        cx.background_executor
            .advance_clock(freshkube_ui::motion::FLASH_STEP);
        cx.run_until_parked();
    }
    let steps = probe::count("table.flash-layer") - flash;
    assert!((6..=8).contains(&steps), "{steps} steps drawn");
    assert_eq!(probe::count("resources"), page);
    assert_eq!(probe::count("chrome.rail"), rail);
    // Once the fade ends, nothing steps.
    let layer = cx
        .update(|cx| view.read(cx).page_flash(cx))
        .expect("the flash");
    cx.read(|cx| assert!(!layer.read(cx).stepping()));
    assert_eq!(
        motion_frame(cx, handle),
        0,
        "no frames once the flash fades"
    );
}

/// The first answer replaces the loading rows, and the motion asks no
/// more frames.
#[gpui_kit::test]
fn the_loading_motion_stops_on_the_first_answer(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = loading_pods(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.hold.lists = false;
            pilot.push_source(window, cx);
        });
        let resources = view.read(cx).resources.clone();
        resources.update(cx, |resources, cx| resources.refresh(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    // At most the frame already asked, which draws the rows.
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        assert_eq!(motion_frame(cx, handle), 0, "no frames once Pods answers");
    }
    assert_eq!(probe::count("table.loading-motion"), motion);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("resource-loading").is_none());
    })
    .unwrap();
}

/// Runs a few frames and asserts that the loading motion asks for none
/// and draws no more: whatever replaced the table took it away.
fn assert_motion_stopped(cx: &mut TestAppContext, handle: AnyWindowHandle, why: &str) {
    // At most the frame already asked, which draws what replaced the rows.
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        assert_eq!(motion_frame(cx, handle), 0, "no frames after {why}");
    }
    assert_eq!(probe::count("table.loading-motion"), motion, "{why}");
}

/// A refusal or a failure replaces the loading rows with a state, and
/// the motion goes with the table: nothing asks for frames over it.
#[gpui_kit::test]
fn the_loading_motion_stops_when_the_list_is_refused_or_fails(cx: &mut TestAppContext) {
    use crate::resources::model::ReadState;
    let (_runtime, handle, view) = loading_pods(cx);
    let resources = cx.update(|cx| view.read(cx).resources.clone());
    for (state, shown) in [
        (
            ReadState::Refused("pods is forbidden".into()),
            "resource-refused",
        ),
        (ReadState::Failed("Timeout".into()), "resource-failed"),
    ] {
        // Listing again, held, brings the loading rows and their motion back.
        cx.update_window(handle, |_, window, cx| {
            resources.update(cx, |resources, cx| resources.refresh(window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        motion_frame(cx, handle);
        assert!(motion_frame(cx, handle) > 0, "the rows move before {shown}");
        cx.update(|cx| resources.update(cx, |resources, cx| resources.deliver_read(state, cx)));
        cx.run_until_parked();
        assert_motion_stopped(cx, handle, shown);
        cx.update_window(handle, |_, window, cx| {
            assert!(window.find(shown).visible());
            assert!(window.try_find("resource-loading").is_none());
            assert!(view.read(cx).page_loading_motion(cx).is_none());
        })
        .unwrap();
    }
}

/// Nodes waiting on Talos that then fails show their failure, not the
/// table, and the motion stops; the cards view never mounts it.
#[gpui_kit::test]
fn the_nodes_loading_motion_stops_on_a_failure_or_the_cards(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-2", cx))
        .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    assert!(
        motion_frame(cx, handle) > 0,
        "the rows move while Talos loads"
    );
    cx.update_window(handle, |_, window, cx| window.click("nodes-view-cards", cx))
        .unwrap();
    cx.run_until_parked();
    // The cards wait under their own skeleton, which may ask frames of
    // its own; the table's motion neither draws nor is mounted.
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    assert_eq!(probe::count("table.loading-motion"), motion, "the cards");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("nodes-loading").visible());
        assert!(view.read(cx).page_loading_motion(cx).is_none());
        window.click("nodes-view-table", cx);
    })
    .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    assert!(motion_frame(cx, handle) > 0, "the table's rows move again");
    cx.update(|cx| view.update(cx, |view, cx| view.simulate_failure(cx)));
    cx.run_until_parked();
    assert_motion_stopped(cx, handle, "a failure");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("nodes-failed").visible());
        assert!(view.read(cx).page_loading_motion(cx).is_none());
    })
    .unwrap();
}

/// Lifecycle's roster pulses through the shell while it waits for its
/// first answer, and the motion stops when the rows or a failure arrive.
#[gpui_kit::test]
fn the_lifecycle_loading_motion_stops_on_its_answer_or_a_failure(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-9", cx))
        .unwrap();
    cx.run_until_parked();
    let lifecycle = cx.update(|cx| view.read(cx).lifecycle.clone());
    for (ok, shown) in [(true, "lifecycle-list"), (false, "lifecycle-state")] {
        cx.update(|cx| lifecycle.update(cx, |lifecycle, cx| lifecycle.wait_again(cx)));
        cx.run_until_parked();
        motion_frame(cx, handle);
        assert!(motion_frame(cx, handle) > 0, "the rows move before {shown}");
        cx.update_window(handle, |_, window, cx| {
            assert!(window.find("lifecycle-loading").visible());
            assert!(view.read(cx).page_loading_motion(cx).is_some());
        })
        .unwrap();
        cx.update(|cx| lifecycle.update(cx, |lifecycle, cx| lifecycle.answer(ok, cx)));
        cx.run_until_parked();
        assert_motion_stopped(cx, handle, shown);
        cx.update_window(handle, |_, window, cx| {
            assert!(window.find(shown).visible());
            assert!(window.try_find("lifecycle-loading").is_none());
            assert!(view.read(cx).page_loading_motion(cx).is_none());
        })
        .unwrap();
    }
}

/// Every screen table shows the shared loading rows under its header, edge
/// to edge as its rows will be, while the overview is read; the shell
/// pulses them, and the first answer stops the motion.
#[gpui_kit::test]
fn screen_tables_show_the_shared_loading_rows_until_the_first_answer(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    let pages = [
        (Page::Health, "workloads", "Workloads"),
        (Page::Etcd, "etcd", "etcd"),
        (Page::Security, "security", "Security"),
        (Page::Operations, "ops", "Operations"),
        // Last: the answer below is checked on it.
        (Page::Lifecycle, "lifecycle", "Lifecycle"),
    ];
    for (page, prefix, title) in pages {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.navigate(page, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        motion_frame(cx, handle);
        assert!(motion_frame(cx, handle) > 0, "{prefix}'s rows move");
        cx.update_window(handle, |_, window, cx| {
            let loading: &'static str = format!("{prefix}-loading").leak();
            assert!(window.find(loading).visible());
            super::super::layout_check::assert_edge_frame(
                window,
                cx,
                &super::super::layout_check::PageFrame {
                    page: format!("{prefix}-page").leak(),
                    title: format!("{prefix}-title").leak(),
                    title_text: title,
                    content: loading,
                },
            );
            assert!(view.read(cx).page_loading_motion(cx).is_some());
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.hold.talos = false;
            // The held read still counts as in flight; let the refresh start one.
            view.overview = crate::state::Snapshot::default();
            view.refresh_now(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    // The example answers through Tokio; draw until Lifecycle's rows are in.
    for _ in 0..50 {
        let waiting = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("lifecycle-loading").is_some()
            })
            .unwrap();
        if !waiting {
            break;
        }
        motion_frame(cx, handle);
    }
    assert_motion_stopped(cx, handle, "the answer");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("lifecycle-loading").is_none());
        window.find("lifecycle-list");
        assert!(view.read(cx).page_loading_motion(cx).is_none());
    })
    .unwrap();
}

/// A page that hides takes its motion with it: the shell no longer mounts
/// it, so nothing asks frames, and a shell redraw doesn't draw it.
#[gpui_kit::test]
fn a_hidden_pages_loading_motion_is_never_mounted(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = loading_pods(cx);
    cx.update_window(handle, |_, window, cx| window.press("secondary-1", cx))
        .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        assert_eq!(motion_frame(cx, handle), 0, "no frames from a hidden page");
    }
    let shell = probe::count("shell");
    cx.update(|cx| view.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert!(probe::count("shell") > shell);
    assert_eq!(probe::count("table.loading-motion"), motion);
    cx.update(|cx| assert!(view.read(cx).page_loading_motion(cx).is_none()));
}

/// Draws the next frame as the platform would, without `render_frame`,
/// which redraws every cached view: a part that missed its notify shows
/// what it drew before.
fn next_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

/// Talos answering for the first time marks the rail through the real
/// path, from the example answer to the overview's cards, and the cached
/// rail draws the mark on the next frame.
#[gpui_kit::test]
fn the_cached_rail_draws_the_marks_the_first_answer_brings(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    next_frame(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.within("nav-nodes").try_find("rail-mark").is_none());
    })
    .unwrap();
    let rail = probe::count("chrome.rail");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.hold.talos = false;
            view.refresh_now(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    next_frame(cx, handle);
    assert!(probe::count("chrome.rail") > rail, "the rail draws again");
    cx.update_window(handle, |_, window, _| {
        assert!(window.within("nav-nodes").find("rail-mark").visible());
    })
    .unwrap();
}

/// The shell mounts Nodes' loading motion from Nodes' view, so a
/// Cards/Table switch, which Nodes makes itself, tells the shell to draw
/// again: the motion goes with the cards and comes back with the table.
#[gpui_kit::test]
fn a_nodes_view_switch_redraws_the_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-2", cx))
        .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    let clicked = |id: &'static str, cx: &mut TestAppContext| {
        let before = probe::count("shell");
        cx.update_window(handle, |_, window, cx| window.click(id, cx))
            .unwrap();
        cx.run_until_parked();
        motion_frame(cx, handle);
        probe::count("shell") - before
    };
    assert!(
        clicked("nodes-view-cards", cx) > 0,
        "the cards redraw the shell"
    );
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    assert_eq!(probe::count("table.loading-motion"), motion, "the cards");
    assert!(
        clicked("nodes-view-table", cx) > 0,
        "the table redraws the shell"
    );
    motion_frame(cx, handle);
    assert!(motion_frame(cx, handle) > 0, "the table's rows move again");
    assert!(
        probe::count("table.loading-motion") > motion,
        "the motion draws again"
    );
}

/// An Expand, which Nodes makes itself, tells the shell to draw again: the
/// expanded pane replaces the table, and the shell mounts the motion only
/// over a drawn table. The next frame draws only a dirty window, so the
/// shell's count rises only if the Expand dirtied it; a settled window
/// draws nothing.
#[gpui_kit::test]
fn a_node_expand_redraws_the_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        crate::desktop::tests::click_node(window, cx, "node-talos-cp-fra1-01");
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        next_frame(cx, handle);
    }
    let settled = probe::count("shell");
    next_frame(cx, handle);
    assert_eq!(probe::count("shell"), settled, "settled, nothing draws");
    for expanded in [true, false] {
        let before = probe::count("shell");
        // Straight on Nodes, as no click's frame would draw the shell anyway.
        cx.update(|cx| {
            let nodes = view.read(cx).node_workspace.clone();
            nodes.update(cx, |nodes, cx| nodes.toggle_expanded(cx));
        });
        next_frame(cx, handle);
        cx.update(|cx| assert_eq!(view.read(cx).node_workspace.read(cx).expanded, expanded));
        assert!(probe::count("shell") > before, "expanded {expanded}");
    }
}

/// A screen that is always on its loading rows, for the node pane's
/// motion.
struct Loading(Entity<freshkube_ui::table::LoadingMotion>);

impl gpui_kit::EventEmitter<crate::screens::ScreenEvent> for Loading {}

impl gpui_kit::Render for Loading {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

impl crate::screens::ScreenPanel for Loading {
    fn set_source(
        &mut self,
        _: Option<crate::screens::ScreenSource>,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) {
    }

    fn activate(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn refresh(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn loading_motion(
        &self,
        _: &gpui_kit::App,
    ) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        Some(self.0.clone())
    }
}

/// The open pane's Talos screen gives the shell its loading motion through
/// Nodes while its tab shows, and none once another tab does.
#[gpui_kit::test]
fn the_pane_screens_loading_motion_reaches_the_shell_through_nodes(cx: &mut TestAppContext) {
    use super::super::{nodes::NodeTab, pages::PageId};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let motion = cx.new(|_| {
        freshkube_ui::table::LoadingRows::new("fake").motion(freshkube_ui::table::Look::Pulse)
    });
    let screen = crate::screens::ScreenHandle::new(cx.new(|_| Loading(motion.clone())));
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            super::super::testing::put_page(view, PageId::Processes, screen, cx)
        })
    });
    let shown = |tab, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            open_node_tab(window, cx, tab);
            view.read(cx).page_loading_motion(cx)
        })
        .unwrap()
    };
    assert_eq!(shown(NodeTab::Processes, cx), Some(motion.clone()));
    assert_eq!(shown(NodeTab::Overview, cx), None);
}
