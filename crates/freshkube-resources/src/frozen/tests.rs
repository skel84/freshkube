use std::time::SystemTime;

use super::*;
use crate::model::{ResourceIdentity, ResourceRow};

fn row(name: &str, cell: usize) -> ResourceRow {
    ResourceRow {
        identity: ResourceIdentity {
            connection: "conn".into(),
            resource: "pods".into(),
            namespace: "payments".into(),
            name: name.into(),
            uid: format!("uid-{name}"),
        },
        cells: vec!["x".repeat(cell)],
        created: None,
        terminating: false,
        resource_version: "1".into(),
        owner: None,
        generated: None,
        pod: None,
    }
}

fn capture(rows: usize, cell: usize) -> Option<FrozenRows> {
    let list: Vec<_> = (0..rows).map(|n| row(&format!("pod-{n}"), cell)).collect();
    FrozenRows::capture(
        ("pods".into(), None, "conn".into()),
        Vec::new(),
        list,
        rows,
        SystemTime::now(),
    )
}

#[test]
fn a_list_keeps_every_row_within_its_share() {
    let frozen = capture(1_204, 8).expect("kept");
    assert_eq!((frozen.kept(), frozen.total()), (1_204, 1_204));
}

#[test]
fn a_list_over_the_row_cap_keeps_none() {
    assert!(capture(MAX_ROWS, 4).is_some());
    assert!(capture(MAX_ROWS + 1, 4).is_none());
}

#[test]
fn an_empty_list_keeps_nothing() {
    assert!(capture(0, 4).is_none());
}

#[test]
fn a_list_too_big_for_its_share_keeps_its_first_rows() {
    // Wide cells: 5,000 rows are over the tab's share of the budget.
    let frozen = capture(MAX_ROWS, 4_000).expect("some kept");
    assert!(frozen.kept() < frozen.total());
    assert_eq!(frozen.total(), MAX_ROWS);
    assert!(frozen.bytes() <= MAX_BYTES / MAX_TABS);
    assert_eq!(frozen.rows[0].identity.name, "pod-0");
}

#[test]
fn only_eight_tabs_keep_rows_and_the_oldest_goes_first() {
    let mut set = FrozenSet::default();
    for tab in 0..MAX_TABS + 3 {
        set.insert(tab, capture(10, 4).unwrap());
    }
    assert_eq!(set.len(), MAX_TABS);
    assert!(!set.contains(&0) && !set.contains(&2));
    assert!(set.contains(&3) && set.contains(&(MAX_TABS + 2)));
}

#[test]
fn the_total_stays_within_the_budget() {
    let mut set = FrozenSet::default();
    for tab in 0..MAX_TABS {
        set.insert(tab, capture(MAX_ROWS, 2_000).unwrap());
        assert!(set.bytes() <= MAX_BYTES, "{} bytes", set.bytes());
    }
    assert!(!set.is_empty());
}

#[test]
fn freezing_a_tab_again_replaces_what_it_kept() {
    let mut set = FrozenSet::default();
    set.insert(1, capture(10, 4).unwrap());
    set.insert(1, capture(20, 4).unwrap());
    assert_eq!(set.len(), 1);
    assert_eq!(set.take(&1).unwrap().kept(), 20);
    assert!(set.take(&1).is_none());
}
