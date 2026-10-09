//! An application's page in the real app, on example data.
use super::super::{ApplicationsPage, Variant};
use super::ApplicationPage;
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

use std::time::Duration;

use freshkube_ui::table::TableColumn as _;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, test::TestWindowExt};

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
const WORKER: &str = "application-part-prod-lon/3/checkout/checkout-worker";

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
                "prod-lon Confirmed",
                "stage Confirmed",
                "# Kargo Warehouses",
                "checkout-images Confirmed",
                "# Argo CD Applications",
                "checkout-dev Claimed",
                "checkout-prod-ams Claimed",
                "checkout-prod-lon Claimed",
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
            Some("Its app.kubernetes.io/part-of label puts it in checkout.")
        );
        let link = window.find("application-detail-link");
        assert!(link.visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_lower_claim_stays_in_whys_value_column(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 760., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(WORKER, cx);
        window.render_frame(cx);
        let why = window.find("application-detail-why").bounds();
        let lower = window.find("application-detail-lower-0").bounds();
        let link = window.find("application-detail-link").bounds();
        // Past the label column, under Why's own words.
        assert!(why.left() > link.left() + px(100.), "{why:?} in {link:?}");
        assert_eq!(lower.left(), why.left());
        assert!(lower.top() > why.top(), "{lower:?} under {why:?}");
    })
    .unwrap();
}

/// At 760 the columns are wider than the page: the table scrolls sideways
/// with its header, so Read from comes into view rather than staying cut off.
#[gpui_kit::test]
fn a_narrow_page_scrolls_the_table_sideways_to_read_from(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    let (_runtime, handle, view) = open(cx, 760., 560., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = shown(&view, cx).unwrap();
        let (columns, width) = {
            let page = page.read(cx);
            (page.columns.len(), page.width)
        };
        let read_from = ("application-sort", columns - 1);
        let viewport = window.find("application-table-scroll").bounds();
        assert!(
            crate::ui::dp_px(width, window) > viewport.size.width,
            "{width} fits in {viewport:?}"
        );
        let before = window.find(read_from).bounds();
        assert!(
            before.right() > viewport.right(),
            "{before:?} in {viewport:?}"
        );
        window.scroll(
            "application-table-scroll",
            ScrollDelta::Pixels(point(px(-400.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let after = window.find(read_from).bounds();
        assert!(after.left() < before.left(), "{after:?} from {before:?}");
        assert!(
            after.right() <= viewport.right() + px(1.),
            "{after:?} in {viewport:?}"
        );
    })
    .unwrap();
}

/// Read from is as wide as its words, up to the widest column, so what
/// wasn't checked shows in full once scrolled to.
#[gpui_kit::test]
fn read_from_fits_what_was_not_checked(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 760., 560., Variant::Acme);
    cx.update_window(handle, |_, _, cx| {
        let page = shown(&view, cx).unwrap();
        let mut rows = page.read(cx).rows.clone();
        let words = |rows: &[super::PartRow]| {
            let (columns, _) = super::table::columns(rows);
            let read_from = columns.last().unwrap();
            assert_eq!(read_from.label().as_ref(), "Read from");
            read_from.width()
        };
        let short = words(&rows);
        rows[0].read_from = "Argo CD read in gitops only on core-fra".into();
        // 39 characters would be 316.5; the widest column is 280.
        assert_eq!(words(&rows), 280.);
        assert!(short < words(&rows));
    })
    .unwrap();
}

/// At 760 and text size 20 the Inspector stacks under the table. The split
/// settles on the frames the app asks for, so a sideways wheel over the table
/// only moves its columns: the table keeps its height and the page stays put.
#[gpui_kit::test]
fn a_sideways_wheel_over_the_stacked_table_keeps_its_height(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    let (_runtime, handle, view) = open(cx, 760., 560., Variant::Acme);
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.run_until_parked();
    // Draw only the frames the app asks for, as the screen does.
    let settle = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        for _ in 0..8 {
            if window.simulate_next_frame(cx) == 0 {
                return;
            }
            window.render_frame(cx);
        }
        panic!("the split never settles");
    };
    let table = |window: &mut gpui_kit::Window| window.find("application-table").bounds();
    let object = ("application-sort", 1usize);
    let read_from = |page: &Entity<ApplicationPage>, cx: &gpui_kit::App| {
        ("application-sort", page.read(cx).columns.len() - 1)
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        settle(window, cx);
        window.click(DEV_STAGE, cx);
        settle(window, cx);
        assert!(window.try_find("application-detail").is_some(), "stacked");
        let settled = table(window);
        let page = shown(&view, cx).unwrap();
        let column = read_from(&page, cx);
        let before = window.find(column).bounds();
        // A wheel draws a frame of its own; the split was settled already.
        window.render_frame(cx);
        assert_eq!(
            table(window),
            settled,
            "the split moved on a frame of its own"
        );
        window.scroll(
            "application-table-scroll",
            ScrollDelta::Pixels(point(px(-600.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        settle(window, cx);
        assert_eq!(table(window), settled, "the wheel moved the split");
        assert!(
            window.find(column).bounds().left() < before.left(),
            "the columns didn't scroll"
        );
        assert_eq!(
            page.read(cx).page_scroll.offset().y,
            px(0.),
            "the page scrolled"
        );
        let pinned = window.find(object).bounds();
        assert!(
            pinned.left() >= settled.left() - px(1.),
            "{pinned:?} in {settled:?}"
        );
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

#[gpui_kit::test]
fn the_page_and_the_inspector_say_what_the_application_is(cx: &mut TestAppContext) {
    // A Kargo Project and an Argo CD Application may share a name.
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = shown(&view, cx).unwrap();
        assert_eq!(page.read(cx).what().as_ref(), "Kargo Project");
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let what = |window: &mut gpui_kit::Window| {
            window
                .find("applications-detail-what")
                .label()
                .map(str::to_owned)
        };
        window.click(CHECKOUT, cx);
        window.render_frame(cx);
        assert_eq!(what(window).as_deref(), Some("Kargo Project"));
        window.click("application-argocd:applicationset/argocd/catalog", cx);
        window.render_frame(cx);
        assert_eq!(what(window).as_deref(), Some("Argo CD ApplicationSet"));
    })
    .unwrap();
}

#[test]
fn what_was_not_checked_names_its_cluster_and_namespaces() {
    use freshkube_core::applications::claims::Unchecked;
    let words = |unchecked| super::unchecked_words(&unchecked, "core-fra");
    assert_eq!(
        words(Unchecked::Namespaces(vec!["gitops".into()])),
        "Argo CD read in gitops only on core-fra"
    );
    assert_eq!(words(Unchecked::NotRead), "Argo CD not read on core-fra");
    assert_eq!(
        words(Unchecked::NotFound("argocd".into())),
        "Argo CD not found on core-fra: argocd holds no Applications"
    );
    assert_eq!(
        words(Unchecked::Capped(500)),
        "Argo CD read in part on core-fra: stopped after 500"
    );
}

/// Reads the list again with the example reshaped.
fn read_again(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    variant: Variant,
) {
    cx.update_window(handle, |_, window, cx| {
        list(view, cx).update(cx, |page, cx| {
            page.set_variant(variant);
            page.refresh(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn list_has_keys(view: &Entity<Pilot>, window: &gpui_kit::Window, cx: &gpui_kit::App) -> bool {
    list(view, cx).read(cx).focus.is_focused(window)
}

#[gpui_kit::test]
fn a_read_without_the_application_closes_its_page_and_gives_the_list_the_keys(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        let page = shown(&view, cx).unwrap();
        assert!(page.read(cx).focus_handle().is_focused(window));
    })
    .unwrap();
    // Nothing served: checkout is gone.
    read_again(cx, handle, &view, Variant::Unserved);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(shown(&view, cx).is_none());
        assert!(window.find("applications-none").visible());
        assert!(list_has_keys(&view, window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_read_updates_the_open_page_and_keeps_its_selection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    let warehouse = "application-part-core-fra/1/checkout/checkout-images";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(warehouse, cx);
        window.render_frame(cx);
    })
    .unwrap();
    // checkout's Stages don't answer this time.
    read_again(cx, handle, &view, Variant::Stages);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = shown(&view, cx).expect("checkout is still there");
        let lines = page.read(cx).lines_text();
        assert_eq!(lines[0], "# May be missing on core-fra", "{lines:?}");
        assert_eq!(
            page.read(cx).selected().map(|key| key.as_ref()),
            Some(warehouse.trim_start_matches("application-part-"))
        );
        assert_eq!(
            window.find("application-detail-title").label(),
            Some("checkout-images")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_read_again_shows_the_last_read_on_the_page_too(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("application-stale").is_none());
        list(&view, cx).update(cx, |page, cx| {
            page.fail_read("connection refused by core-fra.example.test:6443", cx)
        });
        window.render_frame(cx);
        assert!(shown(&view, cx).is_some(), "the last read still has it");
        let stale = window.find("application-stale");
        assert!(stale.visible());
        assert!(stale.label().unwrap().contains("connection refused"));
    })
    .unwrap();
    // A read that answers clears it.
    read_again(cx, handle, &view, Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("application-stale").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_context_closes_the_page_and_its_late_read_never_lands(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    // The old context's read again is in flight, due in 5 s.
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_example_delay(Duration::from_secs(5));
            page.refresh(cx);
        });
        window.render_frame(cx);
        list(&view, cx).update(cx, |page, _| page.set_hold(true));
        view.update(cx, |view, cx| view.choose_context("staging-eu", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = list(&view, cx);
        assert!(shown(&view, cx).is_none());
        assert!(
            !page.read(cx).has_read(),
            "the old context's read never lands"
        );
        assert!(page.read(cx).is_reading(), "staging-eu's own read waits");
        assert!(window.try_find(CHECKOUT).is_none());
        assert!(list_has_keys(&view, window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn labelled_parts_that_may_be_missing_are_a_group_row(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Workloads);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let lines = shown(&view, cx).unwrap().read(cx).lines_text();
        assert!(
            lines
                .iter()
                .any(|line| line == "# May be missing on prod-lon"),
            "{lines:?}"
        );
        // prod-lon's worker wasn't read, so it isn't a part.
        assert!(!lines.iter().any(|line| line.starts_with("checkout-worker")));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_workload_naming_its_application_back_is_confirmed(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.double_click("application-argocd:application/argocd/status-page", cx);
        window.render_frame(cx);
        let lines = shown(&view, cx).unwrap().read(cx).lines_text();
        assert!(
            lines.iter().any(|line| line == "# Deployments"),
            "{lines:?}"
        );
        let deployment = lines
            .iter()
            .skip_while(|line| *line != "# Deployments")
            .nth(1)
            .unwrap();
        assert!(deployment.ends_with(" Confirmed"), "{lines:?}");
    })
    .unwrap();
}

/// checkout's Argo CD Application for `dev`, in `core-fra`.
const DEV_APPLICATION: &str = "application-part-core-fra/2/argocd/checkout-dev";
/// checkout's Warehouse, in `core-fra`.
const WAREHOUSE: &str = "application-part-core-fra/1/checkout/checkout-images";

/// What Resources shows: its kind's key, the object's address and the
/// connection it was read through, once the shell opened a part.
fn opened(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Option<(String, String, String)> {
    let identity = view.read(cx).opened_object(cx)?;
    assert!(!identity.uid.is_empty(), "opened by its identity");
    Some((
        identity.resource.clone(),
        identity.address(),
        identity.connection,
    ))
}

fn notices(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> usize {
    use gpui_kit::component::WindowExt as _;
    window.notifications(cx).len()
}

/// Clicks a part, then acts on it, and lets the shell open what it asked.
fn act_on(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    part: &'static str,
    act: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(handle, |_, window, cx| {
        window.click(part, cx);
        window.render_frame(cx);
        act(window, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn o_opens_a_core_fra_part_in_resources_and_back_keeps_the_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    act_on(cx, handle, DEV_STAGE, |window, cx| window.press("o", cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Resources);
        assert_eq!(
            opened(&view, cx),
            Some((
                "stages.kargo.akuity.io".into(),
                "checkout/dev".into(),
                "example:prod-fra".into()
            ))
        );
        // The rail's Applications comes back to the page and its part.
        window.click("nav-applications", cx);
        window.render_frame(cx);
        let page = shown(&view, cx).expect("the page stays open");
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some(DEV_STAGE.trim_start_matches("application-part-").into())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspectors_button_opens_an_argo_cd_application(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    act_on(cx, handle, DEV_APPLICATION, |window, cx| {
        let button = window.find("application-detail-open");
        assert!(button.visible());
        window.click("application-detail-open", cx);
    });
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            opened(&view, cx),
            Some((
                "applications.argoproj.io".into(),
                "argocd/checkout-dev".into(),
                "example:prod-fra".into()
            ))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_row_menu_opens_a_part_without_opening_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(WAREHOUSE, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = shown(&view, cx).unwrap();
        assert!(page.read(cx).selected().is_some());
        assert!(!page.read(cx).inspects());
        assert!(window.try_find("application-detail").is_none());
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Open in Resources"));
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            opened(&view, cx),
            Some((
                "warehouses.kargo.akuity.io".into(),
                "checkout/checkout-images".into(),
                "example:prod-fra".into()
            ))
        );
    })
    .unwrap();
}

/// acme's `prod-lon` is a cluster no example context opens:
/// its link names its bare key, and the shell refuses it with the notice
/// every link to a cluster that isn't open gets.
#[gpui_kit::test]
fn a_part_in_an_unmapped_acme_cluster_is_refused_and_nothing_moves(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    let before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            notices(window, cx)
        })
        .unwrap();
    // The button is greyed out with why: a click opens nothing and says
    // nothing.
    act_on(cx, handle, WORKER, |window, cx| {
        window.click("application-detail-open", cx);
    });
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(notices(window, cx), before);
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        let button = window.find("application-detail-open");
        assert!(button.visible());
        // O sends the link anyway, and the shell says why it can't.
        window.press("o", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(notices(window, cx), before + 1, "the refusal says so");
        let page = shown(&view, cx).expect("the page stays");
        assert_eq!(
            page.read(cx)
                .selected_row()
                .map(|row| row.open_tip.to_string()),
            Some("prod-lon isn't the open cluster, so its objects don't open in Resources".into())
        );
        assert_eq!(
            view.read(cx).told.last().map(String::as_str),
            Some("Can’t open checkout-worker: it belongs to a cluster that isn’t open")
        );
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert_eq!(opened(&view, cx), None);
        let page = shown(&view, cx).expect("the page stays");
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some(WORKER.trim_start_matches("application-part-").into())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_row_menu_greys_out_a_part_in_a_cluster_that_is_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Variant::Acme);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(WORKER, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert_eq!(opened(&view, cx), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn core_fra_follows_the_open_context(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |view, cx| view.choose_context("staging-eu", window, cx));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(CHECKOUT, cx);
        window.render_frame(cx);
    })
    .unwrap();
    act_on(cx, handle, DEV_STAGE, |window, cx| window.press("o", cx));
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            opened(&view, cx),
            Some((
                "stages.kargo.akuity.io".into(),
                "checkout/dev".into(),
                "example:staging-eu".into()
            ))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn status_pages_deployment_opens_in_resources(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.double_click("application-argocd:application/argocd/status-page", cx);
        window.render_frame(cx);
    })
    .unwrap();
    act_on(
        cx,
        handle,
        "application-part-core-fra/3/status/status-page",
        |window, cx| window.press("o", cx),
    );
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            opened(&view, cx),
            Some((
                "deployments.apps".into(),
                "status/status-page".into(),
                "example:prod-fra".into()
            ))
        );
    })
    .unwrap();
}
