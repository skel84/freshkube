use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, InputEvent, MouseMoveEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, Size, TestAppContext, TouchPhase, Window, WindowHandle, point, px, size,
};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ScreenPanel, ScreenSource, StorageRequest, StorageScreen, ViewMode, example};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::nodes::NodeTab;
use crate::desktop::tests::{fixture as app, open_node_tab};
use crate::{fixture, presentation};

/// The page's frame reaches the split of the table and its details.
const STORAGE_FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "storage-page",
    title: "storage-title",
    title_text: "Storage",
    content: "storage-split",
};

#[test]
fn storage_asks_talosctl_for_the_host_without_its_port() {
    let config = std::path::Path::new("/configs/talosconfig");
    let node = |address: &str| {
        StorageRequest::new("lab", address, Some(config)).map(|request| request.node)
    };
    assert_eq!(node("10.0.0.5:50000").as_deref(), Ok("10.0.0.5"));
    assert_eq!(node("2001:db8::5").as_deref(), Ok("2001:db8::5"));
    assert_eq!(node("[2001:db8::5]:50000").as_deref(), Ok("2001:db8::5"));
    assert!(node("").is_err());
    assert!(node("-flag").is_err());
}

#[test]
fn read_only_is_expected_on_loop_devices_and_flagged_on_disks() {
    let image = super::example::disk("loop0", 147_456, "147 kB", "", "", "", false, true, false);
    let stuck = super::example::disk(
        "sda",
        1 << 30,
        "1.1 GB",
        "QEMU",
        "1",
        "sata",
        false,
        true,
        false,
    );

    assert_eq!(super::disk_type(&image), "Loop");
    assert!(!super::unexpected_read_only(&image));
    assert_eq!(super::disk_type(&stuck), "SSD");
    assert!(super::unexpected_read_only(&stuck));
    let optical = super::example::disk(
        "sr0",
        1 << 30,
        "1.1 GB",
        "QEMU DVD-ROM",
        "3",
        "sata",
        false,
        true,
        true,
    );
    assert!(!super::unexpected_read_only(&optical));
}

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

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<StorageScreen>, WindowHandle<Root>) {
    mount_in(cx, node, size(px(1100.), px(760.)))
}

fn mount_in(
    cx: &mut TestAppContext,
    node: &str,
    bounds: Size<Pixels>,
) -> (Runtime, Entity<StorageScreen>, WindowHandle<Root>) {
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
            let mut view = StorageScreen::new(runtime.handle().clone(), window, cx);
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
fn keyboard_selects_disks_and_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).disks().len(), 2);
        window.click("disk-nvme0n1", cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find("disk-sda").selected(), Some(true));
        assert_eq!(window.find("disk-nvme0n1").selected(), Some(false));
        assert_eq!(screen.read(cx).selected_disk.as_deref(), Some("sda"));
        window.find("disk-details");
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(window.find("disk-nvme0n1").selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn volumes_tab_selects_and_shows_degraded_phase(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("storage-view-volumes", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).mode, ViewMode::Volumes);
        window.click("volume-EPHEMERAL", cx);
        window.render_frame(cx);
        assert_eq!(window.find("volume-EPHEMERAL").selected(), Some(true));
        assert_eq!(
            screen.read(cx).selected_volume.as_deref(),
            Some("EPHEMERAL")
        );
        window.find("volume-details");
        assert!(
            screen
                .read(cx)
                .volumes()
                .iter()
                .any(|volume| volume.info.phase == "waiting")
        );
        window.press("end", cx);
        window.render_frame(cx);
        assert_eq!(window.find("volume-u-data").selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn one_failed_source_shows_partial_notice_and_the_rest(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-03-baremetal-rack-b7");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("partial-notice");
        assert_eq!(screen.read(cx).disks().len(), 1);
        window.find("disk-nvme0n1");
        window.click("storage-view-volumes", cx);
        window.render_frame(cx);
        assert!(window.try_find("volume-EFI").is_none());
        // The table names the missing side as unknown, never as empty.
        window.find("storage-volumes-empty");
        assert!(screen.read(cx).volumes().is_empty());
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
        assert!(window.try_find("storage-table").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected_disk = Some("vda".into());
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected_disk.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn each_table_keeps_its_selection_and_keys_without_rows(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-03-baremetal-rack-b7");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("disk-nvme0n1", cx);
        window.render_frame(cx);
        // Tab reaches the volumes even though they're unknown, and back.
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).mode, ViewMode::Volumes);
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).mode, ViewMode::Disks);
        assert_eq!(window.find("disk-nvme0n1").selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn selection_follows_the_disk_not_its_line(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("disk-sda", cx);
        window.render_frame(cx);
        screen.update(cx, |screen, _| {
            let disks = screen.loader.data().unwrap().disks.as_ref().unwrap();
            let infos: Vec<_> = disks
                .rows
                .iter()
                .rev()
                .map(|row| row.info.clone())
                .collect();
            let volumes = Ok(screen
                .volumes()
                .iter()
                .map(|row| row.info.clone())
                .collect());
            let target = screen.source.as_ref().unwrap().target.clone();
            let data = super::StorageData::new(Ok(infos), volumes);
            screen.loader.resolve(target, Ok(data));
        });
        window.render_frame(cx);
        assert_eq!(window.find("disk-sda").selected(), Some(true));
        assert_eq!(screen.read(cx).selected_disk.as_deref(), Some("sda"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn storage_draws_the_shared_table_at_both_text_sizes(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{Table, assert_table};
    let (_runtime, _screen, handle) = mount(cx, "talos-wk-fra1-02");
    for (text_size, table) in [
        (
            crate::ui::BASE_TEXT,
            Table {
                table: Some("storage-disks-table-scroll"),
                list: "storage-disks-list",
            },
        ),
        (
            20.,
            Table {
                table: Some("storage-volumes-table-scroll"),
                list: "storage-volumes-list",
            },
        ),
    ] {
        cx.update_window(handle.into(), |_, window, cx| {
            crate::text_size::set(text_size, cx);
            if table.list == "storage-volumes-list" {
                window.render_frame(cx);
                window.click("storage-view-volumes", cx);
            }
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

#[gpui_kit::test]
fn volumes_show_their_phase_glyph_and_disks_only_a_warning(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // No disk here is read-only by surprise: the DVD drive is meant to be.
        assert!(window.try_find("disk-sr0-state").is_none());
        assert!(window.try_find("disk-sda-state").is_none());
        window.click("storage-view-volumes", cx);
        window.render_frame(cx);
        window.find("volume-EPHEMERAL-state");
        window.find("volume-EFI-state");
    })
    .unwrap();
}

/// One wheel event at `position`, as the start of a gesture.
fn wheel(window: &mut Window, position: Point<Pixels>, x: f32, y: f32, cx: &mut gpui_kit::App) {
    window.dispatch_event(
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(x), px(y))),
            touch_phase: TouchPhase::Started,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

/// In a window too small for the page and the disks' columns, a sideways
/// wheel over the disks scrolls their columns and leaves the page where it
/// was: the page's vertical scroll doesn't take the sideways movement for
/// its own. A plain wheel at the same place still scrolls the page.
#[gpui_kit::test]
fn a_sideways_wheel_over_the_disks_leaves_the_page_where_it_was(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle) = mount_in(cx, "talos-wk-fra1-02", size(px(640.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        let table = window.find("storage-disks-table-scroll").bounds();
        let bottom = window.viewport_size().height;
        assert!(table.top() + px(40.) < bottom, "the disks don't show");
        let at = point(table.center().x, table.top() + px(30.));
        let model = ("storage-disks-sort", 6usize);
        let left = window.find(model).bounds().left();

        wheel(window, at, -120., 0., cx);
        let moved = left - window.find(model).bounds().left();
        assert!(moved > px(10.), "the columns didn't scroll: {moved:?}");
        let fell = table.top() - window.find("storage-disks-table-scroll").bounds().top();
        assert!(fell.abs() < px(0.5), "the page scrolled {fell:?}");

        wheel(window, at, 0., -60., cx);
        let fell = table.top() - window.find("storage-disks-table-scroll").bounds().top();
        assert!(fell > px(10.), "the page didn't scroll: {fell:?}");
    })
    .unwrap();
}

#[test]
fn the_meta_line_counts_disks_volumes_and_those_not_ready() {
    let data = example(&source("talos-wk-fra1-02")).unwrap();
    assert_eq!(
        data.summary,
        ["3 disks", "2.3 TB", "5 volumes", "1 not ready"]
    );
    assert_eq!(data.disks_label, "Disks 3");
    let data = super::StorageData::new(Err("no talosctl".into()), Ok(Vec::new()));
    assert_eq!(data.summary, ["disks unknown", "0 volumes"]);
    assert_eq!(data.disks_label, "Disks ?");
}

#[gpui_kit::test]
fn storage_is_an_edge_page_at_both_text_sizes(cx: &mut TestAppContext) {
    for text in [None, Some(20.)] {
        let (_runtime, handle, _view) = app(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            open_node_tab(window, cx, NodeTab::Storage);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_untitled_edge_frame(window, cx, &STORAGE_FRAME, None);
            layout_check::assert_table(
                window,
                cx,
                &layout_check::Table {
                    table: Some("storage-disks-table-scroll"),
                    list: "storage-disks-list",
                },
            );
        })
        .unwrap();
    }
}

/// The state in the table's place sits under the toolbar, which keeps the
/// title and Refresh.
fn under_the_toolbar(window: &Window, id: &'static str) {
    let toolbar = window.find("storage-toolbar").bounds();
    window.find("storage-title");
    window.find("storage-refresh");
    let state = window.find(id).bounds();
    assert!(
        state.top() >= toolbar.bottom(),
        "{id} {state:?} isn't under the toolbar {toolbar:?}"
    );
    assert!(window.try_find("storage-table").is_none());
}

#[gpui_kit::test]
fn every_state_sits_under_the_toolbar(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // A node that isn't responding.
        under_the_toolbar(window, "screen-retry");
        // A responding node whose read failed.
        screen.update(cx, |screen, cx| {
            let source = source("talos-wk-fra1-02");
            let target = source.target.clone();
            screen.set_source(Some(source), window, cx);
            screen.loader.resolve(target, Err("talosctl failed".into()));
        });
        window.render_frame(cx);
        under_the_toolbar(window, "screen-retry");
        // No node.
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        // While the overview is read, the table shows its loading rows.
        window.find("storage-loading");
        // Once it answered with no node to target, the state says so.
        crate::screens::set_reading(crate::screens::Reading::Answered, cx);
        window.render_frame(cx);
        under_the_toolbar(window, "storage-state");
        assert!(window.try_find("screen-retry").is_none());
    })
    .unwrap();
}

/// Draws until the header stops asking for another frame to place its
/// parts.
fn settle(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        let asked = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.simulate_next_frame(cx)
            })
            .unwrap();
        if asked == 0 {
            return;
        }
    }
    panic!("the header keeps moving");
}

#[gpui_kit::test]
fn the_folded_segment_switches_sides_as_the_segment_does(cx: &mut TestAppContext) {
    // The title, Refresh and "…" fit; the segment doesn't.
    let (_runtime, screen, handle) = mount_in(cx, "talos-wk-fra1-02", size(px(260.), px(600.)));
    settle(handle.into(), cx);
    for (item, mode) in [(1usize, ViewMode::Volumes), (0, ViewMode::Disks)] {
        cx.update_window(handle.into(), |_, window, cx| {
            assert!(window.try_find("storage-view").is_none(), "not folded");
            window.find("storage-refresh");
            window.click("storage-more", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(item, cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(screen.read_with(cx, |screen, _| screen.mode), mode);
        settle(handle.into(), cx);
    }
}
