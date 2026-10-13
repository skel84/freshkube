//! `Pilot::new` hands each page the value `DebugOptions` holds for it, so a
//! test chooses an example's shape without the environment.
use super::*;
use crate::DebugOptions;
use crate::debug_options::EtcdExample;
use crate::desktop::testing::mount;
use crate::fixture::Hold;
use freshkube_ui::platform::Platform;

fn options(debug: DebugOptions) -> GpuiOptions {
    GpuiOptions::fixture().with_debug(debug)
}

#[gpui_kit::test]
fn the_etcd_shape_reaches_the_etcd_page(cx: &mut TestAppContext) {
    let debug = DebugOptions {
        etcd: Some(EtcdExample::Lost),
        ..DebugOptions::default()
    };
    let (_runtime, handle, pilot) = mount(cx, options(debug), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("etcd-quorum-banner").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_healthy_etcd_example_has_no_quorum_banner(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = mount(cx, options(DebugOptions::default()), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("etcd-quorum-banner").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_list_hold_reaches_both_resources_screens(cx: &mut TestAppContext) {
    let debug = DebugOptions {
        hold: Hold {
            lists: true,
            ..Hold::default()
        },
        ..DebugOptions::default()
    };
    let (_runtime, _handle, pilot) = mount(cx, options(debug), 1280., 880.);
    cx.update(|cx| {
        let pilot = pilot.read(cx);
        assert!(pilot.resources.read(cx).is_holding());
        assert!(pilot.node_pods.read(cx).is_holding());
    });
    let (_runtime, _handle, pilot) = mount(cx, options(DebugOptions::default()), 1280., 880.);
    cx.update(|cx| {
        let pilot = pilot.read(cx);
        assert!(!pilot.resources.read(cx).is_holding());
        assert!(!pilot.node_pods.read(cx).is_holding());
    });
}

#[gpui_kit::test]
fn the_app_menu_button_reaches_the_shell(cx: &mut TestAppContext) {
    let debug = DebugOptions {
        app_menu_button: true,
        ..DebugOptions::default()
    };
    let (_runtime, _handle, pilot) = mount(cx, options(debug), 1280., 880.);
    cx.update(|cx| assert_eq!(pilot.read(cx).menu_platform, Platform::Linux));
}
