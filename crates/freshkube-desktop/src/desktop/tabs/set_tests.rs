use freshkube_nav::{LinkPage, ViewState};

use super::set::*;

fn page(page: LinkPage) -> ViewState {
    ViewState::page(page)
}

fn three() -> (TabSet, TabId, TabId, TabId) {
    let mut set = TabSet::new(page(LinkPage::Overview));
    let a = set.current().id;
    let b = set.open(page(LinkPage::Nodes), false).unwrap();
    let c = set.open(page(LinkPage::Etcd), false).unwrap();
    (set, a, b, c)
}

#[test]
fn a_background_tab_opens_after_the_shown_one_and_the_view_stays() {
    let (set, a, b, c) = three();
    // Each opened right after the shown tab, so the later one sits first.
    let ids: Vec<_> = set.tabs().iter().map(|tab| tab.id).collect();
    assert_eq!(ids, [a, c, b]);
    assert_eq!(set.current().id, a);
}

#[test]
fn a_front_tab_is_shown() {
    let mut set = TabSet::new(page(LinkPage::Overview));
    let id = set.open(page(LinkPage::Health), true).unwrap();
    assert_eq!(set.current().id, id);
    assert_eq!(set.selected(), 1);
}

#[test]
fn the_set_is_capped() {
    let mut set = TabSet::new(page(LinkPage::Overview));
    for _ in 1..MAX_TABS {
        assert!(set.open(page(LinkPage::Nodes), false).is_some());
    }
    assert!(set.is_full());
    assert_eq!(set.open(page(LinkPage::Nodes), true), None);
    assert_eq!(set.len(), MAX_TABS);
}

#[test]
fn duplicating_copies_the_view_with_a_history_of_its_own() {
    let mut set = TabSet::new(page(LinkPage::Overview));
    set.record(page(LinkPage::Nodes));
    assert!(set.current().can_go_back());
    let id = set.duplicate().unwrap();
    assert_eq!(set.current().id, id);
    assert_eq!(set.current().view, page(LinkPage::Nodes));
    assert!(!set.current().can_go_back());
}

#[test]
fn stepping_wraps_and_a_lone_tab_does_not_step() {
    let (mut set, ..) = three();
    assert_eq!(set.step(1), Some(page(LinkPage::Etcd)));
    assert_eq!(set.step(1), Some(page(LinkPage::Nodes)));
    assert_eq!(set.step(1), Some(page(LinkPage::Overview)));
    assert_eq!(set.step(-1), Some(page(LinkPage::Nodes)));
    let mut one = TabSet::new(page(LinkPage::Overview));
    assert_eq!(one.step(1), None);
}

#[test]
fn closing_the_shown_tab_shows_its_right_neighbour_then_the_left() {
    let (mut set, a, b, c) = three();
    // Order: a, c, b.
    set.select(c);
    assert_eq!(set.close(c), Some(Shown::Moved(page(LinkPage::Nodes))));
    assert_eq!(set.current().id, b);
    assert_eq!(set.close(b), Some(Shown::Moved(page(LinkPage::Overview))));
    assert_eq!(set.current().id, a);
}

#[test]
fn closing_another_tab_keeps_the_shown_one() {
    let (mut set, a, b, c) = three();
    set.select(b);
    assert_eq!(set.close(a), Some(Shown::Same));
    assert_eq!(set.current().id, b);
    assert_eq!(set.close(c), Some(Shown::Same));
    assert_eq!(set.current().id, b);
}

#[test]
fn the_last_tab_does_not_close() {
    let mut set = TabSet::new(page(LinkPage::Overview));
    let id = set.current().id;
    assert_eq!(set.close(id), None);
    assert_eq!(set.len(), 1);
}

#[test]
fn reopening_brings_back_the_last_closed_in_front() {
    let (mut set, _, b, c) = three();
    set.close(b);
    set.close(c);
    assert!(set.can_reopen());
    assert_eq!(set.reopen(), Some(page(LinkPage::Etcd)));
    assert_eq!(set.current().view, page(LinkPage::Etcd));
    assert_eq!(set.reopen(), Some(page(LinkPage::Nodes)));
    assert_eq!(set.reopen(), None);
}

#[test]
fn close_others_and_to_the_right() {
    let (mut set, a, b, c) = three();
    // Order: a, c, b; a shown.
    assert_eq!(set.close_to_right(c), Some(Shown::Same));
    assert_eq!(set.len(), 2);
    assert_eq!(set.close_to_right(c), None);
    set.select(c);
    assert_eq!(
        set.close_others(a),
        Some(Shown::Moved(page(LinkPage::Overview)))
    );
    assert_eq!(set.len(), 1);
    assert_eq!(set.current().id, a);
    // The three closed ones are all there to reopen.
    assert!(set.reopen().is_some());
    let _ = b;
}

#[test]
fn closing_to_the_right_of_a_tab_left_of_the_shown_one_shows_it() {
    let (mut set, a, ..) = three();
    set.step(-1);
    assert_eq!(
        set.close_to_right(a),
        Some(Shown::Moved(page(LinkPage::Overview)))
    );
    assert_eq!(set.current().id, a);
}

#[test]
fn history_goes_back_and_forward_and_a_new_view_clears_the_forward() {
    let mut set = TabSet::new(page(LinkPage::Overview));
    assert!(set.record(page(LinkPage::Nodes)));
    assert!(!set.record(page(LinkPage::Nodes)));
    set.record(page(LinkPage::Etcd));
    assert_eq!(set.go_back(), Some(page(LinkPage::Nodes)));
    assert_eq!(set.go_back(), Some(page(LinkPage::Overview)));
    assert_eq!(set.go_back(), None);
    assert_eq!(set.go_forward(), Some(page(LinkPage::Nodes)));
    set.record(page(LinkPage::Health));
    assert!(!set.current().can_go_forward());
    assert_eq!(set.go_forward(), None);
}

#[test]
fn history_keeps_the_newest_steps() {
    let mut set = TabSet::new(page(LinkPage::Overview));
    for i in 0..(HISTORY + 10) {
        let next = if i % 2 == 0 {
            LinkPage::Nodes
        } else {
            LinkPage::Etcd
        };
        set.record(page(next));
    }
    let mut steps = 0;
    while set.go_back().is_some() {
        steps += 1;
    }
    assert_eq!(steps, HISTORY);
}

#[test]
fn restoring_keeps_the_selection_and_the_cap() {
    let views: Vec<_> = (0..SAVED_TABS + 4).map(|_| page(LinkPage::Nodes)).collect();
    let set = TabSet::restored(views, 99, page(LinkPage::Overview));
    assert_eq!(set.len(), 8);
    assert_eq!(set.selected(), 7);
    let empty = TabSet::restored(Vec::new(), 3, page(LinkPage::Overview));
    assert_eq!(empty.views(), [page(LinkPage::Overview)]);
}

#[test]
fn ids_are_not_reused() {
    let (mut set, _, b, _) = three();
    set.close(b);
    let fresh = set.open(page(LinkPage::Health), false).unwrap();
    assert_ne!(fresh, b);
}
