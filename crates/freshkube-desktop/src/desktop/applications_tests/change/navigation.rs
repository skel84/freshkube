use super::*;

#[gpui_kit::test]
fn a_stage_offers_its_freight_and_follows_it_onto_its_gates(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let button = window.find("application-detail-follow");
        assert!(button.visible());
        window.click("application-detail-follow", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change shows");
        assert_eq!(page.read(cx).freight(), "wonky-otter");
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some("prod-ams-eligible".into())
        );
        assert!(window.find("change-page").visible());
        assert!(window.try_find("application-page").is_none());
        let title = window.find("change-detail-title");
        assert_eq!(
            title.label(),
            Some("Kargo Stage prod-ams: Waiting for promotion")
        );
        assert!(window.find("change-detail-promotion").visible());
        assert!(window.find("change-detail-verification").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_warehouse_offers_nothing_to_follow(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("application-part-cicd-fra/1/checkout/checkout-images", cx);
        window.render_frame(cx);
        assert!(window.try_find("application-detail-follow").is_none());
        window.press("f", cx);
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hop_shows_its_link_and_a_gate_its_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("pr-run"), cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find("change-detail-link")
                .label()
                .map(|label| label.starts_with("Claimed by")),
            Some(true)
        );
        window.click(hop("prod-lon-verification"), cx);
        window.render_frame(cx);
        let title = window
            .find("change-detail-title")
            .label()
            .unwrap()
            .to_owned();
        assert!(title.starts_with("Kargo Stage prod-lon"), "{title}");
        assert!(window.find("change-detail-analyses").visible());
        window.click(hop("prod-ams-pods"), cx);
        window.render_frame(cx);
        let link = window
            .find("change-detail-link")
            .label()
            .unwrap()
            .to_owned();
        assert!(link.starts_with("Unknown"), "{link}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_arrows_step_through_hops_and_escape_goes_back(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some("prod-ams-promotion".into())
        );
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected().is_none());
        assert!(window.try_find("change-detail").is_none());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        let app = list(&view, cx)
            .read(cx)
            .open_page()
            .cloned()
            .expect("back on the app");
        assert_eq!(
            app.read(cx).selected().map(|key| key.to_string()),
            Some(PROD_AMS.trim_start_matches("application-part-").into())
        );
        // The application page has the keys again.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(app.read(cx).selected().is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_breadcrumb_goes_back_to_the_application(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-back", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        assert!(window.find("application-page").visible());
    })
    .unwrap();
}

/// Opening another application shows its page, not the change followed
/// from checkout's.
#[gpui_kit::test]
fn opening_another_application_drops_the_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(change(&view, cx).is_some());
        let page = list(&view, cx);
        let other = page
            .read(cx)
            .display
            .rows
            .iter()
            .map(|row| row.name.to_string())
            .find(|name| name != "checkout")
            .expect("acme has another application");
        page.update(cx, |page, cx| page.open_named(&other, window, cx));
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        let open = list(&view, cx).read(cx).open_page().cloned().unwrap();
        assert_eq!(open.read(cx).name().as_ref(), other);
        assert!(window.try_find("change-page").is_none());
    })
    .unwrap();
}

/// A read that no longer has checkout closes its page and the change on
/// it, and the list takes the keys.
#[gpui_kit::test]
fn a_read_without_the_application_closes_the_change_too(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_variant(Variant::Unserved);
            page.refresh(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        assert!(list(&view, cx).read(cx).open_page().is_none());
        assert!(window.try_find("change-page").is_none());
        assert!(list(&view, cx).read(cx).focus.is_focused(window));
    })
    .unwrap();
}

/// A Stage's row menu offers its Freight to follow, as F does.
#[gpui_kit::test]
fn the_row_menu_follows_a_stages_freight(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.right_click(PROD_AMS, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(1usize).label(), Some("Follow wonky-otter"));
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change shows");
        assert_eq!(
            page.read(cx).selected().map(|key| key.as_ref()),
            Some("prod-ams-eligible")
        );
        assert!(window.find("change-page").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_spread_fixture_follows_applications_and_names_the_skipped_namespaces(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., Variant::Spread);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change opens");
        let data = page.read(cx).loader.data().expect("the example answered");
        let dev = data
            .hops
            .iter()
            .find(|hop| hop.key == "dev-argocd")
            .unwrap();
        assert!(dev.detail.contains("gitops"));
        assert_eq!(dev.link, Some(freshkube_delivery::Confidence::Confirmed));
        let missing = window.find("change-missing").label().unwrap().to_owned();
        assert!(missing.contains("delivery, gitops, platform"), "{missing}");
        assert!(missing.contains("sandbox, tenants"), "{missing}");
        assert!(missing.contains("namespace cap"), "{missing}");
    })
    .unwrap();
}
