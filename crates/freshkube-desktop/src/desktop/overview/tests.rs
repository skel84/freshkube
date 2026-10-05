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
fn cards_wait_for_a_first_answer_and_mark_the_last_known_one() {
    use crate::{
        fixture,
        presentation::{
            node_summaries,
            overview::{CardState, Overview},
        },
        resources::example,
    };
    use freshkube_core::kubernetes_summary::{Part, Unavailable};
    let state = |overview: &Overview, id: &str| {
        overview
            .cards
            .iter()
            .find(|card| card.id == id)
            .unwrap()
            .state
            .clone()
    };
    let waiting = Overview::build(&[], &[], None, None, false, false);
    assert!(
        waiting
            .cards
            .iter()
            .all(|card| card.state == CardState::Waiting),
        "nothing has answered"
    );
    let cluster = fixture::cluster("prod-fra", 0);
    let nodes = node_summaries(&cluster);
    let mut kube = example::summary("prod-fra", chrono::Utc::now().timestamp());
    let current = Overview::build(&[], &nodes, Some(&kube), Some(&cluster), false, false);
    assert!(
        current
            .cards
            .iter()
            .all(|card| card.state == CardState::Current)
    );
    // A refused read without earlier evidence says why in its detail; one
    // with evidence shows it as last known.
    let pods = kube.pods.loaded().cloned();
    kube.events = Part::Refused("forbidden".into());
    kube.pods = Part::Failed(Unavailable {
        failure: None,
        message: "timed out".into(),
        last_good: pods,
    });
    let failed = Overview::build(&[], &nodes, Some(&kube), Some(&cluster), false, false);
    assert_eq!(state(&failed, "tile-events"), CardState::Current);
    assert_eq!(
        state(&failed, "tile-pods"),
        CardState::LastKnown("timed out".into())
    );
    let stale = failed.talos_stale("unreachable".into());
    assert_eq!(
        state(&stale, "tile-etcd"),
        CardState::LastKnown("unreachable".into())
    );
    assert_eq!(state(&stale, "tile-nodes"), CardState::Current);
}

#[gpui_kit::test]
fn waiting_cards_show_a_skeleton(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.kubernetes_summary = Default::default();
            pilot.rebuild_joined_nodes();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.within("tile-pods").find("card-skeleton").visible());
        // The Talos cards have their answer.
        assert!(
            window
                .within("tile-etcd")
                .find("tile-card-content")
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_reaches_a_card_and_enter_or_space_opens_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1000.);
    cx.update_window(handle, |_, window, cx| {
        for (id, key, page) in [
            ("tile-nodes", "enter", Page::Nodes),
            ("tile-etcd", "space", Page::Etcd),
        ] {
            window.click("nav-overview", cx);
            window.render_frame(cx);
            let mut tabs = 0;
            while window.find(id).focused() != Some(true) {
                tabs += 1;
                assert!(tabs < 100, "Tab never reached {id}");
                window.press("tab", cx);
            }
            assert_eq!(pilot.read(cx).page, Page::Overview);
            window.press(key, cx);
            assert_eq!(pilot.read(cx).page, page, "{key} on {id}");
        }
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

#[test]
fn services_and_memory_cards_without_evidence_are_unknown() {
    use crate::{
        desktop::{Area, shell::RailMarks},
        fixture,
        presentation::{node_summaries, overview::Overview},
        ui::Tone,
    };
    use freshkube_core::cluster_overview::ClusterOverview;
    // No node answered: nothing counted, nothing measured.
    let silent = ClusterOverview {
        endpoints: vec!["192.0.2.1".into()],
        ..Default::default()
    };
    let silent = Overview::build(
        &[],
        &node_summaries(&silent),
        None,
        Some(&silent),
        false,
        false,
    );
    // The Talos overview hasn't arrived.
    let waiting = Overview::build(&[], &[], None, None, false, false);
    // Nodes answered, but none of their services reports health.
    let cluster = fixture::cluster("prod-fra", 0);
    let mut nodes = node_summaries(&cluster);
    for service in nodes.iter_mut().flat_map(|node| &mut node.services) {
        service.health = None;
    }
    let unreported = Overview::build(&[], &nodes, None, Some(&cluster), false, false);
    for (case, overview, ids) in [
        ("silent", &silent, &["tile-services", "tile-memory"][..]),
        ("waiting", &waiting, &["tile-services", "tile-memory"][..]),
        ("unreported", &unreported, &["tile-services"][..]),
    ] {
        for id in ids {
            let card = overview.cards.iter().find(|card| card.id == *id).unwrap();
            assert_eq!(card.tone, Tone::Unknown, "{case}: {id}");
        }
        // An unknown card leaves its area without a dot, as a good one does.
        let marks = RailMarks::from_cards(&overview.cards, false);
        assert_eq!(marks.tone(Area::ControlPlane), None, "{case}");
        if case != "unreported" {
            assert_eq!(marks.tone(Area::Nodes), None, "{case}");
        }
    }
}

#[gpui_kit::test]
fn a_stale_talos_snapshot_shows_its_cards_as_last_known(cx: &mut TestAppContext) {
    use crate::{desktop::Area, ui::Tone};
    const TALOS_CARDS: [&str; 3] = ["tile-etcd", "tile-services", "tile-memory"];
    let (_runtime, handle, pilot) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let tone = |pilot: &crate::desktop::Pilot, id: &str| {
            let card = pilot
                .overview_display
                .cards
                .iter()
                .find(|card| card.id == id)
                .unwrap();
            (card.tone, card.detail.to_string())
        };
        // The example cluster has an unhealthy service, so Control plane has
        // a dot, and a healthy etcd.
        let before: Vec<_> = TALOS_CARDS
            .iter()
            .map(|id| tone(pilot.read(cx), id))
            .collect();
        assert_eq!(tone(pilot.read(cx), "tile-services").0, Tone::Warn);
        assert_eq!(tone(pilot.read(cx), "tile-etcd").0, Tone::Good);
        assert_eq!(
            pilot.read(cx).rail_marks.tone(Area::ControlPlane),
            Some(Tone::Warn)
        );
        window.click("fixture-fail", cx);
        window.render_frame(cx);
        for id in TALOS_CARDS {
            let (tone, detail) = tone(pilot.read(cx), id);
            assert_eq!(tone, Tone::Unknown, "{id}");
            assert!(detail.starts_with("Last known · "), "{id}: {detail}");
            // The header's stale mark says so too.
            assert!(window.find(format!("{id}-stale")).visible(), "{id}");
        }
        assert_eq!(pilot.read(cx).rail_marks.tone(Area::ControlPlane), None);
        // A refresh that answers brings back the good cards, the warning and its dot.
        window.click("refresh", cx);
        window.render_frame(cx);
        let after: Vec<_> = TALOS_CARDS
            .iter()
            .map(|id| tone(pilot.read(cx), id))
            .collect();
        assert_eq!(after, before);
        assert_eq!(
            pilot.read(cx).rail_marks.tone(Area::ControlPlane),
            Some(Tone::Warn)
        );
    })
    .unwrap();
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

/// The scrolled page's elements of `list`, top to bottom: its group headers'
/// and rows' ids.
fn attention_lines(window: &gpui_kit::Window, list: &str) -> Vec<gpui_kit::ElementId> {
    use gpui_kit::base::test_support::snapshots;
    let list = gpui_kit::ElementId::from(gpui_kit::SharedString::from(list.to_owned()));
    let mut lines: Vec<_> = snapshots(window)
        .into_iter()
        .filter(|line| {
            matches!(
                line.role(),
                Some(gpui_kit::Role::ListBoxOption | gpui_kit::Role::Heading)
            ) && line.path().contains(&list)
        })
        .collect();
    lines.sort_by(|a, b| f32::from(a.bounds().top()).total_cmp(&f32::from(b.bounds().top())));
    lines
        .into_iter()
        .map(|line| line.path().last().unwrap().clone())
        .collect()
}

#[gpui_kit::test]
fn overview_uses_the_shared_frame_with_its_state_in_the_meta_line(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, PageFrame};
    for (width, height, text) in [(1280., 880., 13.), (760., 560., 20.)] {
        let (_runtime, handle, pilot) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            window.click("nav-overview", cx);
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            let context = pilot.read(cx).applied.context.clone().unwrap();
            layout_check::assert_page_frame(
                window,
                cx,
                &PageFrame {
                    page: "overview-page",
                    title: "overview-title",
                    title_text: Box::leak(context.into_boxed_str()),
                    content: "overview-cards",
                },
            );
            for id in ["overview-connection", "roster", "overview-runs"] {
                window.within("overview-scope").find(id);
            }
            assert!(
                window.find("overview-scope").bounds().right()
                    <= window.find("overview-cards").bounds().right() + gpui_kit::px(0.5),
                "the meta line runs past the page at {width}/{text}"
            );
            assert_eq!(
                window.find("roster").label().map(str::to_owned),
                Some(pilot.read(cx).overview_display.roster_tip.to_string())
            );
            assert_actions_inside_rows(window, width, text);
        })
        .unwrap();
    }
}

/// Every row's Open, Logs and Open node buttons sit inside Needs attention,
/// however narrow the page.
fn assert_actions_inside_rows(window: &gpui_kit::Window, width: f32, text: f32) {
    use gpui_kit::base::test_support::snapshots;
    let list = window.find("needs-attention-rows").bounds();
    let actions: Vec<_> = snapshots(window)
        .into_iter()
        .filter(|line| {
            line.path().last().is_some_and(|id| {
                ["attention-open", "attention-logs", "attention-open-node"]
                    .iter()
                    .any(|action| *id == gpui_kit::ElementId::from(*action))
            })
        })
        .collect();
    assert!(
        actions.iter().any(|line| *line.path().last().unwrap()
            == gpui_kit::ElementId::from("attention-logs")),
        "no Logs button at {width}/{text}"
    );
    for action in actions {
        assert!(
            action.bounds().right() <= list.right() + gpui_kit::px(0.5),
            "{:?} runs past Needs attention at {width}/{text}",
            action.path().last()
        );
    }
}

#[gpui_kit::test]
fn needs_attention_groups_compact_rows_by_severity(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, Density, Table};
    use crate::presentation::attention::AttentionGroup;
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1600.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        pilot.update(cx, |pilot, _| pilot.attention_expanded = true);
        window.render_frame(cx);
        let rows = pilot.read(cx).attention.rows.clone();
        let mut expected = Vec::new();
        for row in &rows {
            let header = gpui_kit::ElementId::from(gpui_kit::SharedString::from(row.group.id()));
            if !expected.contains(&header) {
                expected.push(header);
            }
            expected.push(row.id.clone().into());
        }
        assert!(expected.len() > rows.len());
        assert_eq!(attention_lines(window, "needs-attention-rows"), expected);
        let failing = window.find(AttentionGroup::Failing.id());
        let details = pilot.read(cx).attention.details.clone();
        assert!(
            failing
                .label()
                .unwrap()
                .contains(details[AttentionGroup::Failing.index()].as_ref())
        );
        layout_check::assert_table(
            window,
            cx,
            &Table {
                table: None,
                list: "needs-attention-rows",
                density: Density::Compact,
            },
        );
        window.find("needs-attention-title");
        assert!(window.try_find("needs-attention-stale").is_none());
        assert!(window.try_find("overview-collapsed").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn needs_attention_shows_eight_then_fifty_and_says_how_many_are_left(cx: &mut TestAppContext) {
    use crate::presentation::attention;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 1600.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        let template = pilot.read(cx).node_workspace.rows[0].clone();
        let many: Vec<_> = (0..80)
            .map(|ix| {
                let mut row = template.clone();
                row.name = format!("node-{ix:02}").into();
                row.talos.as_mut().unwrap().responding = false;
                row
            })
            .collect();
        pilot.update(cx, |pilot, cx| {
            pilot.attention = attention::build(&many, None, None, chrono::Utc::now());
            cx.notify();
        });
        window.render_frame(cx);
        let shown = |window: &mut gpui_kit::Window| {
            attention_lines(window, "needs-attention-rows")
                .iter()
                .filter(|id| id.to_string().contains("attention-node-"))
                .count()
        };
        assert_eq!(shown(window), 8);
        window
            .within("overview-collapsed")
            .find("attention-show-all");
        window.click("attention-show-all", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).attention_expanded);
        assert_eq!(shown(window), 50);
        // Past the cap the bar stays, without an action, to explain the rest.
        window.find("overview-collapsed");
        assert!(window.try_find("attention-show-all").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_node_pane_shows_all_its_attention_rows_grouped(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, Density, Table};
    use crate::presentation::attention::{self, AttentionGroup};
    let (_runtime, handle, pilot) = fixture(cx, 1600., 1600.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let mut rows = pilot.read(cx).node_workspace.rows.as_ref().clone();
        let node = rows
            .iter_mut()
            .find(|row| row.name == "talos-wk-fra1-02")
            .unwrap();
        let talos = node.talos.as_mut().unwrap();
        talos.responding = false;
        let unhealthy = talos.unhealthy_services().next().unwrap().clone();
        for ix in 0..12 {
            let mut service = unhealthy.clone();
            service.id = format!("extra-{ix:02}");
            talos.services.push(service);
        }
        let built = attention::build(&rows, None, None, chrono::Utc::now());
        let mine = built.by_node["talos-wk-fra1-02"].clone();
        assert!(mine.len() > 8);
        window.click("node-talos-wk-fra1-02", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.attention = built;
            cx.notify();
        });
        window.render_frame(cx);
        let lines = attention_lines(window, "needs-attention-rows");
        for row in &mine {
            assert!(
                lines.contains(&row.id.clone().into()),
                "{} is missing",
                row.id
            );
        }
        for group in [AttentionGroup::Failing, AttentionGroup::Warning] {
            window.within("node-overview").find(group.id());
        }
        assert!(window.try_find("overview-collapsed").is_none());
        layout_check::assert_table(
            window,
            cx,
            &Table {
                table: None,
                list: "needs-attention-rows",
                density: Density::Compact,
            },
        );
    })
    .unwrap();
}
