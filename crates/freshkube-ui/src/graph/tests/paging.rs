use super::*;

/// Pins the service map's pages: these were taken before `unpaged` and
/// `Metrics` existed, so a change to its paging, wrap or spacing fails here.
#[test]
fn the_service_map_s_pages_are_pinned() {
    let got = [
        fingerprint(&mut state(8, 12)),
        fingerprint(&mut state(30, 60)),
        fingerprint(&mut state(120, 300)),
    ];
    assert_eq!(got, PINNED, "{got:#x?}");
}

#[test]
fn pages_hold_twenty_four_boxes_and_count_what_they_draw() {
    let state = state(120, 300);
    assert_eq!(state.pages(), 5);
    assert_eq!(state.placed().count(), PER_PAGE);
    assert_eq!(state.visible_count(), 300);
    assert!(state.markers().count() <= MARKERS);
    assert!(state.summary().starts_with("24 of 120 services · "));
    assert!(state.summary().ends_with(" of 300 filtered calls drawn"));
    assert_eq!(state.display.page_label.as_ref(), "Page 1 of 5");
    assert_eq!(state.display.count.as_ref(), "300 of 300 calls");
}

#[test]
fn every_call_can_be_selected_and_its_ends_stay_on_every_page() {
    let mut state = state(120, 300);
    for ix in 0..state.edges().len() {
        let (from, to) = (state.edges()[ix].from, state.edges()[ix].to);
        assert!(state.select(&from, &to));
        assert_eq!(state.page(), from / PER_PAGE);
        for page in 0..state.pages() {
            state.set_page(page);
            let placed = boxes(&state);
            assert!(placed.iter().any(|&(key, ..)| key == from));
            assert!(placed.iter().any(|&(key, ..)| key == to));
            assert!(placed.len() <= PER_PAGE + 2);
            assert!(
                state
                    .markers()
                    .any(|edge| (edge.from, edge.to) == (from, to))
            );
            assert!(state.markers().count() <= MARKERS + 1);
        }
    }
}

#[test]
fn no_call_runs_through_a_box_on_any_page() {
    let mut state = state(120, 300);
    for page in 0..state.pages() {
        state.set_page(page);
        let placed = boxes(&state);
        assert!(state.shown().count() > 0);
        for (edge, route) in state.shown() {
            for segment in &route.segments {
                for step in 0..=16 {
                    let (x, y) = point_at(segment, step as f32 / 16.);
                    let inside = placed.iter().any(|&(_, bx, by)| {
                        x > bx + 0.5
                            && x < bx + NODE_W - 0.5
                            && y > by + 0.5
                            && y < by + NODE_H - 0.5
                    });
                    assert!(!inside, "page {page}: {} at {x}, {y}", edge.label);
                }
            }
        }
    }
}
