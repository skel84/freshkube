use super::*;
use crate::change::overview::cells::{Column, locate};

fn located(page: &Entity<ChangePage>, cx: &App) -> Option<(usize, Option<Column>)> {
    let page = page.read(cx);
    locate(&page.overview, page.selected()?)
}

#[gpui_kit::test]
fn the_toggle_preserves_the_selection_and_a_link_selection(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A cell is the hop it stands for.
        window.click("change-cell-dev-promoted", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("dev-promotion")
        );
        assert_eq!(located(&page, cx), Some((0, Some(Column::Promoted))));
        window.click("change-view-hops", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("dev-promotion")
        );
        assert!(window.find("change-hop-dev-promotion").visible());
        window.click("change-view-overview", cx);
        window.render_frame(cx);
        assert_eq!(located(&page, cx), Some((0, Some(Column::Promoted))));
        // A link selection carries over too.
        window.click("change-join-build-key", cx);
        assert!(page.read(cx).link_selected);
        window.click("change-view-hops", cx);
        window.render_frame(cx);
        assert!(page.read(cx).link_selected);
        assert!(window.find("change-hop-push-run").visible());
        page.update(cx, |page, cx| page.back(cx));
        page.update(cx, |page, cx| {
            page.set_view(ChangeView::Overview, window, cx)
        });
        hops(&page, window, cx);
        assert!(
            page.read(cx).selected.is_none(),
            "switching never invents a selection"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_running_cell_is_the_pods_row_and_the_synced_cell_the_application(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The cells beyond the Inspector's edge scroll into view when they
        // are selected; the ones in view are pressed.
        for (cell, hop) in [
            ("change-cell-stage-running", "stage-pods"),
            ("change-cell-stage-synced", "stage-argocd"),
            ("change-cell-prod-lon-verified", "prod-lon-verification"),
            ("change-cell-prod-ams-promoted", "prod-ams-promotion"),
        ] {
            window.click(cell, cx);
            assert_eq!(
                page.read(cx).selected().map(|s| s.as_ref()),
                Some(hop),
                "{cell}"
            );
            window.render_frame(cx);
            assert!(window.try_find("change-detail").is_some(), "{cell}");
        }
        // The Stage cell is its first gate.
        window.click("change-stage-prod-ams", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("prod-ams-eligible")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_start_strip_cards_select_the_pull_request_the_build_and_the_freight(
    cx: &mut TestAppContext,
) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (card, hop) in [
            ("change-strip-pr", "pr"),
            ("change-strip-build", "push-run"),
            ("change-strip-freight", "freight"),
        ] {
            window.click(card, cx);
            assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some(hop));
        }
        // A hop of the Build selects the Build card.
        page.update(cx, |page, cx| page.select("signature", window, cx));
        assert!(
            page.read(cx)
                .strip
                .cards
                .iter()
                .any(|card| card.id == "build"
                    && card.hops.iter().any(|hop| hop.as_ref() == "signature"))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_overview_keys_walk_cells_and_the_strip_open_clear_back_and_switch(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let recorded = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, event, _| {
            recorded.borrow_mut().push(match event {
                ChangeEvent::Back => "back",
                ChangeEvent::Open(_) | ChangeEvent::Switch(..) => "open",
                _ => "other",
            })
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        let selected =
            |page: &Entity<ChangePage>, cx: &App| page.read(cx).selected().map(|s| s.to_string());
        window.press("down", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("dev-eligible"));
        window.press("right", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("dev-promotion"));
        window.press("right", cx);
        window.press("right", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("dev-argocd"));
        window.press("down", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("stage-argocd"));
        window.press("up", cx);
        window.press("left", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("dev-verification"));
        // The keys stop at the row's ends.
        for _ in 0..10 {
            window.press("left", cx);
        }
        assert_eq!(selected(&page, cx).as_deref(), Some("dev-eligible"));
        // Up from the top row goes to the strip, and along it.
        window.press("up", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("freight"));
        window.press("left", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("push-run"));
        window.press("left", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("pr"));
        window.press("right", cx);
        window.press("right", cx);
        window.press("right", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("freight"));
        // Down from the strip enters the matrix.
        window.press("down", cx);
        assert_eq!(selected(&page, cx).as_deref(), Some("dev-eligible"));
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.press("o", cx);
        window.press("enter", cx);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Hops);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Overview);
        window.press("escape", cx);
        assert!(page.read(cx).selected.is_none());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(&*events.borrow(), &["open", "open", "back"]);
}

#[gpui_kit::test]
fn the_status_filter_shows_the_stages_it_counts_and_a_selection_gives_it_way(
    cx: &mut TestAppContext,
) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.toggle_filter(Tone::Crit, cx));
        window.render_frame(cx);
        assert!(window.try_find("change-matrix-row-prod-lon").is_some());
        assert!(window.try_find("change-matrix-row-dev").is_none());
        page.update(cx, |page, cx| page.select("dev-verification", window, cx));
        window.render_frame(cx);
        assert_eq!(page.read(cx).filter, None);
        assert!(window.try_find("change-matrix-row-dev").is_some());
        page.update(cx, |page, cx| page.set_view(ChangeView::Hops, window, cx));
        window.render_frame(cx);
        assert!(!page.read(cx).folded.contains(&2));
        assert!(window.find("change-hop-dev-verification").visible());
    })
    .unwrap();
}
