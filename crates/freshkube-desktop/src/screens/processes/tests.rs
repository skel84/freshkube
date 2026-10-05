use std::sync::Arc;

use freshkube_core::inspection::{ProcessSort, ProcessTree};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    App, AppContext, Entity, Pixels, SharedString, Size, TestAppContext, WindowHandle, px, size,
};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ProcessesScreen, ScreenPanel, ScreenSource, StateFilter};
use crate::backend::Target;
use crate::{fixture, presentation};

fn source(node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: "prod-fra".into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

/// The element id of the row on `line`, which the table keys by PID.
fn row_id(screen: &Entity<ProcessesScreen>, line: usize, cx: &App) -> (&'static str, usize) {
    ("process", screen.read(cx).rows()[line].pid as usize)
}

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<ProcessesScreen>, WindowHandle<Root>) {
    mount_in(cx, node, size(px(1100.), px(760.)))
}

fn mount_in(
    cx: &mut TestAppContext,
    node: &str,
    bounds: Size<Pixels>,
) -> (Runtime, Entity<ProcessesScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(bounds, |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ProcessesScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

#[gpui_kit::test]
fn keyboard_selects_and_toggles_the_subtree(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        window.click(row_id(&screen, 0, cx), cx);
        window.press("down", cx);
        window.render_frame(cx);
        let selected = screen.read(cx).selected;
        assert_eq!(selected, Some(screen.read(cx).rows()[1].pid));
        assert_eq!(window.find(row_id(&screen, 1, cx)).selected(), Some(true));
        window.press("t", cx);
        assert!(matches!(
            screen.read(cx).tree,
            ProcessTree::Subtree { root_pid } if Some(root_pid) == selected
        ));
        window.press("t", cx);
        assert_eq!(screen.read(cx).tree, ProcessTree::Flat);
    })
    .unwrap();
}

#[gpui_kit::test]
fn state_filter_and_copy_command(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("state-zombie", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).rows().len(), 1, "one example zombie");
        window.click(row_id(&screen, 0, cx), cx);
        window.render_frame(cx);
        window.click("copy-command", cx);
        assert!(screen.read(cx).copied.is_some());
        assert!(cx.read_from_clipboard().is_some());
        // Optional sources that failed are reported, not shown as failures.
        window.find("partial-notice");
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_offers_retry_without_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_none());
        window.find("screen-retry");
        assert!(window.try_find("processes-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some(1);
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn rows_are_derived_only_when_the_data_or_the_view_change(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let computed = || crate::desktop::probe::count("processes.rows");
        let derive = |cx: &mut App| screen.update(cx, |screen, cx| screen.sync_rows(cx));
        let first = screen.read(cx).rows().len();
        let base = computed();
        // Frames, hover and navigation read the rows already derived.
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(computed(), base, "nothing it depends on changed");
        // The state filter.
        screen.update(cx, |screen, cx| {
            screen.state_filter = StateFilter::Zombie;
            cx.notify();
        });
        derive(cx);
        assert_eq!(computed(), base + 1);
        assert!(screen.read(cx).rows().len() < first);
        // Sort and tree mode.
        screen.update(cx, |screen, cx| {
            screen.state_filter = StateFilter::All;
            screen.set_sort(ProcessSort::ResidentMemory, cx);
        });
        derive(cx);
        assert_eq!(computed(), base + 2);
        screen.update(cx, |screen, cx| {
            screen.tree = ProcessTree::Full;
            cx.notify();
        });
        derive(cx);
        assert_eq!(computed(), base + 3);
        // The filter text.
        screen.update(cx, |screen, cx| {
            screen
                .query
                .update(cx, |input, cx| input.set_value("kubelet", window, cx));
        });
        derive(cx);
        assert_eq!(computed(), base + 4);
        assert!(screen.read(cx).rows().len() < first);
        window.render_frame(cx);
        assert_eq!(computed(), base + 4);
        // A new sample.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        window.render_frame(cx);
        assert_eq!(computed(), base + 5);
    })
    .unwrap();
}

#[gpui_kit::test]
fn selection_follows_the_pid_when_a_sample_reorders_the_rows(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // The first sample has no CPU deltas; the second sorts by them.
        let before: Vec<i32> = screen.read(cx).rows().iter().map(|row| row.pid).collect();
        let pid = before[0];
        window.click(("process", pid as usize), cx);
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        window.render_frame(cx);
        let after: Vec<i32> = screen.read(cx).rows().iter().map(|row| row.pid).collect();
        assert_ne!(before, after, "the second sample reorders the rows");
        assert_ne!(after[0], pid);
        assert_eq!(screen.read(cx).selected, Some(pid));
        assert_eq!(
            window.find(("process", pid as usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find(("process", after[0] as usize)).selected(),
            Some(false)
        );
        // Arrows step from where the process is now.
        window.press("down", cx);
        let line = after.iter().position(|row| *row == pid).unwrap();
        assert_eq!(screen.read(cx).selected, Some(after[line + 1]));
    })
    .unwrap();
}

#[gpui_kit::test]
fn sortable_headers_sort_and_show_their_arrow(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // Glyph, PID, Command, State, CPU, CPU time, Memory, Threads.
        let header = |window: &mut gpui_kit::Window, column: usize| {
            let header = window.find(("processes-sort", column));
            header.label().map(str::to_owned)
        };
        assert_eq!(header(window, 4).as_deref(), Some("CPU, sorted descending"));
        window.click(("processes-sort", 6usize), cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).sort, ProcessSort::ResidentMemory);
        assert_eq!(
            header(window, 6).as_deref(),
            Some("Memory, sorted descending")
        );
        assert_eq!(header(window, 4).as_deref(), Some("CPU"));
        let memory: Vec<u64> = screen
            .read(cx)
            .loader
            .data()
            .map(|data| {
                let rows = screen.read(cx).rows();
                rows.iter()
                    .map(|row| {
                        let entry = data.processes.iter().find(|e| e.process.pid == row.pid);
                        entry.unwrap().process.resident_memory
                    })
                    .collect()
            })
            .unwrap();
        assert!(
            memory.windows(2).all(|pair| pair[0] >= pair[1]),
            "largest first"
        );
        window.click(("processes-sort", 5usize), cx);
        assert_eq!(screen.read(cx).sort, ProcessSort::CpuTime);
        // PID and Command don't sort.
        window.click(("processes-sort", 1usize), cx);
        window.click(("processes-sort", 2usize), cx);
        assert_eq!(screen.read(cx).sort, ProcessSort::CpuTime);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_command_shows_without_a_sideways_scroll(cx: &mut TestAppContext) {
    // About as narrow as the node pane leaves the table at 1280 wide.
    let (_runtime, _screen, handle) = mount_in(cx, "talos-cp-fra1-01", size(px(600.), px(760.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = window.find("processes-table-scroll").bounds();
        // By its label: the header's ids number the columns.
        let command = (0..8usize)
            .filter_map(|column| window.try_find(("processes-sort", column)))
            .find(|header| header.label() == Some("Command"))
            .unwrap()
            .bounds();
        assert!(view.size.width < px(560.), "{view:?}");
        assert!(command.right() <= view.right(), "{command:?} in {view:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn only_zombie_and_disk_wait_rows_have_a_glyph(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // The rows in view have no glyph: none is a zombie or waits on the disk.
        let shown: Vec<i32> = screen.read(cx).rows().iter().map(|row| row.pid).collect();
        let mut checked = 0;
        for pid in shown.into_iter().filter(|pid| ![2216, 2302].contains(pid)) {
            if window.try_find(("process", pid as usize)).is_some() {
                let id = SharedString::from(format!("process-{pid}-state"));
                assert!(window.try_find(id.clone()).is_none(), "{id}");
                checked += 1;
            }
        }
        assert!(checked > 5, "{checked} rows checked");
        // 2216 is the example zombie, 2302 waits on the disk.
        window.click("state-zombie", cx);
        window.render_frame(cx);
        window.find("process-2216-state");
        window.click("state-disk-wait", cx);
        window.render_frame(cx);
        window.find("process-2302-state");
    })
    .unwrap();
}

#[gpui_kit::test]
fn keys_work_while_the_filters_hide_every_row(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row_id(&screen, 0, cx), cx);
        // The control plane has no zombies.
        window.press("z", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).rows().is_empty());
        window.find("processes-empty");
        // Still the page's keys: `d` switches the filter and Escape clears it.
        window.press("d", cx);
        assert_eq!(screen.read(cx).state_filter, StateFilter::DiskWait);
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).state_filter, StateFilter::All);
        assert!(!screen.read(cx).rows().is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn processes_draw_the_shared_table_at_both_text_sizes(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{Table, assert_table};
    let (_runtime, _screen, handle) = mount(cx, "talos-cp-fra1-01");
    let table = Table {
        table: Some("processes-table-scroll"),
        list: "processes-list",
    };
    for text_size in [crate::ui::BASE_TEXT, 20.] {
        cx.update_window(handle.into(), |_, _, cx| {
            crate::text_size::set(text_size, cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            let rows = assert_table(window, cx, &table);
            assert!(rows.header.is_some(), "{rows:#?}");
        })
        .unwrap();
    }
}
