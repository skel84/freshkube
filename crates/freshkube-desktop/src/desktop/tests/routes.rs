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
