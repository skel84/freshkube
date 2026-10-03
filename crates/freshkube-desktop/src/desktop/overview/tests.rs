use crate::{
    desktop::{Page, nodes::NodeTab, tests::fixture},
    resources::Tab,
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

#[gpui_kit::test]
fn every_overview_card_opens_its_target_and_filter(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1000.);
    cx.update_window(handle, |_, window, cx| {
        for (id, page, kind, filter) in [
            ("tile-nodes", Page::Nodes, None, None),
            ("tile-etcd", Page::Etcd, None, None),
            ("tile-workloads", Page::Health, None, None),
            ("tile-pods", Page::Resources, Some("pods"), None),
            ("tile-services", Page::SystemServices, None, None),
            ("tile-memory", Page::Nodes, None, None),
            (
                "tile-events",
                Page::Resources,
                Some("events"),
                Some("Warning"),
            ),
            (
                "tile-storage",
                Page::Resources,
                Some("persistentvolumeclaims"),
                Some("Pending"),
            ),
        ] {
            window.click("nav-overview", cx);
            window.render_frame(cx);
            let expected_filter = if id == "tile-pods" {
                pilot
                    .read(cx)
                    .kubernetes_summary
                    .data()
                    .unwrap()
                    .pods
                    .loaded()
                    .unwrap()
                    .issues
                    .first()
                    .map(|pod| pod.issue.label().to_owned())
            } else {
                filter.map(str::to_owned)
            };
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).page, page, "{id}");
            if let Some(kind) = kind {
                assert_eq!(pilot.read(cx).resource_kind.key(), kind);
            }
            if let Some(filter) = expected_filter {
                assert_eq!(pilot.read(cx).resources.read(cx).filter_value(cx), filter);
            }
            if id == "tile-memory" {
                assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Processes);
                assert!(pilot.read(cx).node_workspace.open);
            }
            if id == "tile-services" {
                assert!(
                    pilot
                        .read(cx)
                        .system_services
                        .read(cx)
                        .is_unhealthy_filter()
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_attention_subject_opens_and_optional_actions_follow(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1600.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, _| {
            pilot.attention_expanded = true;
        });
        window.render_frame(cx);
        let rows = pilot.read(cx).attention.rows.clone();
        assert!(rows.iter().any(|row| row.kind == "Node"));
        assert!(rows.iter().any(|row| row.kind == "Pod"));
        assert!(rows.iter().any(|row| row.kind == "Deployment"));
        assert!(rows.iter().any(|row| row.kind == "System service"));
        assert!(rows.iter().any(|row| row.kind == "Claim"));
        for row in rows {
            for (id, available) in [
                ("attention-open", true),
                ("attention-logs", row.logs.is_some()),
                ("attention-open-node", row.open_node.is_some()),
            ] {
                if !available {
                    continue;
                }
                window.click("nav-overview", cx);
                window.render_frame(cx);
                window.scroll(
                    "overview-page",
                    gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                        gpui_kit::px(0.),
                        gpui_kit::px(40000.),
                    )),
                    cx,
                );
                window.render_frame(cx);
                let top = window.find(row.id.clone()).bounds().top();
                window.scroll(
                    "overview-page",
                    gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                        gpui_kit::px(0.),
                        gpui_kit::px(500.) - top,
                    )),
                    cx,
                );
                window.render_frame(cx);
                window.within(row.id.clone()).click(id, cx);
                window.render_frame(cx);
                assert_ne!(pilot.read(cx).page, Page::Overview, "{} {id}", row.id);
                if row.kind == "Pod" && id == "attention-logs" {
                    assert_eq!(pilot.read(cx).resources.read(cx).detail_tab(cx), Tab::Logs);
                }
                if id == "attention-open-node" {
                    assert_eq!(pilot.read(cx).page, Page::Nodes);
                    assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Services);
                }
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn attention_merges_node_problems_and_etcd_alarms_and_caps_display(cx: &mut TestAppContext) {
    use crate::presentation::attention;
    use crate::ui::Tone;
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1000.);
    cx.update_window(handle, |_, window, cx| {
        let mut rows = pilot.read(cx).node_workspace.rows.as_ref().clone();
        let bad = rows
            .iter_mut()
            .find(|row| row.name == "talos-wk-fra1-02")
            .unwrap();
        bad.talos.as_mut().unwrap().responding = false;
        let kube = pilot.read(cx).kubernetes_summary.data().unwrap().clone();
        let mut talos = pilot.read(cx).overview.data().unwrap().clone();
        talos.etcd_summary.as_mut().unwrap().has_quorum = false;
        talos.etcd_alarms = Some(vec![talos_rs::EtcdAlarm {
            node: "control".into(),
            member_id: 42,
            alarm_type: talos_rs::EtcdAlarmType::NoSpace,
        }]);
        let attention = attention::build(&rows, Some(&kube), Some(&talos), chrono::Utc::now());
        let node = attention
            .rows
            .iter()
            .filter(|row| row.kind == "Node" && row.name == "talos-wk-fra1-02")
            .collect::<Vec<_>>();
        assert_eq!(node.len(), 1);
        assert!(node[0].reason.contains("Talos API not answering"));
        assert!(node[0].reason.contains("Kubernetes NotReady"));
        let etcd = attention
            .rows
            .iter()
            .filter(|row| row.kind == "etcd")
            .collect::<Vec<_>>();
        assert_eq!(etcd.len(), 1);
        assert!(etcd[0].reason.contains("quorum"));
        assert!(etcd[0].reason.contains("NOSPACE"));
        let first_warn = attention
            .rows
            .iter()
            .position(|row| row.tone == Tone::Warn)
            .unwrap();
        assert!(
            attention.rows[first_warn..]
                .iter()
                .all(|row| row.tone == Tone::Warn)
        );
        pilot.update(cx, |pilot, cx| {
            pilot.attention = attention;
            cx.notify();
        });
        window.render_frame(cx);
        window.scroll(
            "overview-page",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-500.))),
            cx,
        );
        window.render_frame(cx);
        window
            .within("attention-etcd-cluster-etcd")
            .click("attention-open", cx);
        assert_eq!(pilot.read(cx).page, Page::Etcd);
        let mut many = Vec::new();
        for ix in 0..80 {
            let mut row = rows[0].clone();
            row.name = format!("node-{ix:02}").into();
            row.talos.as_mut().unwrap().responding = false;
            many.push(row);
        }
        let capped = attention::build(&many, None, None, chrono::Utc::now());
        assert_eq!(capped.rows.len(), 50);
        assert_eq!(capped.total, 80);
        assert_eq!(capped.more, "Show all 80");
    })
    .unwrap();
}

#[test]
fn kubernetes_only_cards_and_refused_parts_keep_the_other_counts() {
    use crate::{presentation::overview::Overview, resources::example};
    use freshkube_core::kubernetes_summary::Part;
    let mut kube = example::summary("prod-fra", chrono::Utc::now().timestamp());
    kube.events = Part::Refused("forbidden".into());
    let display = Overview::build(&[], &[], Some(&kube), None, true, true);
    assert_eq!(
        display.cards.iter().map(|card| card.id).collect::<Vec<_>>(),
        vec!["tile-nodes", "tile-workloads", "tile-pods", "tile-events"]
    );
    assert_eq!(
        display.cards[2].figure,
        kube.pods.loaded().unwrap().total.to_string()
    );
    assert!(
        display.cards[3]
            .detail
            .contains("Can't read events: forbidden")
    );
    assert!(display.subtitle.contains("Kubernetes v1.32.3"));
}

#[gpui_kit::test]
fn object_frontdoor_asks_before_navigation_and_cancel_keeps_the_shell(cx: &mut TestAppContext) {
    use crate::resources::{example, model::ObjectRef};
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1000.);
    let pod = example::read("prod-fra", "pods", None, chrono::Utc::now().timestamp())
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.cells.iter().any(|cell| cell == "Running"))
        .unwrap()
        .identity;
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                pod.clone().into(),
                Tab::Shell,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("pod-shell-start", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("detail-shell-running").visible());
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("persistentvolumeclaims").unwrap(),
                ObjectRef {
                    namespace: "batch".into(),
                    name: "report-data".into(),
                    uid: String::new(),
                },
                Tab::Overview,
                window,
                cx,
            )
        });
        assert_eq!(pilot.read(cx).resource_kind.key(), "pods");
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).resource_kind.key(), "pods");
        assert!(window.find("detail-shell-running").visible());
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("persistentvolumeclaims").unwrap(),
                ObjectRef {
                    namespace: "batch".into(),
                    name: "report-data".into(),
                    uid: String::new(),
                },
                Tab::Overview,
                window,
                cx,
            )
        });
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).resource_kind.key(), "persistentvolumeclaims");
        assert!(window.try_find("detail-shell-running").is_none());
    })
    .unwrap();
}
