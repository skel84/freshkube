use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ScreenPanel, ScreenSource, StorageScreen, ViewMode};
use crate::backend::Target;
use crate::{fixture, presentation};

#[test]
fn read_only_is_expected_on_loop_devices_and_flagged_on_disks() {
    let image = super::disk("loop0", 147_456, "147 kB", "", "", "", false, true, false);
    let stuck = super::disk(
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
    let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
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
        window.click(("disk", 0usize), cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("disk", 1usize)).selected(), Some(true));
        assert_eq!(window.find(("disk", 0usize)).selected(), Some(false));
        assert_eq!(screen.read(cx).selected_disk.as_deref(), Some("sda"));
        window.find("disk-details");
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("disk", 0usize)).selected(), Some(true));
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
        window.click(("volume", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(window.find(("volume", 3usize)).selected(), Some(true));
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
                .any(|volume| volume.phase == "waiting")
        );
        window.press("end", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("volume", 4usize)).selected(), Some(true));
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
        window.find(("disk", 0usize));
        window.click("storage-view-volumes", cx);
        window.render_frame(cx);
        assert!(window.try_find(("volume", 0usize)).is_none());
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
        assert!(window.try_find("storage-disks").is_none());
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
