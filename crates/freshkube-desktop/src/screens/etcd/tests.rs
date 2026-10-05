use std::sync::Arc;

use freshkube_core::inspection::{InspectionSource, InspectionUnavailable, assemble_etcd_health};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, SharedString, TestAppContext, WindowHandle, px, size};
use talos_rs::{EtcdAlarm, EtcdAlarmType, EtcdMemberInfo, EtcdMemberStatus};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    EtcdHealthSnapshot, EtcdScreen, MemberRole, MemberState, ScreenPanel, ScreenSource, alarm_tone,
    quorum_banner, quorum_view,
};
use crate::backend::Target;
use crate::desktop::{layout_check, tests::fixture as app};
use crate::ui::Tone;
use crate::{fixture, presentation};

/// The page's frame reaches the split, whose details pane is always drawn
/// beside or below the table, so the table is checked on its own.
const ETCD_FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "etcd-page",
    title: "etcd-title",
    title_text: "etcd",
    content: "etcd-split",
};

const ETCD_PAGE: layout_check::TablePage = layout_check::TablePage {
    page: "etcd-page",
    title: "etcd-title",
    title_text: "etcd",
    table: "etcd-table-scroll",
    list: "etcd-list",
};

const ETCD_TABLE: layout_check::Table = layout_check::Table {
    table: Some("etcd-table-scroll"),
    list: "etcd-list",
};

/// The row of the member at `ix` in the roster: rows are named by member.
fn row(screen: &Entity<EtcdScreen>, ix: usize, cx: &gpui_kit::App) -> SharedString {
    format!("etcd-member-{:x}", screen.read(cx).members()[ix].info.id).into()
}

fn role(id: u64) -> SharedString {
    format!("etcd-role-{id:x}").into()
}

fn snapshot(members: u64, answered: impl IntoIterator<Item = (u64, u64)>) -> EtcdHealthSnapshot {
    assemble_etcd_health(
        freshkube_core::inspection::InspectionTarget::new("cp-1", "10.0.0.1"),
        (1..=members)
            .map(|id| info(id, &format!("cp-{id}")))
            .collect(),
        answered
            .into_iter()
            .map(|(id, leader)| status(id, leader))
            .collect(),
        Vec::new(),
        Vec::new(),
    )
}

/// Shows `snapshot` as the screen's answer for its own target.
fn answer(screen: &Entity<EtcdScreen>, snapshot: EtcdHealthSnapshot, cx: &mut gpui_kit::App) {
    let target = source("talos-cp-fra1-01").target;
    screen.update(cx, |screen, cx| {
        screen.loader.resolve(target, Ok(snapshot));
        cx.notify();
    });
}

fn source(node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: "prod-fra".into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

fn mount(cx: &mut TestAppContext, node: &str) -> (Runtime, Entity<EtcdScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = EtcdScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

fn info(id: u64, name: &str) -> EtcdMemberInfo {
    EtcdMemberInfo {
        id,
        hostname: name.into(),
        peer_urls: vec![format!("https://10.0.0.{id}:2380")],
        client_urls: vec![format!("https://10.0.0.{id}:2379")],
        is_learner: false,
    }
}

fn status(id: u64, leader: u64) -> EtcdMemberStatus {
    EtcdMemberStatus {
        node: format!("n{id}"),
        member_id: id,
        protocol_version: "3.6.0".into(),
        db_size: 1 << 20,
        db_size_in_use: 1 << 19,
        leader_id: leader,
        raft_index: 100,
        raft_term: 2,
        raft_applied_index: 100,
        errors: Vec::new(),
        is_learner: false,
    }
}

#[gpui_kit::test]
fn keyboard_selects_members_and_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).members().len(), 3);
        let rows: Vec<SharedString> = (0..3).map(|ix| row(&screen, ix, cx)).collect();
        window.click(rows[0].clone(), cx);
        assert_eq!(window.find(rows[0].clone()).selected(), Some(true));
        window.press("down", cx);
        assert_eq!(window.find(rows[1].clone()).selected(), Some(true));
        assert_eq!(window.find(rows[0].clone()).selected(), Some(false));
        let name = screen
            .read(cx)
            .selected_member()
            .unwrap()
            .info
            .hostname
            .clone();
        assert_eq!(name, "talos-cp-fra1-02");
        assert!(
            window
                .find("etcd-details")
                .label()
                .is_some_and(|label| label.contains(&name))
        );
        window.press("end", cx);
        assert_eq!(window.find(rows[2].clone()).selected(), Some(true));
        assert!(
            window
                .find("etcd-details")
                .label()
                .is_some_and(|label| label.contains("talos-cp-fra1-03-baremetal-rack-b7"))
        );
        window.press("home", cx);
        assert_eq!(window.find(rows[0].clone()).selected(), Some(true));
        window.press("escape", cx);
        assert_eq!(window.find(rows[0].clone()).selected(), Some(false));
        assert!(window.try_find("etcd-details").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn leader_is_marked_and_quorum_is_healthy(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let ids: Vec<u64> = screen
            .read(cx)
            .members()
            .iter()
            .map(|m| m.info.id)
            .collect();
        assert_eq!(window.find(role(ids[0])).label(), Some("Leader"));
        assert_eq!(window.find(role(ids[1])).label(), Some("Follower"));
        assert_eq!(window.find(role(ids[2])).label(), Some("Follower"));
        let label = window.find("etcd-quorum").label().unwrap().to_owned();
        assert!(
            label.contains("tolerates 1 additional member failure"),
            "{label}"
        );
        assert!(window.try_find("partial-notice").is_none());
        let snapshot = screen.read(cx).loader.data().unwrap().clone();
        assert!(snapshot.quorum.has_quorum());
        // A calm quorum and no alarms: no banners, the meta says so.
        assert!(window.try_find("etcd-quorum-banner").is_none());
        assert!(window.try_find("etcd-alarms").is_none());
        window.find("etcd-scope");
        let meta = &screen.read(cx).derived.meta_after;
        assert!(meta.iter().any(|part| part == "no alarms"), "{meta:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_member_is_not_reported_rather_than_failed(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        let target = source("talos-cp-fra1-01").target;
        screen.update(cx, |screen, cx| {
            let snapshot = assemble_etcd_health(
                source("talos-cp-fra1-01").inspection_target(),
                vec![info(1, "cp-a"), info(2, "cp-b"), info(3, "cp-c")],
                vec![status(1, 1), status(2, 1)],
                Vec::new(),
                Vec::new(),
            );
            screen.loader.resolve(target, Ok(snapshot));
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find(role(3)).label(), Some("Not reported"));
        let row = window.find("etcd-member-3").label().unwrap().to_owned();
        assert!(!row.to_lowercase().contains("fail"), "{row}");
        assert!(!row.to_lowercase().contains("down"), "{row}");
        window.find("partial-notice");
        // Two of three still answer: degraded, never "no quorum".
        let quorum = window.find("etcd-quorum").label().unwrap().to_owned();
        assert!(quorum.starts_with("Degraded"), "{quorum}");
        assert!(
            quorum.contains("tolerates 0 additional member failures"),
            "{quorum}"
        );
        // A leader and no failure to spare: at risk, a warning, not critical.
        assert!(
            window
                .find("etcd-quorum-banner")
                .label()
                .is_some_and(|label| label.starts_with("Quorum at risk")),
        );
        window.click("etcd-member-3", cx);
        window.render_frame(cx);
        assert_eq!(
            screen.read(cx).selected_member().map(MemberRole::of),
            Some(MemberRole::NotReported)
        );
    })
    .unwrap();
}

#[test]
fn quorum_display_has_no_spare_capacity_at_two_of_three_or_three_of_five() {
    for (answered, total) in [(2, 3), (3, 5)] {
        let snapshot = assemble_etcd_health(
            freshkube_core::inspection::InspectionTarget::new("cp-a", "10.0.0.1"),
            (1..=total)
                .map(|id| info(id, &format!("cp-{id}")))
                .collect(),
            (1..=answered).map(|id| status(id, 1)).collect(),
            Vec::new(),
            Vec::new(),
        );
        let view = quorum_view(&snapshot);
        assert_eq!(view.label, "Degraded");
        assert_eq!(view.tone, crate::ui::Tone::Warn);
        assert!(
            view.detail
                .contains("tolerates 0 additional member failures")
        );
    }
}

#[test]
fn missing_statuses_make_quorum_unknown_not_lost() {
    let snapshot = assemble_etcd_health(
        presentation::node_summaries(&fixture::cluster("prod-fra", 1))
            .first()
            .map(|node| {
                freshkube_core::inspection::InspectionTarget::new(&node.name, &node.address)
            })
            .unwrap(),
        vec![info(1, "a"), info(2, "b"), info(3, "c")],
        Vec::new(),
        Vec::new(),
        vec![InspectionUnavailable {
            source: InspectionSource::EtcdStatus,
            message: "timed out".into(),
        }],
    );
    let view = quorum_view(&snapshot);
    assert_eq!(view.label, "Quorum not reported");
}

#[gpui_kit::test]
fn alarms_and_errors_are_shown_when_reported(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        let target = source("talos-cp-fra1-01").target;
        screen.update(cx, |screen, cx| {
            let mut broken = status(2, 1);
            broken.errors = vec!["etcdserver: no space".into()];
            let snapshot = assemble_etcd_health(
                source("talos-cp-fra1-01").inspection_target(),
                vec![info(1, "cp-a"), info(2, "cp-b")],
                vec![status(1, 1), broken],
                vec![EtcdAlarm {
                    node: "cp-b".into(),
                    member_id: 2,
                    alarm_type: EtcdAlarmType::NoSpace,
                }],
                Vec::new(),
            );
            screen.loader.resolve(target, Ok(snapshot));
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find(("etcd-alarm", 0usize)).label(),
            Some("NOSPACE on cp-b")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some(1);
            assert!(screen.loader.data().is_some());
            screen.set_source(Some(source("talos-cp-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_none());
        window.find("screen-retry");
        assert!(window.try_find("etcd-list").is_none());
    })
    .unwrap();
}

#[test]
fn quorum_banners_come_from_the_members_answers() {
    let banner = |members, answered: &[(u64, u64)]| {
        quorum_banner(&snapshot(members, answered.iter().copied()))
            .map(|banner| (banner.tone, banner.lead, banner.body))
    };
    // Members answered and none reports a leader: lost, critical.
    let (tone, lead, body) = banner(3, &[(1, 0), (2, 0), (3, 0)]).unwrap();
    assert_eq!((tone, lead), (Tone::Crit, "No quorum"));
    assert!(body.starts_with("3 of 3 voting members answered"), "{body}");
    // A leader and no failure to spare.
    let (tone, lead, body) = banner(3, &[(1, 1), (2, 1)]).unwrap();
    assert_eq!((tone, lead), (Tone::Warn, "Quorum at risk"));
    assert!(
        body.contains("2 of 3") && body.contains("needs 2") && body.contains("one more failure"),
        "{body}"
    );
    let (tone, lead, _) = banner(2, &[(1, 1), (2, 1)]).unwrap();
    assert_eq!((tone, lead), (Tone::Warn, "Quorum at risk"));
    // Too few answered to confirm, but a leader is reported: never critical.
    let (tone, lead, body) = banner(5, &[(1, 1), (2, 1)]).unwrap();
    assert_eq!((tone, lead), (Tone::Warn, "Quorum unconfirmed"));
    assert!(
        body.contains("2 of 5") && body.contains("needs 3"),
        "{body}"
    );
    // Calm: a failure to spare, or one member by design.
    assert!(banner(3, &[(1, 1), (2, 1), (3, 1)]).is_none());
    assert!(banner(5, &[(1, 1), (2, 1), (3, 1), (4, 1)]).is_none());
    assert!(banner(1, &[(1, 1)]).is_none());

    // The summary agrees with the banner: lost when the banner says so, and
    // a single member calm.
    let summary = |members, answered: &[(u64, u64)]| {
        let view = quorum_view(&snapshot(members, answered.iter().copied()));
        (view.tone, view.label)
    };
    assert_eq!(
        summary(3, &[(1, 0), (2, 0), (3, 0)]),
        (Tone::Crit, "No quorum")
    );
    assert_eq!(summary(1, &[(1, 1)]), (Tone::Good, "Quorum"));
    assert_eq!(summary(2, &[(1, 1), (2, 1)]), (Tone::Warn, "Quorum"));
    assert_eq!(
        summary(3, &[(1, 1), (2, 1), (3, 1)]),
        (Tone::Good, "Quorum")
    );
    // Nothing answered: not reported, which the partial notice says.
    assert!(banner(3, &[]).is_none());
}

#[gpui_kit::test]
fn a_lost_quorum_shows_a_critical_banner(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        answer(&screen, snapshot(3, [(1, 0), (2, 0), (3, 0)]), cx);
        window.render_frame(cx);
        let label = window
            .find("etcd-quorum-banner")
            .label()
            .unwrap()
            .to_owned();
        assert!(label.starts_with("No quorum"), "{label}");
        // No member shows green under it: each answers, but follows no
        // leader, so its glyph warns and its role says so.
        for id in 1..=3u64 {
            let glyph = SharedString::from(format!("etcd-member-health-{id:x}"));
            assert_eq!(window.find(glyph).label(), Some("No leader"));
            let role = SharedString::from(format!("etcd-role-{id:x}"));
            assert_eq!(window.find(role).label(), Some("No leader"));
        }
        assert_eq!(
            window.find("etcd-tally-without-a-leader").label(),
            Some("3 without a leader")
        );
        assert_eq!(window.find("etcd-tally-healthy").label(), Some("0 healthy"));
        assert!(
            screen
                .read(cx)
                .derived
                .rows
                .iter()
                .all(|row| row.state.tone() == Tone::Warn)
        );
        // A recovered answer takes the banner away.
        answer(&screen, snapshot(3, [(1, 1), (2, 1), (3, 1)]), cx);
        window.render_frame(cx);
        assert!(window.try_find("etcd-quorum-banner").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_alarm_banner_shows_three_and_counts_the_rest(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        let alarms = (1..=5)
            .map(|id| EtcdAlarm {
                node: format!("cp-{id}"),
                member_id: id,
                alarm_type: EtcdAlarmType::NoSpace,
            })
            .collect();
        let snapshot = assemble_etcd_health(
            freshkube_core::inspection::InspectionTarget::new("cp-1", "10.0.0.1"),
            (1..=5).map(|id| info(id, &format!("cp-{id}"))).collect(),
            (1..=5).map(|id| status(id, 1)).collect(),
            alarms,
            Vec::new(),
        );
        answer(&screen, snapshot, cx);
        window.render_frame(cx);
        assert_eq!(window.find("etcd-alarms").label(), Some("etcd alarms"));
        // NOSPACE refuses writes: the banner is as critical as its rows.
        assert_eq!(screen.read(cx).derived.alarm_tone, Tone::Crit);
        assert_eq!(
            window.find("etcd-tally-with-issues").label(),
            Some("5 with issues")
        );
        for ix in 0..3usize {
            window.find(("etcd-alarm", ix));
        }
        assert!(window.try_find(("etcd-alarm", 3usize)).is_none());
        window.find("etcd-alarm-more");
        // Each member's own alarm stays in its details.
        window.click("etcd-member-5", cx);
        window.render_frame(cx);
        window.find("etcd-details");
    })
    .unwrap();
}

#[gpui_kit::test]
fn status_chips_filter_the_members_and_clear(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        answer(&screen, snapshot(3, [(1, 1), (2, 1)]), cx);
        window.render_frame(cx);
        assert!(
            window
                .find("etcd-tally-not-reported")
                .label()
                .is_some_and(|label| label.starts_with('1')),
        );
        window.click("etcd-tally-not-reported", cx);
        window.render_frame(cx);
        window.find("etcd-member-3");
        assert!(window.try_find("etcd-member-1").is_none());
        window.click("etcd-tally-not-reported", cx);
        window.render_frame(cx);
        window.find("etcd-member-1");
        window.find("etcd-member-3");
    })
    .unwrap();
}

#[gpui_kit::test]
fn etcd_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = app(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.press("secondary-6", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // At rest the table is the page's; with a member selected the
            // split of table and details runs edge to edge in its place.
            layout_check::assert_table_page(window, cx, &ETCD_PAGE);
            window.press("down", cx);
            layout_check::assert_edge_frame(window, cx, &ETCD_FRAME);
            layout_check::assert_table(window, cx, &ETCD_TABLE);
            window.press("escape", cx);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_details_sit_beside_the_table_when_wide_and_below_when_narrow(cx: &mut TestAppContext) {
    for (width, height, text, beside) in [(1280., 880., None, true), (760., 560., Some(20.), false)]
    {
        let (_runtime, handle, _view) = app(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            window.press("secondary-6", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // At rest the table has the page; a selection opens the details.
            assert!(window.try_find("etcd-details").is_none());
            window.press("down", cx);
            window.render_frame(cx);
            let table = window.find("etcd-table-scroll").bounds();
            let details = window.find("etcd-details").bounds();
            if beside {
                assert!(details.left() >= table.right(), "{table:?} {details:?}");
            } else {
                assert!(details.top() >= table.bottom(), "{table:?} {details:?}");
            }
        })
        .unwrap();
    }
}

#[test]
fn every_etcd_alarm_is_critical() {
    // NOSPACE stops writes and CORRUPT is data corruption; a kind this app
    // doesn't know counts its member as having issues, so it matches.
    for alarm in [
        EtcdAlarmType::NoSpace,
        EtcdAlarmType::Corrupt,
        EtcdAlarmType::Unknown(9),
    ] {
        assert_eq!(alarm_tone(&alarm), Tone::Crit, "{alarm:?}");
    }
}

#[test]
fn a_member_without_a_leader_warns_and_never_reads_healthy() {
    let lost = snapshot(3, [(1, 0), (2, 0), (3, 0)]);
    for member in &lost.members {
        assert_eq!(MemberState::of(member), MemberState::NoLeader);
        assert_eq!(MemberRole::of(member), MemberRole::NoLeader);
    }
    let led = snapshot(3, [(1, 1), (2, 1), (3, 1)]);
    let states: Vec<_> = led.members.iter().map(MemberState::of).collect();
    assert_eq!(states, [MemberState::Healthy; 3]);
    // A member that answered nothing stays not reported, not leaderless.
    let silent = snapshot(3, [(1, 1), (2, 1)]);
    assert_eq!(
        MemberState::of(&silent.members[2]),
        MemberState::NotReported
    );
}
