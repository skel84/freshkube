//! Waiting to ship in the real app, on example data: acme's `checkout`,
//! which fans out to two production regions, `catalog`, two applications
//! that are caught up and two that weren't read, opened from the
//! Applications column.
use std::time::Duration;

use freshkube_delivery::waiting::{self as core_waiting, Waiting};

use crate::applications::testing::waiting::Body;
use crate::applications::{ApplicationsPage, WaitingPage};
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

const WAITING: layout_check::TablePage = layout_check::TablePage {
    page: "waiting-page",
    title: "waiting-title",
    title_text: "Waiting to ship",
    table: "waiting-table-scroll",
    list: "waiting-list",
};

const WONKY: &str = "waiting-row-checkout/wonky-otter";
const HERON: &str = "waiting-row-checkout/quiet-heron";

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn waiting(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<WaitingPage> {
    list(view, cx).read(cx).waiting_page().clone()
}

/// The app with `prepare` done to the page before the column's Waiting to
/// ship row opens it.
fn open_with(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    prepare: impl FnOnce(&mut WaitingPage),
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        waiting(&view, cx).update(cx, |page, _| prepare(page));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        // A narrow window shows no column: the row's own action.
        if window.try_find("nav-waiting").is_some() {
            window.click("nav-waiting", cx);
        } else {
            list(&view, cx).update(cx, |page, cx| page.show_waiting(window, cx));
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

/// Types into the find box and lets the page derive its lines.
fn find(cx: &mut TestAppContext, handle: AnyWindowHandle, text: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.click("waiting-find", cx);
        window.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn waiting_to_ship_is_the_second_row_under_changes(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert!(list(&view, cx).read(cx).waiting_shown());
        assert!(!list(&view, cx).read(cx).changes_shown());
        assert!(window.find("waiting-page").visible());
        assert!(window.try_find("applications-page").is_none());
        assert!(window.try_find("changes-page").is_none());
        let changes = window.find("nav-changes");
        let waiting_row = window.find("nav-waiting");
        let all = window.find("nav-all-applications");
        assert!(changes.bounds().top() < waiting_row.bounds().top());
        assert!(waiting_row.bounds().top() < all.bounds().top());
        assert_eq!(waiting_row.selected(), Some(true));
        assert_ne!(changes.selected(), Some(true));
        // Changes takes the place, and Waiting to ship stops reading.
        window.click("nav-changes", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).read(cx).changes_shown());
        assert!(!list(&view, cx).read(cx).waiting_shown());
        assert!(!waiting(&view, cx).read(cx).is_reading());
        window.click("nav-waiting", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).read(cx).waiting_shown());
        assert!(!list(&view, cx).read(cx).changes_shown());
        // All applications comes back to the list.
        window.click("nav-all-applications", cx);
        window.render_frame(cx);
        assert!(!list(&view, cx).read(cx).waiting_shown());
        assert!(window.find("applications-page").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn each_application_is_a_group_oldest_wait_first_with_the_rest_after(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = waiting(&view, cx);
        assert_eq!(
            page.read(cx).lines_text(),
            [
                "# checkout",
                "checkout/quiet-heron",
                "checkout/merry-owl",
                "checkout/wonky-otter",
                "checkout/tidy-badger",
                "# catalog",
                "catalog/plucky-newt",
                "# Not read",
                "billing",
                "payments",
                "# Caught up",
            ],
            "the caught-up group starts folded while anything waits"
        );
        assert!(window.find("waiting-showing").visible());
        window.click("waiting-show-all", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).lines_text()[10..],
            ["# Caught up", "cart", "search"]
        );
        assert!(window.try_find("waiting-showing").is_none());
        window.click("waiting-fold-caught-up", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).lines_text().len(), 11);
    })
    .unwrap();
}

#[gpui_kit::test]
fn apps_that_were_not_read_are_rows_with_their_cause(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let why = window.find("waiting-row-payments-why");
        let label = why.label().unwrap_or_default().to_owned();
        assert!(label.starts_with("Can't read "), "{label}");
        assert!(label.contains("forbidden"), "{label}");
        assert!(window.find("waiting-row-billing-why").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_page_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            let layout = layout_check::assert_table_page(window, cx, &WAITING);
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
                waiting(&view, cx).update(cx, |page, cx| {
                    page.select("checkout/quiet-heron", window, cx)
                });
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "waiting-split",
                    "waiting-table",
                    "waiting-detail",
                    "waiting-detail-title",
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn a_selection_says_which_stages_are_production_and_what_promoting_would_ship(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = open(cx, 1600., 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(HERON, cx);
        window.render_frame(cx);
        assert!(window.find("waiting-detail").visible());
        let page = waiting(&view, cx);
        let page = page.read(cx);
        let row = page
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == "checkout/quiet-heron")
            .unwrap();
        assert_eq!(
            row.detail.production[0].as_ref(),
            "Production is prod-ams and prod-lon: the Stages nothing takes Freight from."
        );
        assert!(
            row.detail
                .production
                .iter()
                .any(|line| line.starts_with("The application shows prod-ams")),
            "{:?}",
            row.detail.production
        );
        assert_eq!(
            row.detail.ship[0].as_ref(),
            "prod-ams: quiet-heron brings 1 Freight"
        );
        // Merged is its own column, and the wait runs from the Stage before
        // production.
        assert_ne!(row.merged, row.waited);
        for part in ["trace", "production", "ship", "basis"] {
            // The Inspector scrolls, so the last sections may be below the fold.
            assert!(
                window.try_find(format!("waiting-detail-{part}")).is_some(),
                "{part}"
            );
        }
        assert_eq!(
            window.find("waiting-detail-stage-3").label(),
            Some("prod-lon (production): Runs it now")
        );
        window.click("waiting-row-checkout/tidy-badger", cx);
        window.render_frame(cx);
        assert!(window.find("waiting-detail-why").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_clears_the_selection_and_closes_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1600., 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(WONKY, cx);
        window.render_frame(cx);
        assert!(window.find("waiting-detail").visible());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("waiting-detail").is_none());
        assert_eq!(waiting(&view, cx).read(cx).selected(), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_find_box_narrows_the_list_by_pull_request(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    find(cx, handle, "#418");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            waiting(&view, cx).read(cx).lines_text(),
            ["# checkout", "checkout/wonky-otter"]
        );
        assert!(window.find(WONKY).visible());
        assert!(window.try_find(HERON).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_find_box_narrows_the_list_by_commit_and_traces_one_that_shipped(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1600., 900.);
    find(cx, handle, "e93b6f4");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            waiting(&view, cx).read(cx).lines_text(),
            ["# checkout", "checkout/quiet-heron"]
        );
        window.click(HERON, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("waiting-detail-stage-3").label(),
            Some("prod-lon (production): Runs it now")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_commit_already_in_production_is_found_among_the_listed_freight(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1600., 900.);
    find(cx, handle, "5d20b1a");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            waiting(&view, cx).read(cx).lines_text(),
            ["# checkout", "checkout/calm-wren"]
        );
        window.click("waiting-row-checkout/calm-wren", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("waiting-detail-state").label(),
            Some("In production")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_search_that_finds_nothing_says_why(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    find(cx, handle, "9999");
    cx.update_window(handle, |_, window, cx| {
        let page = waiting(&view, cx);
        assert!(matches!(
            page.read(cx).body(),
            Body::Empty { title, .. } if title.contains("pull request 9999")
        ));
        assert!(window.find("waiting-none").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_search_that_asks_nothing_says_how_to_ask(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    find(cx, handle, "retry keys");
    cx.update_window(handle, |_, _, cx| {
        let page = waiting(&view, cx);
        assert!(matches!(
            page.read(cx).body(),
            Body::Empty { title, .. } if title.contains("isn't a pull request")
        ));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_page_shows_loading_rows_until_it_answers(cx: &mut TestAppContext) {
    let delay = Duration::from_secs(2);
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_example_delay(delay));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(waiting(&view, cx).read(cx).is_reading());
        assert!(
            window
                .within("waiting-list")
                .find("waiting-loading")
                .visible()
        );
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_some());
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!waiting(&view, cx).read(cx).is_reading());
        assert!(window.try_find("waiting-loading").is_none());
        assert!(window.find(WONKY).visible());
    })
    .unwrap();
}

/// Hidden, the page drops its read; shown again within 30 s it reads
/// nothing, and after them it reads again.
#[gpui_kit::test]
fn the_page_reads_only_while_it_shows(cx: &mut TestAppContext) {
    let delay = Duration::from_secs(2);
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_example_delay(delay));
    cx.update_window(handle, |_, window, cx| {
        assert!(waiting(&view, cx).read(cx).is_reading());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        assert!(!waiting(&view, cx).read(cx).is_reading(), "hidden: no read");
    })
    .unwrap();
    cx.executor().advance_clock(delay * 2);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(waiting(&view, cx).read(cx).lines_text().is_empty());
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).read(cx).waiting_shown());
        assert!(waiting(&view, cx).read(cx).is_reading());
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!waiting(&view, cx).read(cx).lines_text().is_empty());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(!waiting(&view, cx).read(cx).is_reading(), "still fresh");
        // Another page of the area hides it too.
        window.click("nav-changes", cx);
        window.render_frame(cx);
        assert!(!waiting(&view, cx).read(cx).is_reading());
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click("nav-waiting", cx);
        window.render_frame(cx);
        assert!(waiting(&view, cx).read(cx).is_reading());
    })
    .unwrap();
}

fn answered(
    cx: &mut TestAppContext,
    answer: Waiting,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_hold(true));
    cx.update_window(handle, |_, window, cx| {
        waiting(&view, cx).update(cx, |page, cx| page.read_answer(answer, cx));
        window.render_frame(cx);
    })
    .unwrap();
    (runtime, handle, view)
}

fn unreached(why: &str) -> Waiting {
    let mut read = core_waiting::example::read(chrono::Utc::now());
    read.unreached = Some(why.to_owned());
    core_waiting::derive(&read)
}

#[gpui_kit::test]
fn a_kargo_that_was_not_reached_fails_in_its_words_never_none(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = answered(cx, unreached("no answer within 5 s"));
    cx.update_window(handle, |_, window, cx| {
        assert!(matches!(
            waiting(&view, cx).read(cx).body(),
            Body::Failed { reason } if reason.contains("no answer within 5 s")
        ));
        assert!(window.find("waiting-failed").visible());
        assert!(window.find("waiting-retry").visible());
        assert!(window.try_find("waiting-none").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_read_offers_retry_and_a_failed_refresh_keeps_the_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_with(cx, 1280., 880., |page| page.set_hold(true));
    cx.update_window(handle, |_, window, cx| {
        waiting(&view, cx).update(cx, |page, cx| page.fail_read("connection reset", cx));
        window.render_frame(cx);
        assert!(window.find("waiting-failed").visible());
        waiting(&view, cx).update(cx, |page, cx| {
            page.read_answer(
                core_waiting::derive(&core_waiting::example::read(chrono::Utc::now())),
                cx,
            );
            page.fail_read("connection reset", cx);
        });
        window.render_frame(cx);
        assert!(window.find("waiting-stale").visible());
        assert!(window.find(WONKY).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn what_may_be_missing_is_a_banner_and_a_row_that_says_why_is_not_repeated_in_it(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The cap on `catalog` is in the banner; `payments` and `billing`
        // say why in their own rows, not twice.
        let banner = window.find("waiting-missing");
        let label = banner.label().unwrap_or_default().to_owned();
        assert!(label.starts_with("May be missing: "), "{label}");
        assert!(
            label.contains("catalog lists its 50 newest Freight; 4 older"),
            "{label}"
        );
        assert!(
            !label.contains("payments") && !label.contains("billing"),
            "{label}"
        );
        assert!(window.find(WONKY).visible());
    })
    .unwrap();
}

fn derived() -> Waiting {
    core_waiting::derive(&core_waiting::example::read(chrono::Utc::now()))
}

#[gpui_kit::test]
fn a_pull_request_that_was_not_read_says_so_never_the_bare_commit(cx: &mut TestAppContext) {
    use core_waiting::PullRead;
    let mut waiting_read = core_waiting::example::read(chrono::Utc::now());
    if let freshkube_delivery::source::Source::Read(projects) = &mut waiting_read.projects {
        for project in projects.iter_mut() {
            let unread = |rate_limit| PullRead::Unread {
                rate_limit,
                why: "Pull requests weren't read: GitHub's limit is low: 8 left until 14:05 UTC"
                    .into(),
            };
            if project.name == "checkout" {
                project.pulls.insert("wonky-otter".into(), unread(true));
                project.pulls.insert("tidy-badger".into(), unread(false));
            }
        }
    }
    let (_runtime, handle, view) = answered(cx, core_waiting::derive(&waiting_read));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = waiting(&view, cx);
        let page = page.read(cx);
        let named = |key: &str| {
            page.display
                .rows
                .iter()
                .find(|row| row.key.as_ref() == key)
                .map(|row| (row.named.to_string(), row.by_commit))
                .unwrap()
        };
        assert_eq!(
            named("checkout/wonky-otter"),
            ("not read (rate limit)".into(), false)
        );
        assert_eq!(named("checkout/tidy-badger"), ("not read".into(), false));
        // A pull request GitHub answered it names none for shows the commit.
        assert_eq!(named("catalog/plucky-newt"), ("0c4e8b1".into(), true));
        // The cause is the Inspector's.
        let row = page
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == "checkout/wonky-otter")
            .unwrap();
        // The heading names the change, and the field says it once.
        assert_eq!(
            row.detail.heading.as_ref(),
            "a1f3c9e Retry idempotency keys"
        );
        assert!(
            row.detail.fields.iter().any(|(label, value, _)| {
                *label == "Pull request"
                    && value.starts_with("Not read (rate limit): GitHub's limit is low")
            }),
            "{:?}",
            row.detail.fields
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_capped_project_says_how_many_older_freight_are_not_listed_where_it_shows(
    cx: &mut TestAppContext,
) {
    let mut waiting_read = derived();
    for app in &mut waiting_read.apps {
        if app.name == "checkout" {
            app.older = 10;
        }
    }
    let (_runtime, handle, view) = answered(cx, waiting_read);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = waiting(&view, cx);
        let page = page.read(cx);
        let group = page
            .display
            .groups
            .iter()
            .find(|group| group.title.as_ref() == "checkout")
            .unwrap();
        assert!(
            group
                .detail
                .iter()
                .any(|line| line.starts_with("10 older Freight not listed")),
            "{:?}",
            group.detail
        );
        let catalog = page
            .display
            .groups
            .iter()
            .find(|group| group.title.as_ref() == "catalog")
            .unwrap();
        // `catalog` is capped in the example itself: its own count.
        assert!(
            catalog
                .detail
                .iter()
                .any(|line| line.starts_with("4 older Freight not listed")),
            "{:?}",
            catalog.detail
        );
        let row = page
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == "checkout/quiet-heron")
            .unwrap();
        assert!(row.detail.basis.contains("10 older Freight not listed"));
        assert!(
            row.detail
                .production
                .iter()
                .any(|line| line.starts_with("10 older Freight not listed")),
            "{:?}",
            row.detail.production
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_github_not_set_up_a_row_keeps_its_commit_and_the_inspector_says_why(
    cx: &mut TestAppContext,
) {
    let mut waiting_read = core_waiting::example::read(chrono::Utc::now());
    waiting_read.pulls_read = false;
    if let freshkube_delivery::source::Source::Read(projects) = &mut waiting_read.projects {
        projects
            .iter_mut()
            .for_each(|project| project.pulls.clear());
    }
    let (_runtime, handle, view) = answered(cx, core_waiting::derive(&waiting_read));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = waiting(&view, cx);
        let page = page.read(cx);
        let row = page
            .display
            .rows
            .iter()
            .find(|row| row.key.as_ref() == "checkout/wonky-otter")
            .unwrap();
        assert_eq!((row.named.as_ref(), row.by_commit), ("a1f3c9e", true));
        assert!(
            row.detail.fields.iter().any(|(label, value, _)| {
                *label == "Pull request" && value.contains("GitHub isn't set up")
            }),
            "{:?}",
            row.detail.fields
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_table_keeps_beside_the_inspector_at_1280_without_scrolling_sideways(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        waiting(&view, cx).update(cx, |page, cx| {
            page.select("checkout/quiet-heron", window, cx)
        });
        window.render_frame(cx);
        let least = waiting(&view, cx).read(cx).table_width();
        let table = f32::from(window.find("waiting-table").bounds().size.width);
        assert!(
            table >= least,
            "the table is {table} wide, its columns {least}"
        );
    })
    .unwrap();
}
