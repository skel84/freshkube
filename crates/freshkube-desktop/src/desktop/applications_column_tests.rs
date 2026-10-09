//! The Applications column (docs/DESIGN.md, "The column as a source
//! list"): All applications, then each application under the rule that
//! found it, or why there are none.
use super::shell::ColumnKey;
use super::tests::{chrome_redrawn, fixture};
use super::{Area, Page, Pilot};
use crate::applications::{Column, ColumnApp, ColumnSection, Variant};
use freshkube_core::applications::Rule;
use freshkube_ui::source_list::Line;
use freshkube_ui::table::TableSource as _;
use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, SharedString, TestAppContext};

const CART: &str = "nav-application-kargo:cart";
const ALL: &str = "nav-all-applications";

/// The shell on the Applications area, its example read in.
fn applications(
    cx: &mut TestAppContext,
    variant: Variant,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = view.read(cx).applications.clone();
        page.update(cx, |page, _| page.set_variant(variant));
        view.update(cx, |pilot, cx| {
            pilot.show_area(Area::Applications, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (runtime, handle, view)
}

/// The column's lines as (id, key) pairs: a row's key, `None` for the rest.
fn lines(view: &Entity<Pilot>, cx: &mut TestAppContext) -> Vec<(String, Option<ColumnKey>)> {
    cx.update(|cx| {
        view.read(cx)
            .column_list
            .lines()
            .iter()
            .map(|line| {
                (
                    line.id().to_string(),
                    line.row().map(|row| row.key().clone()),
                )
            })
            .collect()
    })
}

#[gpui_kit::test]
fn the_column_lists_each_application_under_its_rule(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let ids: Vec<String> = lines(&view, cx).into_iter().map(|(id, _)| id).collect();
    assert_eq!(
        ids,
        [
            ALL,
            "nav-applications-kargo",
            CART,
            "nav-application-kargo:checkout",
            "nav-applications-argocd",
            "nav-application-argocd:applicationset/argocd/catalog",
            "nav-application-argocd:application/argocd/status-page",
            "nav-applications-part-of",
            "nav-application-part-of:loyalty",
        ],
        "in the table's order, and no overrides section without one"
    );
    cx.update_window(handle, |_, window, cx| {
        let rows = view.read(cx).column_list.lines();
        let all = rows[0].row().unwrap();
        assert_eq!(all.counted().map(|c| c.as_ref()), Some("5"));
        assert!(all.is_current(), "the list shows");
        let cart = rows[2].row().unwrap();
        assert!(cart.tip().contains("Kargo Project · core-fra, dev-fra"));
        assert_eq!(window.find(ALL).selected(), Some(true));
        assert_eq!(window.find("nav-applications-kargo").selected(), None);
    })
    .unwrap();
}

/// A row opens its application's page and takes the fill; All
/// applications goes back to the list with that application selected.
#[gpui_kit::test]
fn a_row_opens_its_application_and_all_applications_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let page = cx.update(|cx| view.read(cx).applications.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click(CART, cx);
        window.render_frame(cx);
        let shown = page
            .read(cx)
            .open_page()
            .map(|open| open.read(cx).id().as_str().to_owned());
        assert_eq!(shown.as_deref(), Some("kargo:cart"));
        assert_eq!(window.find(CART).selected(), Some(true));
        assert_ne!(window.find(ALL).selected(), Some(true));
        let focus = page.read(cx).open_page().unwrap().read(cx).focus_handle();
        assert!(focus.is_focused(window), "the page has the keyboard");
        // Another row swaps the page.
        window.click("nav-application-part-of:loyalty", cx);
        window.render_frame(cx);
        let shown = page
            .read(cx)
            .open_page()
            .map(|open| open.read(cx).id().as_str().to_owned());
        assert_eq!(shown.as_deref(), Some("part-of:loyalty"));
        assert_ne!(window.find(CART).selected(), Some(true));
        window.click(ALL, cx);
        window.render_frame(cx);
        assert!(page.read(cx).open_page().is_none());
        assert_eq!(
            page.read(cx).selected_key().map(|key| key.as_ref()),
            Some("part-of:loyalty"),
            "the list keeps the application that showed"
        );
        assert_eq!(window.find(ALL).selected(), Some(true));
    })
    .unwrap();
    // The page's breadcrumb back moves the fill too.
    cx.update_window(handle, |_, window, cx| {
        window.click(CART, cx);
        window.render_frame(cx);
        window.click("application-back", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(ALL).selected(), Some(true));
        assert_ne!(window.find(CART).selected(), Some(true));
    })
    .unwrap();
}

/// The keyboard passes over the section labels, and Enter opens the
/// application with the keyboard on its page.
#[gpui_kit::test]
fn the_column_opens_an_application_from_the_keyboard(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let focus = cx.update(|cx| view.read(cx).column_list.focus_handle().clone());
    cx.update_window(handle, |_, window, cx| {
        window.focus(&focus, cx);
        window.render_frame(cx);
        let cursor = |cx: &gpui_kit::App| view.read(cx).column_list.cursor().cloned();
        assert_eq!(cursor(cx), Some(ColumnKey::AllApplications));
        window.press("down", cx);
        assert_eq!(
            cursor(cx),
            Some(ColumnKey::Application("kargo:cart".into()))
        );
        window.press("down", cx);
        window.press("down", cx);
        assert_eq!(
            cursor(cx),
            Some(ColumnKey::Application(
                "argocd:applicationset/argocd/catalog".into()
            )),
            "past the Argo CD label"
        );
        window.press("enter", cx);
        window.render_frame(cx);
        let page = view.read(cx).applications.clone();
        let open = page.read(cx).open_page().cloned().expect("catalog's page");
        assert_eq!(
            open.read(cx).id().as_str(),
            "argocd:applicationset/argocd/catalog"
        );
        assert!(open.read(cx).focus_handle().is_focused(window));
        assert_eq!(view.read(cx).page, Page::Applications);
    })
    .unwrap();
}

/// Before an answer, refused, failed and empty, a note says why under All
/// applications; a failed read offers Retry, which reads again.
#[gpui_kit::test]
fn the_column_says_why_it_lists_no_applications(cx: &mut TestAppContext) {
    for (variant, text, retry) in [
        (Variant::Refused, "Not permitted", false),
        (Variant::Failed, "Couldn't read", true),
        (Variant::Unserved, "No applications found", false),
    ] {
        let (_runtime, handle, view) = applications(cx, variant);
        let ids: Vec<String> = lines(&view, cx).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [ALL, "nav-applications-status"], "{variant:?}");
        cx.update_window(handle, |_, window, cx| {
            let note = window.find("nav-applications-status");
            assert!(
                note.label().is_some_and(|label| label.contains(text)),
                "{variant:?}: {:?}",
                note.label()
            );
            assert_eq!(
                window.try_find("nav-applications-status-retry").is_some(),
                retry,
                "{variant:?}"
            );
            let all = view.read(cx).column_list.lines()[0]
                .row()
                .unwrap()
                .counted()
                .cloned();
            assert_eq!(all, None, "{variant:?}: no count of nothing");
            if retry {
                let page = view.read(cx).applications.clone();
                page.update(cx, |page, _| page.set_hold(true));
                window.click("nav-applications-status-retry", cx);
                assert!(page.read(cx).is_reading(), "Retry reads again");
            }
        })
        .unwrap();
        if retry {
            // Until it answers the note says so, and Retry waits.
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window
                        .find("nav-applications-status")
                        .label()
                        .is_some_and(|label| label.contains("Reading…"))
                );
                assert!(window.try_find("nav-applications-status-retry").is_none());
            })
            .unwrap();
        }
    }
    // Held, the first read never answers.
    let (_runtime, _handle, view) = fixture(cx, 1280., 880.);
    let page = cx.update(|cx| view.read(cx).applications.clone());
    cx.update(|cx| page.update(cx, |page, _| page.set_hold(true)));
    let handle = _handle;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_area(Area::Applications, window, cx)
        });
        window.render_frame(cx);
        assert!(
            window
                .find("nav-applications-status")
                .label()
                .is_some_and(|label| label.contains("Reading…"))
        );
    })
    .unwrap();
}

/// Past thirty rows a menu names every application.
#[gpui_kit::test]
fn the_column_names_the_rest_in_a_menu(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let app = |ix: usize| ColumnApp {
        key: format!("part-of:app-{ix:02}").into(),
        name: format!("app-{ix:02}").into(),
        detail: "part-of label · core-fra".into(),
        menu: format!("app-{ix:02}").into(),
        incomplete: (ix == 0).then(|| SharedString::from("may be incomplete: why")),
    };
    let column = Column {
        sections: vec![ColumnSection {
            rule: Rule::PartOf,
            label: "part-of label",
            apps: (0..35).map(app).collect(),
        }],
        note: None,
        total: 35,
    };
    cx.update(|cx| {
        let page = view.read(cx).applications.clone();
        page.update(cx, |page, cx| page.set_column(column, cx));
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let list = view.read(cx).column_list.lines();
        let rows = list
            .iter()
            .filter(|line| {
                matches!(
                    line.row().map(|row| row.key()),
                    Some(ColumnKey::Application(_))
                )
            })
            .count();
        assert_eq!(rows, 30);
        let Some(Line::Menu(menu)) = list.last() else {
            panic!("the last line is the menu");
        };
        assert_eq!(menu.entries().len(), 35);
        assert_eq!(
            list[2].row().unwrap().marked(),
            Some(freshkube_ui::ui::Tone::Unknown),
            "an incomplete application is marked"
        );
        assert!(
            window
                .find("nav-applications-more")
                .label()
                .is_some_and(|l| l.contains("5 more applications…"))
        );
    })
    .unwrap();
}

/// The column draws again for another application shown, never for the
/// page's own redraws, as a selection or a hover asks, nor for the same read
/// again.
#[gpui_kit::test]
fn the_column_redraws_only_for_what_it_shows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let page = cx.update(|cx| view.read(cx).applications.clone());
    let notify = page.clone();
    let redrawn = chrome_redrawn(cx, handle, |_, cx| notify.update(cx, |_, cx| cx.notify()));
    assert!(redrawn.is_empty(), "the page's redraw: {redrawn:?}");
    let open = page.clone();
    let redrawn = chrome_redrawn(cx, handle, |window, cx| {
        open.update(cx, |page, cx| page.open_key("kargo:cart", window, cx))
    });
    assert!(redrawn.contains(&"chrome.column"), "{redrawn:?}");
    let refresh = page.clone();
    let redrawn = chrome_redrawn(cx, handle, |_, cx| {
        refresh.update(cx, |page, cx| page.refresh(cx))
    });
    assert!(redrawn.is_empty(), "the same read again: {redrawn:?}");
}

/// Opening from the column clears a filter that hides the application, so
/// the list comes back showing what it has selected.
#[gpui_kit::test]
fn opening_from_the_column_never_leaves_the_list_filtered_against_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let page = cx.update(|cx| view.read(cx).applications.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click("applications-tally-read", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).names().contains(&"cart".to_owned()));
        window.click(CART, cx);
        window.render_frame(cx);
        window.click(ALL, cx);
        window.render_frame(cx);
        assert!(page.read(cx).names().contains(&"cart".to_owned()));
        assert_eq!(
            page.read(cx).selected_key().map(|key| key.as_ref()),
            Some("kargo:cart")
        );
        assert!(window.find("applications-detail").visible());
    })
    .unwrap();
}

/// A read the open application isn't in closes its page, and the fill goes
/// back to All applications.
#[gpui_kit::test]
fn a_read_without_the_open_application_moves_the_fill_back(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let page = cx.update(|cx| view.read(cx).applications.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click(CART, cx);
        window.render_frame(cx);
        assert_eq!(window.find(CART).selected(), Some(true));
        page.update(cx, |page, cx| {
            page.set_variant(Variant::Refused);
            page.refresh(cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(page.read(cx).open_page().is_none());
        assert!(window.try_find(CART).is_none(), "Kargo wasn't read");
        assert_eq!(window.find(ALL).selected(), Some(true));
    })
    .unwrap();
}

/// Another cluster drops what was read: the column says it is reading
/// until that cluster answers, never the last cluster's applications.
#[gpui_kit::test]
fn another_cluster_shows_reading_until_it_answers(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = applications(cx, Variant::Acme);
    let page = cx.update(|cx| view.read(cx).applications.clone());
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, _| page.set_hold(true));
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("dev-fra".into(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let ids: Vec<String> = lines(&view, cx).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, [ALL, "nav-applications-status"]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("nav-applications-status")
                .label()
                .is_some_and(|label| label.contains("Reading…"))
        );
    })
    .unwrap();
}
