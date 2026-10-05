use std::sync::Arc;

use freshkube_core::inspection::{InspectionSource, InspectionUnavailable, assemble_etcd_health};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use talos_rs::{EtcdAlarm, EtcdAlarmType, EtcdMemberInfo, EtcdMemberStatus};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{EtcdScreen, MemberRole, ScreenPanel, ScreenSource, quorum_view};
use crate::backend::Target;
use crate::{fixture, presentation};

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
        window.click(("etcd-member", 0usize), cx);
        assert_eq!(window.find(("etcd-member", 0usize)).selected(), Some(true));
        window.press("down", cx);
        assert_eq!(window.find(("etcd-member", 1usize)).selected(), Some(true));
        assert_eq!(window.find(("etcd-member", 0usize)).selected(), Some(false));
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
        assert_eq!(window.find(("etcd-member", 2usize)).selected(), Some(true));
        assert!(
            window
                .find("etcd-details")
                .label()
                .is_some_and(|label| label.contains("talos-cp-fra1-03-baremetal-rack-b7"))
        );
        window.press("home", cx);
        assert_eq!(window.find(("etcd-member", 0usize)).selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn leader_is_marked_and_quorum_is_healthy(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(("etcd-leader", 0usize));
        assert!(window.try_find(("etcd-leader", 1usize)).is_none());
        assert_eq!(window.find(("etcd-role", 0usize)).label(), Some("Leader"));
        assert_eq!(window.find(("etcd-role", 1usize)).label(), Some("Follower"));
        let label = window.find("etcd-quorum").label().unwrap().to_owned();
        assert!(
            label.contains("tolerates 1 additional member failure"),
            "{label}"
        );
        assert!(window.try_find("partial-notice").is_none());
        let snapshot = screen.read(cx).loader.data().unwrap().clone();
        assert!(snapshot.quorum.has_quorum());
        window.find("etcd-alarms");
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
        assert_eq!(
            window.find(("etcd-role", 2usize)).label(),
            Some("Not reported")
        );
        let row = window
            .find(("etcd-member", 2usize))
            .label()
            .unwrap()
            .to_owned();
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
        window.click(("etcd-member", 2usize), cx);
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
