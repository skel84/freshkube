//! An application's Timeline tab in the real app, on example data (#844).
use crate::applications::testing::page::{ApplicationPage, PageTab, TimelineView};
use crate::applications::{ApplicationsPage, Variant};
use crate::desktop::{Pilot, layout_check, tests::fixture};
use freshkube_delivery::timeline::example::Shape;

use std::time::Duration;

use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

const CHECKOUT: &str = "application-kargo:checkout";
const TIMELINE: layout_check::TablePage = layout_check::TablePage {
    page: "application-page",
    title: "application-title",
    title_text: "checkout",
    table: "timeline-table-scroll",
    list: "timeline-list",
};

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn page(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationPage> {
    list(view, cx).read(cx).open_page().cloned().unwrap()
}

fn timeline(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<TimelineView> {
    page(view, cx).read(cx).timeline().clone()
}

/// checkout's page opened with the example sources in `shape`.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: Shape,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| {
            page.set_variant(Variant::Acme);
            page.set_timeline_shape(shape);
        });
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

fn show_timeline(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.click("application-tab-timeline", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn chips(view: &Entity<Pilot>, cx: &mut TestAppContext) -> Vec<String> {
    cx.read(|cx| timeline(view, cx).read(cx).chips_text())
}

fn lines(view: &Entity<Pilot>, cx: &mut TestAppContext) -> Vec<String> {
    cx.read(|cx| timeline(view, cx).read(cx).lines_text())
}

#[gpui_kit::test]
fn the_tab_reads_only_while_it_shows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Busy);
    // The parts show first: nothing of the timeline is read.
    let revision = cx.read(|cx| timeline(&view, cx).read(cx).read_revision());
    assert!(!cx.read(|cx| timeline(&view, cx).read(cx).answered()));
    assert!(lines(&view, cx).is_empty());

    show_timeline(cx, handle);
    let first = cx.read(|cx| timeline(&view, cx).read(cx).read_revision());
    assert!(first > revision, "showing the tab reads");
    assert!(cx.read(|cx| timeline(&view, cx).read(cx).answered()));

    // Back to the parts and again within 30 s: what it has is fresh.
    cx.update_window(handle, |_, window, cx| {
        window.click("application-tab-parts", cx);
        window.render_frame(cx);
        window.click("application-tab-timeline", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| timeline(&view, cx).read(cx).read_revision()),
        first
    );

    // Hidden past 30 s, showing reads again.
    cx.update_window(handle, |_, window, cx| {
        window.click("application-tab-parts", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(31));
    show_timeline(cx, handle);
    assert!(cx.read(|cx| timeline(&view, cx).read(cx).read_revision()) > first);
}

#[gpui_kit::test]
fn the_list_shows_loading_rows_until_the_first_answer(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The applications are read; the timeline's sources answer slowly.
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_example_delay(Duration::from_secs(5)));
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click("application-tab-timeline", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| timeline(&view, cx).read(cx).is_reading()));
    assert!(lines(&view, cx).is_empty());
    assert!(
        chips(&view, cx).is_empty(),
        "no chip before a source answered"
    );
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!cx.read(|cx| timeline(&view, cx).read(cx).is_reading()));
    assert_eq!(chips(&view, cx).len(), 7);
}

#[gpui_kit::test]
fn leaving_the_page_stops_the_read_and_returning_reads_once_old(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Busy);
    show_timeline(cx, handle);
    let first = cx.read(|cx| timeline(&view, cx).read(cx).read_revision());
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| page(&view, cx).read(cx).tab()),
        PageTab::Timeline
    );
    assert!(cx.read(|cx| timeline(&view, cx).read(cx).read_revision()) > first);
}

#[gpui_kit::test]
fn every_source_is_a_chip_and_events_say_where_the_cluster_stops_keeping_them(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Busy);
    show_timeline(cx, handle);
    assert_eq!(
        chips(&view, cx),
        [
            "Promotions 2",
            "Syncs 1",
            "Incidents 1",
            "Deploys 1",
            "Events 2 · kept 1 h",
            "Restarts 1",
            "Nodes 0",
        ]
    );
    let lines = lines(&view, cx);
    let kept = lines
        .iter()
        .position(|line| line.ends_with("Events Not kept by the cluster"))
        .expect("a row where events stop");
    // The Warning events are above it, newer; the deploy two hours back below.
    assert!(lines[..kept].iter().any(|line| line.contains("BackOff")));
    assert!(lines[kept..].iter().any(|line| line.contains("deploy/api")));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let label = window
            .find("timeline-chip-events")
            .label()
            .unwrap()
            .to_owned();
        assert!(
            label.contains("The cluster keeps it for about 1 h only"),
            "{label}"
        );
        assert!(window.find("timeline-chip-incidents").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_window_is_3_h_and_goes_up_to_24_h(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Busy);
    show_timeline(cx, handle);
    assert_eq!(cx.read(|cx| timeline(&view, cx).read(cx).window_hours()), 3);
    assert_eq!(chips(&view, cx)[6], "Nodes 0");
    cx.update_window(handle, |_, window, cx| {
        window.click("timeline-window-6", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The node went NotReady four hours ago.
    assert_eq!(chips(&view, cx)[6], "Nodes 1");
    assert!(
        lines(&view, cx)
            .iter()
            .any(|line| line.contains("worker-2 NotReady"))
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("timeline-window-24", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| timeline(&view, cx).read(cx).window_hours()),
        24
    );
}

#[gpui_kit::test]
fn a_source_that_could_not_be_read_says_why_beside_the_ones_that_were(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Partial);
    show_timeline(cx, handle);
    let chips = chips(&view, cx);
    assert!(
        chips.contains(&"Incidents not read: refused (403)".to_owned()),
        "{chips:?}"
    );
    assert!(
        chips.contains(&"Syncs not read: Argo CD isn't installed".to_owned()),
        "{chips:?}"
    );
    assert!(chips.contains(&"Deploys 1".to_owned()));
    let lines = lines(&view, cx);
    assert!(
        !lines.iter().any(|line| line.contains("Incidents")),
        "{lines:?}"
    );
    // Their hours are not read by them, and the quiet rows say so.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let label = window
            .find("timeline-chip-incidents")
            .label()
            .unwrap()
            .to_owned();
        assert_eq!(label, "Incidents: not read: refused (403)");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_quiet_window_names_every_source_it_read(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Quiet);
    show_timeline(cx, handle);
    let lines = lines(&view, cx);
    let details = cx.read(|cx| timeline(&view, cx).read(cx).detail_text());
    // Nothing happened, so every row is a note: quiet hours, and where the
    // cluster stops keeping Events.
    assert!(
        lines
            .iter()
            .all(|line| line.contains("Quiet") || line.contains("Not kept")),
        "{lines:?}"
    );
    let quiet: Vec<_> = lines
        .iter()
        .zip(&details)
        .filter(|(line, _)| line.contains("Quiet"))
        .map(|(_, detail)| detail.as_str())
        .collect();
    // The last hour was read by all seven; the two before it by six, Events
    // reaching back an hour only.
    assert_eq!(quiet.len(), 2, "{lines:?}");
    assert_eq!(
        quiet[0],
        "Read: Promotions, Syncs, Incidents, Deploys, Events, Restarts, Nodes"
    );
    assert_eq!(
        quiet[1],
        "Read: Promotions, Syncs, Incidents, Deploys, Restarts, Nodes · Not read: Events"
    );
}

#[gpui_kit::test]
fn nothing_read_is_never_shown_as_quiet(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::NoneRead);
    show_timeline(cx, handle);
    assert!(lines(&view, cx).is_empty());
    assert!(
        chips(&view, cx)
            .iter()
            .all(|chip| chip.contains("not read"))
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("timeline-empty").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_timeline_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880., Shape::Busy);
    show_timeline(cx, handle);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            layout_check::assert_table_page_from(window, cx, &TIMELINE, "application-back");
        })
        .unwrap();
    }
}
