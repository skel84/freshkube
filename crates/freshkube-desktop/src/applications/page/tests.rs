//! An application's page in the real app, on example data.
use super::super::{ApplicationsPage, Variant};
use super::ApplicationPage;
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

const APPLICATION: layout_check::TablePage = layout_check::TablePage {
    page: "application-page",
    title: "application-title",
    title_text: "checkout",
    table: "application-table-scroll",
    list: "application-list",
};

const CHECKOUT: &str = "application-kargo:checkout";
/// checkout's first Stage, `dev` in `core-fra`.
const DEV_STAGE: &str = "application-part-core-fra/0/checkout/dev";
const WORKER: &str = "application-part-prod-fra/3/checkout/checkout-worker";

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn shown(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Option<Entity<ApplicationPage>> {
    list(view, cx).read(cx).open_page().cloned()
}

/// The list on example data, shaped before it first shows, with checkout
/// selected and opened with Enter.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    variant: Variant,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(variant));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

#[gpui_kit::test]
fn the_application_page_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            assert_eq!(view.read(cx).applications().1, Page::Applications);
            assert!(shown(&view, cx).is_some());
            let layout =
                layout_check::assert_table_page_from(window, cx, &APPLICATION, "application-back");
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn parts_come_in_kind_order_with_their_links(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let lines = shown(&view, cx).unwrap().read(cx).lines_text();
        assert_eq!(
            lines,
            [
                "# Kargo Stages",
                "dev Confirmed",
                "prod-ams Confirmed",
                "prod-fra Confirmed",
                "stage Confirmed",
                "# Kargo Warehouses",
                "checkout-images Confirmed",
                "# Argo CD Applications",
                "checkout-dev Claimed",
                "checkout-prod-ams Claimed",
                "checkout-prod-fra Claimed",
                "checkout-stage Claimed",
                // Argo CD on core-fra was read in argocd only, so the
                // Applications checkout's Stages promote to may be missing.
                "# May be missing on core-fra",
                "# Deployments",
                "checkout-api By label",
                "checkout-api By label",
                "checkout-worker By label",
            ]
        );
        assert!(
            window
                .find("application-group-missing-2-core-fra")
                .visible()
        );
        assert!(window.find("application-title").visible());
        let link = |row: &str| {
            window
                .find(gpui_kit::SharedString::from(format!("{row}-link")))
                .label()
                .map(str::to_owned)
        };
        assert_eq!(link(DEV_STAGE).as_deref(), Some("Confirmed"));
        assert_eq!(link(WORKER).as_deref(), Some("By label"));
        let legend = window
            .find("application-legend")
            .label()
            .unwrap()
            .to_owned();
        assert!(
            legend.contains("By label: only its part-of label says so"),
            "{legend}"
        );
        assert!(
            legend.contains("Unknown: a side couldn't be read"),
            "{legend}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_lower_claim_shows_on_its_part(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(WORKER, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("application-detail-title").label(),
            Some("checkout-worker")
        );
        assert_eq!(
            window.find("application-detail-lower-0").label(),
            Some("part-of label would put it in checkout")
        );
        let link = window.find("application-detail-link");
        assert!(link.visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_part_whose_project_was_not_read_is_unknown(cx: &mut TestAppContext) {
    // Kargo refused: checkout-dev names a Project nobody read.
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Partial));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.double_click("application-argocd:application/argocd/checkout-dev", cx);
        window.render_frame(cx);
        let lines = shown(&view, cx).unwrap().read(cx).lines_text();
        assert_eq!(lines, ["# Argo CD Applications", "checkout-dev Unknown"]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn stages_that_did_not_answer_are_a_group_row_not_an_empty_group(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Stages);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let lines = shown(&view, cx).unwrap().read(cx).lines_text();
        assert_eq!(lines[0], "# May be missing on core-fra");
        assert_eq!(lines[1], "# Kargo Warehouses");
        assert!(
            !lines.iter().any(|line| line == "# Kargo Stages"),
            "{lines:?}"
        );
        let group = window.find("application-group-missing-0-core-fra");
        assert!(group.visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_row_menu_opens_an_application(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.right_click(CHECKOUT, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Selected as an arrow selects, with the Inspector left closed.
        assert!(window.try_find("applications-detail").is_none());
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Open"));
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(shown(&view, cx).is_some());
        assert!(window.find("application-title").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_breadcrumb_and_escape_go_back_to_the_list_with_its_selection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    // Each step's event reaches the list once the update ends.
    let mut step = |act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(&|window, cx| window.click("application-back", cx));
    step(&|window, cx| {
        window.render_frame(cx);
        assert!(shown(&view, cx).is_none());
        assert_eq!(
            window.find("applications-detail-title").label(),
            Some("checkout")
        );
        // Enter opens it again.
        window.press("enter", cx);
    });
    step(&|window, cx| {
        assert!(shown(&view, cx).is_some());
        window.press("down", cx);
    });
    step(&|window, cx| {
        assert!(shown(&view, cx).unwrap().read(cx).selected().is_some());
        // Escape clears the part, then goes back.
        window.press("escape", cx);
    });
    step(&|window, cx| {
        assert!(shown(&view, cx).unwrap().read(cx).selected().is_none());
        window.press("escape", cx);
    });
    step(&|window, cx| {
        window.render_frame(cx);
        assert!(shown(&view, cx).is_none());
        assert_eq!(
            window.find("applications-detail-title").label(),
            Some("checkout")
        );
    });
}

#[gpui_kit::test]
fn another_context_closes_the_application(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, _, cx| {
        assert!(shown(&view, cx).is_some());
        list(&view, cx).update(cx, |page, cx| page.set_source(None, cx));
        assert!(shown(&view, cx).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspector_sits_beside_a_wide_table_and_under_a_narrow_one(cx: &mut TestAppContext) {
    for (width, height) in [(1600., 900.), (760., 560.)] {
        for size in [None, Some(20.)] {
            let (_runtime, handle, _view) = open(cx, width, height, Variant::Acme);
            cx.update_window(handle, |_, window, cx| {
                if let Some(size) = size {
                    crate::text_size::set(size, cx);
                }
                window.render_frame(cx);
                window.click(DEV_STAGE, cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "application-split",
                    "application-table",
                    "application-detail",
                    "application-detail-title",
                );
            })
            .unwrap();
        }
    }
}
