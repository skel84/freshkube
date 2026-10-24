//! The Routes page opening a route in Resources, through the shell: the
//! fixture's example routes must be the same objects Resources lists.
use super::*;

/// The shell on the Routes page, with `checkout` selected.
fn routes_page(cx: &mut TestAppContext, width: f32) -> (AnyWindowHandle, Entity<Pilot>) {
    let (_runtime, handle, pilot) = fixture(cx, width, 880.);
    // The runtime must outlive the window.
    std::mem::forget(_runtime);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-k8s-group-networking", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-routes", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.find("routes-title");
        window.click("routes-row-shop-checkout", cx);
        window.render_frame(cx);
    })
    .unwrap();
    (handle, pilot)
}

fn tab_count(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> usize {
    cx.read(|cx| pilot.read(cx).tabs.set.tabs().len())
}

#[gpui_kit::test]
fn every_way_to_open_a_trace_loads_it_in_the_same_tab(cx: &mut TestAppContext) {
    for how in ["enter", "t", "toolbar", "inspector"] {
        let (handle, pilot) = routes_page(cx, 1280.);
        let before = tab_count(cx, &pilot);
        cx.update_window(handle, |_, window, cx| match how {
            "enter" => window.press("enter", cx),
            "t" => window.press("t", cx),
            "toolbar" => window.click("routes-open-trace", cx),
            _ => window.click("routes-detail-trace", cx),
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).navigation.page, Page::Resources, "{how}");
            // The route is the object the list holds, shown in this tab.
            let opened = pilot
                .read(cx)
                .pages
                .resources
                .read(cx)
                .detail_identity(cx)
                .map(|identity| (identity.resource.clone(), identity.name.clone()));
            assert_eq!(
                opened,
                Some((
                    "httproutes.gateway.networking.k8s.io".into(),
                    "checkout".into()
                )),
                "{how}"
            );
            window.find("detail-tab-trace");
            assert!(window.try_find("detail-gone").is_none(), "{how}: deleted");
            window.find("detail-trace-page");
            window.find("route-trace-full");
        })
        .unwrap();
        assert_eq!(tab_count(cx, &pilot), before, "{how}: a new tab opened");
    }
}

#[gpui_kit::test]
fn open_route_shows_the_overview_with_the_compact_trace(cx: &mut TestAppContext) {
    for how in ["o", "toolbar", "inspector"] {
        let (handle, pilot) = routes_page(cx, 1280.);
        cx.update_window(handle, |_, window, cx| match how {
            "o" => window.press("o", cx),
            "toolbar" => window.click("routes-open-route", cx),
            _ => window.click("routes-detail-route", cx),
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).navigation.page, Page::Resources, "{how}");
            assert!(window.try_find("detail-gone").is_none(), "{how}: deleted");
            window.find("route-trace-compact");
        })
        .unwrap();
    }
}

/// A press and a release at the middle of `id`, with `modifiers` held.
fn click_with(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    id: &'static str,
    modifiers: gpui_kit::Modifiers,
) {
    use gpui_kit::{MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
    cx.update_window(handle, |_, window, cx| {
        let position = window.find(id).bounds().center();
        for input in [
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
            }),
        ] {
            window.dispatch_event(input, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}

/// The toolbar's Open trace follows the click's modifiers as every link
/// does: a plain click changes the shown tab, the platform modifier opens a
/// background one.
#[gpui_kit::test]
fn the_toolbar_button_opens_here_on_a_plain_click_and_in_the_background_with_the_modifier(
    cx: &mut TestAppContext,
) {
    let command = if cfg!(target_os = "macos") {
        gpui_kit::Modifiers::command()
    } else {
        gpui_kit::Modifiers::control()
    };
    let (handle, pilot) = routes_page(cx, 1280.);
    let before = tab_count(cx, &pilot);
    click_with(
        cx,
        handle,
        "routes-open-trace",
        gpui_kit::Modifiers::default(),
    );
    assert_eq!(tab_count(cx, &pilot), before, "a plain click opened a tab");
    cx.read(|cx| assert_eq!(pilot.read(cx).navigation.page, Page::Resources));

    let (handle, pilot) = routes_page(cx, 1280.);
    click_with(cx, handle, "routes-open-trace", command);
    assert_eq!(tab_count(cx, &pilot), before + 1);
    cx.read(|cx| assert_eq!(pilot.read(cx).navigation.page, Page::Routes));
}
