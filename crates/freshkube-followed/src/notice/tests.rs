use chrono::{DateTime, Duration, TimeZone, Utc};

use super::*;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap() + Duration::seconds(seconds)
}

fn update(follow: u64, failure: bool, sensitive: bool) -> Update {
    Update {
        follow,
        state: format!("s{follow}"),
        headline: format!("Headline {follow}"),
        detail: format!("Detail {follow}"),
        failure,
        sensitive,
        at: at(0),
    }
}

#[test]
fn the_first_update_is_told_at_once_and_the_next_within_two_minutes_group() {
    let mut hub = Hub::default();
    let first = hub.push(update(1, false, false), true, at(0));
    assert_eq!(first.map(|n| n.updates.len()), Some(1));
    assert!(hub.push(update(2, false, false), true, at(30)).is_none());
    assert!(hub.push(update(3, false, false), true, at(90)).is_none());
    assert_eq!(hub.due(), Some(at(120)));
    assert!(hub.flush(at(100)).is_none());
    let group = hub.flush(at(120)).expect("the group");
    assert_eq!(group.updates.len(), 2);
    assert_eq!(group.summary(), "Freshkube · 2 updates");
    // The window is over; the next update is told at once.
    assert!(hub.push(update(4, false, false), true, at(200)).is_some());
}

#[test]
fn a_failure_inside_a_window_is_told_at_once_with_what_waited() {
    let mut hub = Hub::default();
    assert!(hub.push(update(1, false, false), true, at(0)).is_some());
    assert!(hub.push(update(2, false, false), true, at(10)).is_none());
    let failure = hub
        .push(update(3, true, false), true, at(20))
        .expect("a failure does not wait");
    assert_eq!(failure.updates.len(), 2);
    assert!(failure.has_failure());
    // Nothing is left to wait for.
    assert!(hub.flush(at(130)).is_none());
}

#[test]
fn without_grouping_each_update_is_told() {
    let mut hub = Hub::default();
    assert!(hub.push(update(1, false, false), false, at(0)).is_some());
    assert!(hub.push(update(2, false, false), false, at(1)).is_some());
    assert!(hub.due().is_none());
}

#[test]
fn quiet_hours_wrap_midnight_and_off_holds_nothing() {
    let quiet = Quiet {
        enabled: true,
        from: 22 * 60,
        until: 7 * 60,
    };
    assert!(quiet.holds(23 * 60));
    assert!(quiet.holds(60));
    assert!(!quiet.holds(7 * 60));
    assert!(!quiet.holds(12 * 60));
    assert!(!Quiet::default().holds(23 * 60));
    let day = Quiet {
        enabled: true,
        from: 9 * 60,
        until: 17 * 60,
    };
    assert!(day.holds(12 * 60));
    assert!(!day.holds(18 * 60));
}

#[test]
fn a_notice_with_a_failure_is_a_failure() {
    let notice = Notice {
        id: 1,
        at: at(0),
        updates: vec![update(1, false, false), update(2, true, false)],
        held: false,
    };
    assert!(notice.has_failure());
}

#[test]
fn hide_details_defaults_on_for_sensitive_clusters_and_the_title_alone_shows() {
    let notice = Notice {
        id: 1,
        at: at(0),
        updates: vec![update(1, true, true)],
        held: false,
    };
    let default = Settings::default();
    let words = notice.words(&default);
    assert_eq!(words.title, "Freshkube · 1 update");
    assert_eq!(words.body, None);
    // Turned off by the person, the details show.
    let shown = Settings {
        hide_details: Some(false),
        ..default
    };
    assert_eq!(notice.words(&shown).title, "Headline 1");
    // An open cluster shows details by default, and can hide them.
    let open = Notice {
        updates: vec![update(1, true, false)],
        ..notice
    };
    assert_eq!(open.words(&default).title, "Headline 1");
    let hidden = Settings {
        hide_details: Some(true),
        ..default
    };
    assert_eq!(open.words(&hidden).body, None);
}

#[test]
fn opening_the_bell_moves_new_to_earlier() {
    let mut inbox = Inbox::default();
    for id in 1..=3 {
        inbox.push(Notice {
            id,
            at: at(0),
            updates: vec![update(id, true, false)],
            held: false,
        });
    }
    assert_eq!(inbox.new.front().map(|n| n.id), Some(3));
    inbox.mark_seen();
    assert!(inbox.new.is_empty());
    assert_eq!(
        inbox.earlier.iter().map(|n| n.id).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
}
