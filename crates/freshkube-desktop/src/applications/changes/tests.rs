//! The Changes list in the real app, on example data: acme's `checkout`
//! and `cart`, opened from the Applications column.
use std::time::Duration;

use freshkube_delivery::changes::{Changes, Gap, Note};

use super::super::ApplicationsPage;
use super::ChangesPage;
use super::display::Body;
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, test::TestWindowExt,
};

const CHANGES: layout_check::TablePage = layout_check::TablePage {
    page: "changes-page",
    title: "changes-title",
    title_text: "Changes",
    table: "changes-table-scroll",
    list: "changes-list",
};

const WONKY: &str = "changes-row-checkout/wonky-otter";
const LYNX: &str = "changes-row-checkout/brave-lynx";

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn changes(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ChangesPage> {
    list(view, cx).read(cx).changes_page().clone()
}

/// The app with `prepare` done to the Changes list before the column's
/// Changes row opens it.
fn open_with(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    prepare: impl FnOnce(&mut ChangesPage),
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, _| prepare(page));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        // A narrow window shows no column: the row's own action.
        if window.try_find("nav-changes").is_some() {
            window.click("nav-changes", cx);
        } else {
            list(&view, cx).update(cx, |page, cx| page.show_changes(window, cx));
        }
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    open_with(cx, width, height, |_| {})
}

#[gpui_kit::test]
fn changes_leads_the_column_and_opens_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert!(list(&view, cx).read(cx).changes_shown());
        assert!(window.find("changes-page").visible());
        assert!(window.try_find("applications-page").is_none());
        let changes_row = window.find("nav-changes");
        let all = window.find("nav-all-applications");
        assert!(changes_row.bounds().top() < all.bounds().top());
        assert_eq!(changes_row.selected(), Some(true));
        assert_ne!(all.selected(), Some(true));
        // All applications comes back to the list, which stops reading
        // the changes.
        window.click("nav-all-applications", cx);
        window.render_frame(cx);
        assert!(!list(&view, cx).read(cx).changes_shown());
        assert!(window.find("applications-page").visible());
        assert!(!changes(&view, cx).read(cx).is_reading());
    })
    .unwrap();
}

#[gpui_kit::test]
fn rows_need_a_look_first_with_the_rest_folded(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = changes(&view, cx);
        assert_eq!(
            page.read(cx).lines_text(),
            [
                "# Not fully confirmed",
                "lucky-finch",
                "brave-lynx",
                "# Confirmed to the running pods",
                "# Not running in any Stage",
            ]
        );
        assert!(window.find("changes-showing").visible());
        // One Stages cell: the row's own Stages in promotion order.
        assert_eq!(
            window
                .find(SharedString::from(format!("{LYNX}-stages")))
                .label(),
            Some("dev – · stage – · prod-ams Refused · prod-lon –")
        );
        window.click("changes-fold-confirmed", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).lines_text()[3..6],
            [
                "# Confirmed to the running pods",
                "wonky-otter",
                "sleepy-mole"
            ]
        );
        window.click("changes-show-all", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).lines_text().len(), 9);
        assert!(window.try_find("changes-showing").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_changes_list_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            let layout = layout_check::assert_table_page(window, cx, &CHANGES);
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_inspector_sits_beside_a_wide_table_and_under_a_narrow_one(cx: &mut TestAppContext) {
    for (width, height) in [(1600., 900.), (760., 560.)] {
        for size in [None, Some(20.)] {
            let (_runtime, handle, view) = open(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                if let Some(size) = size {
                    crate::text_size::set(size, cx);
                }
                window.render_frame(cx);
                // A narrow table scrolls sideways, wider than the window a
                // click lands in: select the row as the keyboard does.
                changes(&view, cx).update(cx, |page, cx| {
                    page.select("checkout/brave-lynx", window, cx)
                });
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "changes-split",
                    "changes-table",
                    "changes-detail",
                    "changes-detail-title",
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn a_selection_shows_its_trail_and_enter_opens_the_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // A folded row selected by the keyboard unfolds its group.
        changes(&view, cx).update(cx, |page, cx| {
            page.select("checkout/wonky-otter", window, cx)
        });
        window.render_frame(cx);
        assert!(window.find(WONKY).visible());
        assert_eq!(
            window.find("changes-detail-title").label(),
            Some("#418 Retry idempotency keys")
        );
        assert!(window.find("changes-detail-trail").visible());
        assert!(window.find("changes-detail-stage-0").visible());
        assert!(window.try_find("changes-detail-stage-2").is_some());
        // Beside the Inspector at 1280, with the column open, every column
        // fits, with no sideways scroll.
        let detail = window.find("changes-detail").bounds();
        let age = window
            .find(SharedString::from(format!("{WONKY}-age")))
            .bounds();
        assert!(age.right() <= detail.left(), "{age:?} beside {detail:?}");
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = changes(&view, cx);
        let change = page
            .read(cx)
            .change_page()
            .cloned()
            .expect("the change shows");
        assert_eq!(change.read(cx).freight(), "wonky-otter");
        assert!(window.find("change-page").visible());
        window.click("change-back", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = changes(&view, cx);
        assert!(page.read(cx).change_page().is_none());
        assert!(window.find("changes-page").visible());
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some("checkout/wonky-otter".into())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_freight_no_stage_runs_opens_with_every_stage_not_reached(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            page.select("checkout/tidy-badger", window, cx)
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("changes-detail-no-trail").label(),
            Some("No Stage runs it, so it wasn't followed")
        );
        window.click("changes-detail-open", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let change = changes(&view, cx).read(cx).change_page().cloned().unwrap();
        assert_eq!(change.read(cx).freight(), "tidy-badger");
        assert!(window.try_find("change-failed").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_list_shows_loading_rows_until_it_answers(cx: &mut TestAppContext) {
    let delay = Duration::from_secs(2);
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_example_delay(delay));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(changes(&view, cx).read(cx).is_reading());
        assert!(
            window
                .within("changes-list")
                .find("changes-loading")
                .visible()
        );
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_some());
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!changes(&view, cx).read(cx).is_reading());
        assert!(window.try_find("changes-loading").is_none());
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_none());
        assert!(window.find(LYNX).visible());
    })
    .unwrap();
}

/// Hidden, the list drops its read; shown again within 30 s it reads
/// nothing, and after them it reads again.
#[gpui_kit::test]
fn the_list_reads_only_while_it_shows(cx: &mut TestAppContext) {
    let delay = Duration::from_secs(2);
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_example_delay(delay));
    cx.update_window(handle, |_, window, cx| {
        assert!(changes(&view, cx).read(cx).is_reading());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        assert!(!changes(&view, cx).read(cx).is_reading());
    })
    .unwrap();
    cx.executor().advance_clock(delay * 2);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(changes(&view, cx).read(cx).body(), &Body::Table);
        assert!(changes(&view, cx).read(cx).lines_text().is_empty());
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).read(cx).changes_shown());
        assert!(changes(&view, cx).read(cx).is_reading());
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!changes(&view, cx).read(cx).lines_text().is_empty());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(!changes(&view, cx).read(cx).is_reading(), "still fresh");
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(changes(&view, cx).read(cx).is_reading());
    })
    .unwrap();
}

fn answered(
    cx: &mut TestAppContext,
    answer: Changes,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_hold(true));
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| page.read_answer(answer, cx));
        window.render_frame(cx);
    })
    .unwrap();
    (runtime, handle, view)
}

#[gpui_kit::test]
fn no_freight_is_empty_only_when_kargo_answered(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = answered(cx, Changes::default());
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("changes-none").visible());
        assert!(matches!(
            changes(&view, cx).read(cx).body(),
            Body::Empty { title, .. } if title.as_ref() == "No changes found"
        ));
    })
    .unwrap();
}

#[gpui_kit::test]
fn refused_kargo_says_so_never_none(cx: &mut TestAppContext) {
    let refused = Changes {
        notes: vec![Note {
            gap: Gap::Refused,
            project: None,
            text: "Kargo projects were refused on core-fra: projects is forbidden".into(),
        }],
        ..Changes::default()
    };
    let (_runtime, handle, _view) = answered(cx, refused);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("changes-refused").visible());
        assert!(window.try_find("changes-none").is_none());
        assert!(window.try_find("changes-retry").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_read_offers_retry(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_hold(true));
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("changes-loading").visible());
        changes(&view, cx).update(cx, |page, cx| page.fail_read("connection refused", cx));
        window.render_frame(cx);
        assert!(window.find("changes-failed").visible());
        window.click("changes-retry", cx);
        window.render_frame(cx);
        assert!(changes(&view, cx).read(cx).is_reading());
    })
    .unwrap();
}

/// Rows beside a project that wasn't read keep the table, with a banner.
#[gpui_kit::test]
fn a_refused_project_beside_rows_is_a_banner(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let mut read = freshkube_delivery::changes::derive(
            &freshkube_delivery::changes::example::read(chrono::Utc::now()),
        );
        read.notes.push(Note {
            gap: Gap::Refused,
            project: Some("loyalty".into()),
            text: "loyalty's Freight were refused on core-fra: freights is forbidden".into(),
        });
        changes(&view, cx).update(cx, |page, cx| page.read_answer(read, cx));
        window.render_frame(cx);
        let banner = window.find("changes-missing");
        assert!(banner.visible());
        assert!(window.find(LYNX).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspector_says_why_each_stage_is_what_it_is(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            page.select("checkout/brave-lynx", window, cx)
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("changes-detail-stage-2").label(),
            Some(
                "prod-ams: Refused. Reading Pods there was refused, so what runs can't be \
                 confirmed. A refused list isn't an empty one."
            )
        );
        assert_eq!(
            window.find("changes-detail-stage-3").label(),
            Some("prod-lon: –. prod-lon runs wonky-otter")
        );
    })
    .unwrap();
}

/// A source that never connects: a kubeconfig context with no files.
fn live_source(context: &str) -> crate::resources::ClusterSource {
    use crate::resources::direct::DirectAccess;
    let access = DirectAccess::new(
        Vec::new(),
        context.into(),
        freshkube_core::ConfigurationRevision::default(),
    );
    crate::resources::ClusterSource {
        id: access.id(),
        context: context.into(),
        access: crate::resources::ClusterAccess::Direct(access),
    }
}

/// The page on a live source with example rows answered, reading nothing.
fn answered_live(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            page.set_source(Some(live_source("homelab")), cx);
            assert!(page.is_reading(), "a shown page reads its new source");
            let read = freshkube_delivery::changes::derive(
                &freshkube_delivery::changes::example::read(chrono::Utc::now()),
            );
            page.read_answer(read, cx);
            assert!(!page.is_reading());
        });
        window.render_frame(cx);
    })
    .unwrap();
    (runtime, handle, view)
}

#[gpui_kit::test]
fn another_connection_drops_the_read_in_flight_and_its_rows(cx: &mut TestAppContext) {
    let delay = Duration::from_secs(2);
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_example_delay(delay));
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            assert!(page.is_reading());
            // The next read stays in flight, so only the first could land.
            page.set_source(
                Some(crate::resources::ClusterSource {
                    id: "example:other".into(),
                    context: "other".into(),
                    access: crate::resources::ClusterAccess::Example(crate::fixture::Hold {
                        applications: true,
                        ..crate::fixture::Hold::NONE
                    }),
                }),
                cx,
            );
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor().advance_clock(delay * 2);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = changes(&view, cx);
        assert!(
            page.read(cx).lines_text().is_empty(),
            "the late answer didn't land"
        );
        assert!(
            page.read(cx).is_reading(),
            "the new connection's read is in flight"
        );
        assert!(window.try_find(LYNX).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_rows_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| page.fail_read("connection refused", cx));
        window.render_frame(cx);
        let stale = window.find("changes-stale");
        assert!(stale.visible());
        assert!(
            stale
                .label()
                .is_some_and(|label| label.contains("connection refused"))
        );
        assert!(window.find(LYNX).visible());
        assert!(window.try_find("changes-failed").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_clears_the_selection_and_closes_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            page.select("checkout/brave-lynx", window, cx);
            page.focus(window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("changes-detail").visible());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("changes-detail").is_none());
        assert_eq!(changes(&view, cx).read(cx).selected(), None);
        assert!(window.find(LYNX).visible(), "the list stays");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_remap_reads_a_live_list_again(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = answered_live(cx);
    cx.update_window(handle, |_, _, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            let connections = super::super::links::Connections::default();
            let remapped = super::super::across::Across {
                mapping: freshkube_delivery::change::live::Mapping {
                    entries: vec!["homelab".into()],
                    ..Default::default()
                },
                ..Default::default()
            };
            page.set_clusters(connections.clone(), remapped.clone(), cx);
            assert!(page.is_reading(), "its Stages resolve by the new mapping");
            let read = freshkube_delivery::changes::derive(
                &freshkube_delivery::changes::example::read(chrono::Utc::now()),
            );
            page.read_answer(read, cx);
            page.set_clusters(connections, remapped, cx);
            assert!(!page.is_reading(), "the same mapping reads nothing");
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_argo_cd_namespace_reads_a_live_list_again(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = answered_live(cx);
    cx.update_window(handle, |_, _, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            page.set_argocd_namespace("argocd".into(), cx);
            assert!(!page.is_reading(), "the namespace it read in");
            // Shown, it reads at once.
            page.set_argocd_namespace("deploy".into(), cx);
            assert!(page.is_reading(), "what it has was read in argocd");
            page.read_answer(Changes::default(), cx);
            // Hidden, it reads when next shown.
            page.set_visible(false, cx);
            page.set_argocd_namespace("gitops".into(), cx);
            assert!(!page.is_reading());
            page.set_visible(true, cx);
            assert!(page.is_reading(), "what it has was read in argocd");
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_connection_forgets_where_argo_cd_was_found(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = answered_live(cx);
    cx.update_window(handle, |_, _, cx| {
        changes(&view, cx).update(cx, |page, cx| {
            page.set_argocd_namespace("gitops".into(), cx);
            page.read_answer(Changes::default(), cx);
            page.set_source(Some(live_source("other")), cx);
            assert_eq!(page.argocd_namespace, "argocd", "until its read finds it");
            assert_eq!(page.read_namespace.as_deref(), Some("argocd"));
        });
    })
    .unwrap();
}

/// No rows beside a project that wasn't read: the banner says what, and
/// the empty state says none only of what was read.
#[gpui_kit::test]
fn no_rows_beside_an_unread_project_never_says_none(cx: &mut TestAppContext) {
    let unread = Changes {
        notes: vec![Note {
            gap: Gap::NotServed,
            project: Some("loyalty".into()),
            text: "loyalty's Freight aren't served on core-fra".into(),
        }],
        ..Changes::default()
    };
    let (_runtime, handle, view) = answered(cx, unread);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("changes-none").visible());
        assert!(window.find("changes-missing").visible());
        assert!(matches!(
            changes(&view, cx).read(cx).body(),
            Body::Empty { title, .. } if title.as_ref() == "No changes found in what was read"
        ));
    })
    .unwrap();
}

/// A Stage the cell has no room for is counted, never dropped silently.
#[gpui_kit::test]
fn stages_the_cell_drops_end_it_with_a_count(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = changes(&view, cx);
        let row = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == "checkout/brave-lynx")
            .cloned()
            .unwrap();
        assert_eq!(row.stages.len(), 4);
        // dev, stage and prod-ams fit beside the count; prod-lon is the +1.
        assert_eq!(row.stages_shown, 3);
        assert_eq!(
            row.stages_more.as_ref().map(|more| more.as_ref()),
            Some("+1")
        );
        let cell = window
            .find(SharedString::from(format!("{LYNX}-stages")))
            .bounds();
        let more = window
            .find(SharedString::from(format!("{LYNX}-stages-more")))
            .bounds();
        assert!(more.right() <= cell.right(), "{more:?} in {cell:?}");
        assert_eq!(
            window
                .find(SharedString::from(format!("{LYNX}-stages")))
                .label(),
            Some("dev – · stage – · prod-ams Refused · prod-lon –"),
            "the tooltip and label keep every Stage"
        );
    })
    .unwrap();
}

/// The title keeps the Change cell beside the Inspector: its application
/// follows it, dimmer, and gives way first (#568).
#[gpui_kit::test]
fn the_title_keeps_its_cell_and_its_application_gives_way(cx: &mut TestAppContext) {
    const TITLE: &str = "changes-row-cart/lucky-finch-title";
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            freshkube_ui::table::word_cut_shown(TITLE).as_deref(),
            Some("Keep the basket when signing in · cart")
        );
        changes(&view, cx).update(cx, |page, cx| page.select("cart/lucky-finch", window, cx));
        window.render_frame(cx);
        assert!(window.find("changes-detail").visible());
        let shown = freshkube_ui::table::word_cut_shown(TITLE).expect("drawn");
        assert!(
            shown.starts_with("Keep the basket") && shown.ends_with('…'),
            "the title keeps the cell: {shown}"
        );
        assert!(
            !shown.contains("cart"),
            "the application goes first: {shown}"
        );
    })
    .unwrap();
}

/// The list in the example reshaped as `variant`.
fn open_shaped(
    cx: &mut TestAppContext,
    variant: super::super::Variant,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(variant));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click("nav-changes", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

#[gpui_kit::test]
fn an_unreachable_kargo_entry_fails_in_its_name_never_none(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open_shaped(cx, super::super::Variant::Unreachable);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("changes-failed").visible());
        assert!(window.try_find("changes-none").is_none());
        assert!(window.try_find(WONKY).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn argo_cd_refused_keeps_the_rows_with_a_banner(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open_shaped(cx, super::super::Variant::ArgoRefused);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let missing = window.find("changes-missing").label().unwrap().to_owned();
        assert!(missing.contains("refused on core-fra"), "{missing}");
        assert!(window.find(LYNX).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_unreachable_destination_keeps_the_rows_with_a_banner(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_shaped(cx, super::super::Variant::Destination);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let missing = window.find("changes-missing").label().unwrap().to_owned();
        assert!(
            missing.contains(
                "Couldn't reach dev-fra: connection refused by dev-fra.example.test:6443"
            ),
            "{missing}"
        );
        assert!(window.find(LYNX).visible());
        // dev's cell is Unknown, as its group row on the change page is.
        let page = changes(&view, cx);
        let row = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == "checkout/wonky-otter")
            .cloned()
            .expect("wonky-otter's row");
        let dev = row
            .stages
            .iter()
            .find(|cell| cell.stage.as_ref() == "dev")
            .expect("dev's cell");
        assert_eq!(dev.tone, Some(freshkube_ui::ui::Tone::Unknown));
        assert_eq!(dev.word.as_ref(), "Unknown");
    })
    .unwrap();
}
