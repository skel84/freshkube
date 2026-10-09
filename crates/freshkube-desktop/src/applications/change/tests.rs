//! The change page in the real app, on example data: checkout's Freight
//! `wonky-otter`, followed from its application page's Stage.
use super::super::{ApplicationsPage, Variant};
use super::ChangePage;
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, test::TestWindowExt,
};

const CHANGE: layout_check::TablePage = layout_check::TablePage {
    page: "change-page",
    title: "change-title",
    title_text: "wonky-otter",
    table: "change-table-scroll",
    list: "change-list",
};

const CHECKOUT: &str = "application-kargo:checkout";
/// checkout's Stage `prod-ams`, on its application page.
const PROD_AMS: &str = "application-part-core-fra/0/checkout/prod-ams";

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn change(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Option<Entity<ChangePage>> {
    list(view, cx).read(cx).change_page().cloned()
}

fn hop(key: &str) -> SharedString {
    format!("change-hop-{key}").into()
}

/// checkout's page with its Stage prod-ams selected, before following it.
fn on_stage(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// The change, followed from prod-ams with F.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = on_stage(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

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
        window.click("application-part-core-fra/1/checkout/checkout-images", cx);
        window.render_frame(cx);
        assert!(window.try_find("application-detail-follow").is_none());
        window.press("f", cx);
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_change_page_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.press("escape", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).applications().1, Page::Applications);
            assert!(change(&view, cx).is_some());
            let layout = layout_check::assert_table_page_from(window, cx, &CHANGE, "change-back");
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn hops_come_in_travel_order_with_fine_stages_folded(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx).lines_text(),
            [
                "# Build",
                "pr",
                "pr-run",
                "push-run",
                "signature",
                "policy",
                "image",
                "# Freight",
                "freight",
                // dev and stage are fine, so they fold while prod needs a
                // look; travel order stays.
                "# Stage dev",
                "# Stage stage",
                "# Stage prod-ams",
                "prod-ams-eligible",
                "prod-ams-promotion",
                "prod-ams-verification",
                "prod-ams-argocd",
                "prod-ams-pods",
                "# Stage prod-lon",
                "prod-lon-eligible",
                "prod-lon-promotion",
                "prod-lon-verification",
                "prod-lon-argocd",
                "prod-lon-pods",
            ]
        );
        assert!(window.find("change-showing").visible());
        assert_eq!(
            window.find("change-hop-pr-run-link").label(),
            Some("Claimed")
        );
        assert_eq!(
            window.find("change-hop-prod-ams-pods-link").label(),
            Some("Unknown")
        );
        // A gate has no link: it isn't joined to anything.
        assert!(
            window
                .try_find("change-hop-prod-ams-promotion-link")
                .is_none()
        );
        window.click("change-show-all", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        assert_eq!(
            lines.iter().filter(|line| !line.starts_with('#')).count(),
            27
        );
        assert!(window.try_find("change-showing").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fold_opens_and_closes_its_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        // Group 2 is the Stage dev.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        assert!(page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chip_shows_only_its_hops_in_every_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        window.click("change-tally-failing", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).lines_text(),
            ["# Stage prod-lon", "prod-lon-verification"]
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        // Folded Stages show their ok rows under a filter, and the hand
        // approval's blue dot counts as ok.
        assert!(lines.contains(&"dev-pods".to_owned()), "{lines:?}");
        assert!(lines.contains(&"prod-lon-eligible".to_owned()), "{lines:?}");
        assert!(
            !lines.contains(&"prod-ams-promotion".to_owned()),
            "{lines:?}"
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx)
                .lines_text()
                .contains(&"# Stage dev".to_owned())
        );
        assert!(!page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
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

#[gpui_kit::test]
fn the_inspector_sits_beside_a_wide_table_and_under_a_narrow_one(cx: &mut TestAppContext) {
    for (width, height) in [(1600., 900.), (760., 560.)] {
        for size in [None, Some(20.)] {
            let (_runtime, handle, _view) = open(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                if let Some(size) = size {
                    crate::text_size::set(size, cx);
                }
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "change-split",
                    "change-table",
                    "change-detail",
                    "change-detail-title",
                );
            })
            .unwrap();
        }
    }
}

/// #522: beside the Inspector, Detail ends at a whole word, with its whole
/// text in the tooltip.
#[gpui_kit::test]
fn detail_cuts_at_a_word_beside_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        let mut cut = 0;
        for row in &page.read(cx).rows {
            let id = SharedString::from(format!("change-hop-{}-detail", row.key));
            let Some(shown) = freshkube_ui::table::word_cut_shown(id) else {
                continue;
            };
            if shown == row.detail {
                continue;
            }
            cut += 1;
            let kept = shown.strip_suffix('…').expect("an ellipsis");
            assert!(row.detail.starts_with(kept), "{shown} of {}", row.detail);
            let next = row.detail[kept.len()..].chars().next();
            assert!(
                next.is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation() || c == '·'),
                "{shown:?} cuts a word of {:?}",
                row.detail
            );
        }
        assert!(cut > 0, "some Detail is cut beside the Inspector");
    })
    .unwrap();
}

#[gpui_kit::test]
fn o_opens_the_freight_in_resources(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("freight"), cx);
        window.render_frame(cx);
        window.press("o", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Resources);
        let identity = view.read(cx).opened_object(cx).expect("opened");
        assert_eq!(identity.resource, "freights.kargo.akuity.io");
        assert_eq!(identity.address(), "checkout/wonky-otter");
        // The rail's Applications comes back to the change.
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(window.find("change-page").visible());
    })
    .unwrap();
}

/// The pods of a Stage live on its own cluster, which no example context
/// opens: the button is greyed out and the menu item too.
#[gpui_kit::test]
fn a_hop_on_a_cluster_that_is_not_open_does_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-lon-pods"), cx);
        window.render_frame(cx);
        window.click("change-action-0", cx);
        window.render_frame(cx);
        window.right_click(hop("prod-lon-pods"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Open in Resources"));
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert!(view.read(cx).opened_object(cx).is_none());
        assert!(change(&view, cx).is_some());
    })
    .unwrap();
}

/// A step's logs and the tools' pages aren't linked yet: drawn greyed out,
/// they open nothing.
#[gpui_kit::test]
fn unlinked_actions_open_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("pr"), cx);
        window.render_frame(cx);
        assert!(window.find("change-action-0").visible());
        window.click("change-action-0", cx);
        window.render_frame(cx);
        assert!(change(&view, cx).is_some());
        assert_eq!(view.read(cx).applications().1, Page::Applications);
    })
    .unwrap();
}
