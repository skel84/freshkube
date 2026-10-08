//! The Applications page in the real app, on example data.
use super::{ApplicationsPage, Variant};
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

const APPLICATIONS: layout_check::TablePage = layout_check::TablePage {
    page: "applications-page",
    title: "applications-title",
    title_text: "Applications",
    table: "applications-table-scroll",
    list: "applications-list",
};

const CART: &str = "application-kargo:cart";
const CATALOG: &str = "application-argocd:applicationset/argocd/catalog";
const LOYALTY: &str = "application-part-of:loyalty";

fn page(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

/// The app on example data, with the page shaped before it first shows.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: impl FnOnce(&mut ApplicationsPage),
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        page(&view, cx).update(cx, |page, _| shape(page));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

#[gpui_kit::test]
fn applications_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |_| {});
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            assert_eq!(view.read(cx).applications().1, Page::Applications);
            let layout = layout_check::assert_table_page(window, cx, &APPLICATIONS);
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn acme_lists_every_cluster_grouped_by_the_rule_that_found_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |_| {});
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = page(&view, cx);
        assert_eq!(
            page.read(cx).names(),
            ["cart", "checkout", "catalog", "status-page", "loyalty"]
        );
        for group in [
            "applications-group-kargo-projects",
            "applications-group-argo-cd",
            "applications-group-part-of-label",
        ] {
            assert!(window.find(group).visible(), "{group}");
        }
        assert!(window.find(CART).visible());
        assert!(window.find(LOYALTY).visible());
        // Five clusters serve neither Kargo nor Argo CD; argocd is read
        // alone: facts under the header, not warnings.
        let legend = window
            .find("applications-legend")
            .label()
            .unwrap()
            .to_owned();
        assert!(
            legend.contains("Kargo isn't served on 5 clusters"),
            "{legend}"
        );
        assert!(
            legend.contains("Argo CD read in argocd only on core-fra"),
            "{legend}"
        );
        assert!(window.try_find("applications-missing").is_none());
        let status = page.read(cx).status.clone();
        assert!(
            format!("{status:?}").contains("5 applications in 6 clusters"),
            "{status:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn core_fra_alone_shows_what_one_cluster_holds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |page| page.set_variant(Variant::Single));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names = page(&view, cx).read(cx).names();
        assert!(names.contains(&"cart".to_owned()), "{names:?}");
        // loyalty is labelled in dev-fra, which isn't read.
        assert!(!names.contains(&"loyalty".to_owned()), "{names:?}");
        assert!(window.try_find(LOYALTY).is_none());
        let status = format!("{:?}", page(&view, cx).read(cx).status);
        assert!(status.contains("in 1 cluster"), "{status}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn reads_only_while_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = page(&view, cx);
        page.update(cx, |page, _| page.set_hold(true));
        window.render_frame(cx);
        assert!(!page.read(cx).has_read(), "nothing is read while hidden");
        assert!(!page.read(cx).is_reading());

        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(page.read(cx).is_reading());
        assert!(
            window
                .within("applications-list")
                .find("applications-loading")
                .visible()
        );
        assert!(
            view.read(cx)
                .applications()
                .0
                .read(cx)
                .loading_motion()
                .is_some()
        );

        // Hiding drops the read in flight.
        window.click("nav-overview", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).is_reading());
        assert!(!page.read(cx).has_read());

        // Shown again with nothing read, it reads again; answered, it
        // shows the rows.
        page.update(cx, |page, _| page.set_hold(false));
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(page.read(cx).has_read());
        assert!(!page.read(cx).is_reading());
        assert!(window.find(CART).visible());
        assert!(window.try_find("applications-loading").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fresh_read_is_shown_again_without_reading(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |_| {});
    cx.update_window(handle, |_, window, cx| {
        let page = page(&view, cx);
        window.click("nav-overview", cx);
        // Held now, a read would stay in flight: it must not start.
        page.update(cx, |page, _| page.set_hold(true));
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).is_reading());
        assert!(window.find(CART).visible());

        // Refresh reads again, whatever is fresh.
        window.click("applications-refresh", cx);
        window.render_frame(cx);
        assert!(page.read(cx).is_reading());
        // What was read stays shown while the next read runs.
        assert!(window.find(CART).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_stale_read_is_read_again_when_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |_| {});
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        page(&view, cx).update(cx, |page, _| page.set_hold(true));
    })
    .unwrap();
    cx.executor().advance_clock(super::FRESH_FOR);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(page(&view, cx).read(cx).is_reading());
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_context_drops_the_read_and_the_selection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |_| {});
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(CART, cx);
        window.render_frame(cx);
        assert!(window.find("applications-detail").visible());
        page(&view, cx).update(cx, |page, _| page.set_hold(true));
        view.update(cx, |view, cx| view.choose_context("staging-eu", window, cx));
        window.render_frame(cx);
        let page = page(&view, cx);
        assert!(!page.read(cx).has_read(), "staging-eu's read is its own");
        assert!(page.read(cx).is_reading(), "shown, so it reads at once");
        assert!(window.try_find("applications-detail").is_none());
        assert!(window.try_find(CART).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_read_with_nothing_found_is_never_empty(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) =
        open(cx, 1280., 880., |page| page.set_variant(Variant::Refused));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let state = window.find("applications-refused");
        assert!(state.visible());
        assert!(window.try_find("applications-none").is_none());
        assert!(window.try_find("applications-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn nothing_served_anywhere_is_the_empty_state(cx: &mut TestAppContext) {
    let (_runtime, handle, view) =
        open(cx, 1280., 880., |page| page.set_variant(Variant::Unserved));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("applications-none").visible());
        assert!(matches!(
            page(&view, cx).read(cx).body(),
            super::display::Body::Empty { .. }
        ));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_cluster_that_didnt_answer_fails_with_retry(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |page| page.set_variant(Variant::Failed));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("applications-failed").visible());
        page(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        window.click("applications-retry", cx);
        window.render_frame(cx);
        assert!(window.try_find("applications-failed").is_none());
        assert!(window.find(CART).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_source_beside_found_applications_says_they_may_be_missing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |page| page.set_variant(Variant::Partial));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let missing = window
            .find("applications-missing")
            .label()
            .unwrap()
            .to_owned();
        assert!(
            missing.contains("Kargo Projects were refused on core-fra"),
            "{missing}"
        );
        assert!(window.try_find(CART).is_none());
        assert!(window.find(CATALOG).visible());
        assert!(
            window
                .try_find("applications-group-kargo-projects")
                .is_none()
        );
        let names = page(&view, cx).read(cx).names();
        assert!(names.contains(&"loyalty".to_owned()), "{names:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_capped_source_marks_its_applications_incomplete(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880., |page| page.set_variant(Variant::Capped));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("applications-missing").visible());
        window.click("applications-tally-incomplete", cx);
        window.render_frame(cx);
        assert!(window.find(CATALOG).visible());
        assert!(window.try_find(LOYALTY).is_none());
        let mark = gpui_kit::SharedString::from(format!("{CATALOG}-mark"));
        assert_eq!(
            window.find(mark).label(),
            Some("May be incomplete: a source it depends on wasn't read in full")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_filter_and_chips_narrow_the_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., |_| {});
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("applications-filter", cx);
        window.input("dev-fra", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names = page(&view, cx).read(cx).names();
        assert!(names.contains(&"loyalty".to_owned()), "{names:?}");
        assert!(!names.contains(&"status-page".to_owned()), "{names:?}");
        window.click("applications-filter", cx);
        window.input("no-such-application", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("applications-empty").visible());
        assert!(page(&view, cx).read(cx).names().is_empty());
        assert!(window.try_find(CART).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_row_opens_the_inspector_and_escape_closes_it(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880., |_| {});
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("applications-detail").is_none());
        window.click(CART, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("applications-detail-title").label(),
            Some("cart")
        );
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("applications-detail-title").label(),
            Some("checkout")
        );
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("applications-detail").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspector_sits_beside_a_wide_table_and_under_a_narrow_one(cx: &mut TestAppContext) {
    for (width, height) in [(1600., 900.), (1000., 880.)] {
        let (_runtime, handle, _view) = open(cx, width, height, |_| {});
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(CART, cx);
            layout_check::assert_inspector(
                window,
                cx,
                "applications-split",
                "applications-table",
                "applications-detail",
                "applications-detail-title",
            );
        })
        .unwrap();
    }
}
