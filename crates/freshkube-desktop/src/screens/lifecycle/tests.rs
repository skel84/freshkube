use std::sync::Arc;

use freshkube_core::security_lifecycle::SourceSnapshot;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    Drift, Item, LifecycleScreen, LifecycleView, ScreenPanel, ScreenSource, alert_rows,
    etcd_verdict, example, node_rows, presence,
};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::tests::fixture as app;
use crate::ui::Tone;
use crate::{fixture, presentation};
use freshkube_core::HealthIndicator;
use freshkube_core::indicators::QuorumState;
use freshkube_core::security_lifecycle::EtcdPreOperationAudit;
use freshkube_ui::status;

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

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<LifecycleScreen>, WindowHandle<Root>) {
    mount_sized(cx, node, 1100.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    node: &str,
    width: f32,
) -> (Runtime, Entity<LifecycleScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(size(px(width), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = LifecycleScreen::new(runtime.handle().clone(), window, cx);
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

/// A node row's id, from the node's name.
fn node(name: &str) -> gpui_kit::SharedString {
    format!("lifecycle-node-{name}").into()
}

fn example_view() -> LifecycleView {
    example(&source("talos-cp-fra1-01")).unwrap()
}

#[gpui_kit::test]
fn keyboard_selection_moves_through_nodes_and_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        // The details open with a selection.
        assert!(window.try_find("lifecycle-detail").is_none());
        window.click(node("talos-cp-fra1-01"), cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(node("talos-cp-fra1-02")).selected(), Some(true));
        assert_eq!(
            window.find(node("talos-cp-fra1-01")).selected(),
            Some(false)
        );
        let details = window
            .find("lifecycle-detail-title")
            .label()
            .unwrap()
            .to_owned();
        assert!(details.contains("talos-cp-fra1-02"), "{details}");
        // The alert follows the last node; End reaches it.
        window.press("end", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("lifecycle-alert", 0usize)).selected(),
            Some(true)
        );
        let details = window
            .find("lifecycle-detail-title")
            .label()
            .unwrap()
            .to_owned();
        assert!(details.contains("skew"), "{details}");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).selected.is_none());
        assert!(window.try_find("lifecycle-detail").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn details_sit_beside_the_roster_only_when_it_fits_whole(cx: &mut TestAppContext) {
    for (width, beside) in [(1700., true), (1300., false)] {
        let (_runtime, _screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(node("talos-cp-fra1-01"), cx);
            window.render_frame(cx);
            let scroll = window.find("lifecycle-table-scroll").bounds();
            let row = window.find(node("talos-cp-fra1-01")).bounds();
            let details = window.find("lifecycle-detail").bounds();
            if beside {
                assert!(details.left() >= scroll.right(), "{width}: {details:?}");
                assert!(row.right() <= scroll.right(), "{width}: {row:?} {scroll:?}");
            } else {
                assert!(details.top() >= scroll.bottom(), "{width}: {details:?}");
            }
        })
        .unwrap();
    }
}

/// DESIGN.md's table at both text sizes, and no clipped column: the roster
/// scrolls sideways across the page, its name stays at the left edge, and
/// the last column can be brought fully into view, at the two sizes #138
/// found it cut off.
#[gpui_kit::test]
fn the_roster_is_the_shared_table_and_its_last_column_is_reachable(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{Table, assert_table};
    use gpui_kit::{ScrollDelta, point};
    let table = Table {
        table: Some("lifecycle-table-scroll"),
        list: "lifecycle-list",
    };
    for (width, text_size) in [(1280., 14.), (760., 20.)] {
        let (_runtime, _screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            crate::text_size::set(text_size, cx);
            window.render_frame(cx);
            let rows = assert_table(window, cx, &table);
            assert!(rows.header.is_some(), "{width}/{text_size}: {rows:#?}");
            let view = window.find("lifecycle-table-scroll").bounds();
            let name = window.find(("lifecycle-sort", 0usize)).bounds().left();
            window.scroll(
                "lifecycle-table-scroll",
                ScrollDelta::Pixels(point(px(-10_000.), px(0.))),
                cx,
            );
            window.render_frame(cx);
            let last = window.find(("lifecycle-sort", 5usize)).bounds();
            assert!(
                last.right() <= view.right() + px(1.5) && last.left() >= view.left(),
                "{width}/{text_size}: the last column {last:?} is outside its table {view:?}"
            );
            let kept = window.find(("lifecycle-sort", 0usize)).bounds().left();
            assert!(
                (kept - name).abs() <= px(1.5),
                "{width}/{text_size}: the name moved from {name:?} to {kept:?}"
            );
        })
        .unwrap();
    }
}

#[test]
fn too_few_answers_is_unconfirmed_and_not_safe_rather_than_lost() {
    let verdict = etcd_verdict(&SourceSnapshot::Available(EtcdPreOperationAudit {
        total_members: 3,
        responding_members: 1,
        quorum_required: 2,
        can_lose: 0,
        quorum: QuorumState::NoQuorum {
            healthy: 1,
            total: 3,
        },
    }));
    assert!(matches!(verdict.tone, Tone::Warn));
    assert_eq!(verdict.label, "Quorum unconfirmed");
    assert!(
        verdict.detail.contains("1/3 members responding"),
        "{}",
        verdict.detail
    );
}

#[gpui_kit::test]
fn kubelet_skew_alert_is_shown(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let label = window
            .find(("lifecycle-alert", 0usize))
            .label()
            .unwrap()
            .to_owned();
        assert!(label.contains("Kubelet patch version skew"), "{label}");
        assert!(label.contains("talos-wk-fra1-02"), "{label}");
        // The skewed worker's row marks its kubelet as behind.
        let row = window
            .find(node("talos-wk-fra1-02"))
            .label()
            .unwrap()
            .to_owned();
        assert!(row.contains("kubelet v1.34.1"), "{row}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_is_not_reported_rather_than_failed(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let row = window
            .find(node("talos-wk-fra1-03"))
            .label()
            .unwrap()
            .to_owned();
        assert!(row.starts_with("talos-wk-fra1-03"), "{row}");
        assert!(row.contains("Talos not reported"), "{row}");
        assert!(row.contains("config not reported"), "{row}");
        for word in ["down", "failed", "unhealthy"] {
            assert!(!row.contains(word), "{row}");
        }
        let view = screen.read(cx).loader.data().unwrap().clone();
        let rows = node_rows(&view);
        assert!(!rows[5].reported());
        assert_eq!(rows[5].drift, Drift::Unknown);
        // Not reporting is named in the notice, and raises no alert.
        let notice = window.find("partial-notice").label().unwrap().to_owned();
        assert!(notice.contains("talos-wk-fra1-03"), "{notice}");
        let alerts = alert_rows(&view, &rows);
        assert!(
            alerts
                .iter()
                .all(|alert| !alert.message.contains("wk-fra1-03"))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn unavailable_source_is_named_in_the_partial_notice(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let mut view = example_view();
            view.snapshot.talos_discovery = SourceSnapshot::Unavailable {
                reason: "Talos discovery is unavailable: example".into(),
            };
            view.kubelets = SourceSnapshot::Unavailable {
                reason: "kubeconfig example".into(),
            };
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.resolve(target, Ok(view));
            cx.notify();
        });
        window.render_frame(cx);
        let notice = window.find("partial-notice").label().unwrap().to_owned();
        assert!(notice.contains("Talos discovery"), "{notice}");
        assert!(notice.contains("Kubelet versions"), "{notice}");
        // Unknown sources raise no skew or roster alert.
        let view = screen.read(cx).loader.data().unwrap().clone();
        let rows = node_rows(&view);
        assert!(rows.iter().all(|row| row.in_discovery.is_none()));
        assert!(alert_rows(&view, &rows).is_empty());
        let row = window
            .find(node("talos-cp-fra1-01"))
            .label()
            .unwrap()
            .to_owned();
        assert!(row.contains("unknown (discovery wasn't read)"), "{row}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some(Item::Node("talos-cp-fra1-01".into()));
            // The same target keeps its data.
            screen.set_source(Some(source("talos-cp-fra1-01")), window, cx);
            assert!(screen.loader.data().is_some());
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
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
    })
    .unwrap();
}

#[test]
fn drift_needs_two_observed_hashes() {
    let mut view = example_view();
    // Only one hash was read: no drift can be claimed.
    for node in view.snapshot.nodes.iter_mut().skip(1) {
        node.config_hash = SourceSnapshot::Unavailable {
            reason: "not read".into(),
        };
    }
    let rows = node_rows(&view);
    assert_eq!(rows[0].drift, Drift::OnlyReading);
    assert!(rows.iter().skip(1).all(|row| row.drift == Drift::Unknown));
    // Two different hashes read: the odd one out differs.
    view.snapshot.nodes[1].config_hash = SourceSnapshot::Available("43".into());
    view.snapshot.nodes[2].config_hash = SourceSnapshot::Available("42".into());
    let rows = node_rows(&view);
    assert_eq!(rows[0].drift, Drift::InSync);
    assert_eq!(rows[1].drift, Drift::Differs);
}

#[gpui_kit::test]
fn shared_nodes_update_both_lifecycle_views_and_incomplete_rosters_prove_no_absence(
    cx: &mut TestAppContext,
) {
    use freshkube_core::kubernetes_summary::{Session, SessionIdentity, Source, SubscriptionKey};
    use k8s_openapi::api::core::v1::Node;
    use kube::runtime::watcher::Event;
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    let session = Session::new(SessionIdentity::new("fixture-lifecycle", 1));
    let mut node: Node = serde_json::from_value(serde_json::json!({
        "metadata":{"name":"talos-cp-fra1-01","uid":"node-uid","resourceVersion":"1"},
        "status":{"nodeInfo":{"kubeletVersion":"v1.32.3"},"addresses":[{"type":"InternalIP","address":"10.0.0.1"}]}
    })).unwrap();
    session
        .apply::<Node>(0, Event::Init, chrono::Utc::now())
        .unwrap();
    session
        .apply(0, Event::InitApply(node.clone()), chrono::Utc::now())
        .unwrap();
    session
        .apply::<Node>(0, Event::InitDone, chrono::Utc::now())
        .unwrap();
    let subscription = session
        .subscribe(SubscriptionKey::summary(
            session.identity().clone(),
            Source::Nodes,
        ))
        .unwrap();
    for version in ["v1.32.3", "v1.33.1"] {
        let delayed = (version == "v1.33.1").then(|| {
            cx.update(|cx| {
                screen.update(cx, |screen, _| {
                    let old = screen.loader.data().unwrap().clone();
                    let request = screen
                        .loader
                        .state
                        .begin(screen.source.as_ref().unwrap().target.clone());
                    (request, old)
                })
            })
        });
        node.status
            .as_mut()
            .unwrap()
            .node_info
            .as_mut()
            .unwrap()
            .kubelet_version = version.into();
        node.metadata.resource_version = Some(version.into());
        session
            .apply(0, Event::Apply(node.clone()), chrono::Utc::now())
            .unwrap();
        let publication = session.derive(chrono::Utc::now());
        session.publish(publication.clone());
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.observe_nodes(Some(subscription.clone()), cx)
            });
            let data = screen.read(cx).loader.data().unwrap();
            assert_eq!(screen.read(cx).loader.is_loading(), delayed.is_some());
            assert_eq!(
                data.snapshot.kubernetes_roster,
                publication.summary.node_roster()
            );
            assert_eq!(data.kubelets.value().unwrap()[0].version, version);
            assert!(Arc::ptr_eq(
                &screen
                    .read(cx)
                    .summary_nodes
                    .as_ref()
                    .unwrap()
                    .latest()
                    .unwrap(),
                &publication
            ));
            window.render_frame(cx);
            window.find("lifecycle-list");
        })
        .unwrap();
        if let Some((request, old)) = delayed {
            cx.update(|cx| {
                screen.update(cx, |screen, cx| {
                    assert!(screen.loader.state.apply(&request, Ok(old)));
                    cx.notify();
                })
            });
            cx.run_until_parked();
            cx.update(|cx| {
                let data = screen.read(cx).loader.data().unwrap();
                assert_eq!(data.kubelets.value().unwrap()[0].version, version);
                assert_eq!(
                    data.snapshot.kubernetes_roster,
                    publication.summary.node_roster()
                );
            });
        }
    }
    let timestamps = cx.update(|cx| {
        screen.update(cx, |screen, _| {
            let request = screen
                .loader
                .state
                .begin(screen.source.as_ref().unwrap().target.clone());
            assert!(
                screen
                    .loader
                    .state
                    .apply(&request, Err("Talos unavailable".into()))
            );
            (
                screen.loader.last_successful(),
                screen.loader.last_failure(),
            )
        })
    });
    session
        .apply::<Node>(0, Event::Init, chrono::Utc::now())
        .unwrap();
    session.publish(session.derive(chrono::Utc::now()));
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.observe_nodes(Some(subscription), cx)
        });
        let data = screen.read(cx).loader.data().unwrap();
        assert_eq!(screen.read(cx).loader.error(), Some("Talos unavailable"));
        assert_eq!(
            (
                screen.read(cx).loader.last_successful(),
                screen.read(cx).loader.last_failure()
            ),
            timestamps
        );
        assert!(!data.snapshot.kubernetes_roster.is_available());
        assert!(
            data.display
                .rows
                .iter()
                .all(|row| row.in_kubernetes.is_none())
        );
        assert!(
            !data
                .display
                .alerts
                .iter()
                .any(|alert| alert.message.contains("not registered in Kubernetes"))
        );
        assert_eq!(data.kubelets.value().unwrap()[0].version, "v1.33.1");
        window.render_frame(cx);
        screen.update(cx, |screen, cx| screen.observe_nodes(None, cx));
        assert!(screen.read(cx).summary_nodes.is_none());
    })
    .unwrap();
}

#[test]
fn partial_kubelets_keep_row_skew_but_suppress_cluster_alerts() {
    let mut view = example_view();
    let known = view.kubelets.value().unwrap().clone();
    view.kubelets = SourceSnapshot::Partial {
        value: known.clone(),
        warnings: vec!["Refreshing Nodes; showing last known versions".into()],
    };
    let rows = node_rows(&view);
    assert!(rows.iter().any(|row| row.kubelet_behind));
    assert!(alert_rows(&view, &rows).is_empty());

    view.kubelets = SourceSnapshot::Available(known);
    let rows = node_rows(&view);
    assert!(
        alert_rows(&view, &rows)
            .iter()
            .any(|alert| alert.message.starts_with("Kubelet patch version skew:"))
    );

    for source in [
        SourceSnapshot::Unavailable {
            reason: "Nodes refused".into(),
        },
        SourceSnapshot::Available(Vec::new()),
    ] {
        view.kubelets = source;
        let rows = node_rows(&view);
        assert!(rows.iter().all(|row| !row.kubelet_behind));
        assert!(alert_rows(&view, &rows).is_empty());
    }
}

#[test]
fn support_alert_uses_first_reported_talos_instead_of_newest_or_majority() {
    let mut view = example_view();
    for node in &mut view.snapshot.nodes {
        node.version = SourceSnapshot::Available("v1.12.0".into());
    }
    view.snapshot.nodes[0].version = SourceSnapshot::Available("v1.6.0".into());
    let rows = node_rows(&view);
    let support = alert_rows(&view, &rows)
        .into_iter()
        .find(|alert| alert.message.contains("outside the Kubernetes range"))
        .unwrap();
    assert!(
        support
            .message
            .contains("Talos v1.6.0 supports (v1.24 – v1.29)")
    );
    assert_eq!(support.nodes.len(), rows.len());
    assert_eq!(support.evidence.len(), rows.len());

    // An unlisted (or malformed) first version must not fall through to a
    // later reported version that happens to have a known support range.
    for version in ["v1.13.2", "garbage"] {
        view.snapshot.nodes[0].version = SourceSnapshot::Available(version.into());
        let rows = node_rows(&view);
        assert!(
            !alert_rows(&view, &rows)
                .iter()
                .any(|alert| alert.message.contains("outside the Kubernetes range"))
        );
    }
    view.snapshot.nodes[0].version = SourceSnapshot::Unavailable {
        reason: "Talos did not answer".into(),
    };
    let rows = node_rows(&view);
    assert!(
        !alert_rows(&view, &rows)
            .iter()
            .any(|alert| alert.message.contains("outside the Kubernetes range"))
    );
}

#[test]
fn minor_and_support_alerts_keep_wording_node_order_and_evidence() {
    let mut view = example_view();
    let mut kubelets = view.kubelets.value().unwrap().clone();
    for entry in &mut kubelets {
        entry.version = "garbage".into();
    }
    kubelets[0].version = "v1.29.9".into();
    kubelets[1].version = "v1.36.0+k3s1".into();
    let names = [kubelets[0].name.clone(), kubelets[1].name.clone()];
    view.kubelets = SourceSnapshot::Available(kubelets);
    view.snapshot.nodes[0].version = SourceSnapshot::Available("v1.12.0".into());
    let rows = node_rows(&view);
    assert!(rows[0].kubelet_behind);
    assert!(!rows[1].kubelet_behind);
    assert!(rows.iter().skip(2).all(|row| !row.kubelet_behind));
    let alerts = alert_rows(&view, &rows);
    assert_eq!(alerts.len(), 2);
    assert_eq!(
        alerts[0].message,
        format!(
            "Kubelet minor version skew: {} behind v1.36.0+k3s1",
            names[0]
        )
    );
    assert_eq!(alerts[0].nodes, [names[0].clone()]);
    let evidence = vec![
        (names[0].clone(), "v1.29.9".into()),
        (names[1].clone(), "v1.36.0+k3s1".into()),
    ];
    assert_eq!(alerts[0].evidence, evidence);
    assert_eq!(alerts[1].evidence, evidence);
    assert_eq!(alerts[1].nodes, names);
    assert_eq!(
        alerts[1].message,
        format!(
            "Kubelet on {} and {} is outside the Kubernetes range Talos v1.12.0 supports (v1.30 – v1.35)",
            names[0], names[1]
        )
    );
    assert!(
        alerts
            .iter()
            .all(|alert| alert.health == freshkube_core::HealthIndicator::Warning)
    );
}

/// A node missing from a roster is worth a look, not a failure, and a roster
/// that wasn't read says nothing either way.
#[test]
fn roster_presence_glyphs_never_show_a_failure() {
    assert_eq!(presence(Some(true)), Tone::Good);
    assert_eq!(presence(Some(false)), Tone::Warn);
    assert_eq!(presence(None), Tone::Unknown);
}

const LIFECYCLE_PAGE: layout_check::TablePage = layout_check::TablePage {
    page: "lifecycle-page",
    title: "lifecycle-title",
    title_text: "Lifecycle",
    table: "lifecycle-table-scroll",
    list: "lifecycle-list",
};

/// With a node selected, the split of roster and details runs edge to edge
/// in the table's place.
const LIFECYCLE_SPLIT: layout_check::PageFrame = layout_check::PageFrame {
    page: "lifecycle-page",
    title: "lifecycle-title",
    title_text: "Lifecycle",
    content: "lifecycle-split",
};

const LIFECYCLE_TABLE: layout_check::Table = layout_check::Table {
    table: Some("lifecycle-table-scroll"),
    list: "lifecycle-list",
};

/// Lifecycle is a table page, as Pods is: the toolbar header and the bare
/// roster edge to edge, at both text sizes, with or without the details.
/// Its versions, etcd verdict and alerts are the status bar's segment, not
/// a row of stats.
#[gpui_kit::test]
fn lifecycle_is_a_table_page_with_its_status_in_the_bar(cx: &mut TestAppContext) {
    for text in [None, Some(20.)] {
        let (_runtime, handle, _view) = app(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            window.press("secondary-9", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_table_page(window, cx, &LIFECYCLE_PAGE);
            window.find("lifecycle-refresh");
            let scope = window.find("lifecycle-scope");
            assert!(
                scope.path().contains(&ElementId::from("status-bar")),
                "the segment is in the status bar"
            );
            let line = scope.label().unwrap_or_default().to_owned();
            for part in [
                "example data",
                "updated ",
                "Talos v",
                "kubelet v",
                "etcd pre-check",
            ] {
                assert!(line.contains(part), "{part} in {line}");
            }
            // No row of stats; the alerts are a section under the table,
            // which `kubelet_skew_alert_is_shown` finds.
            assert!(window.try_find("lifecycle-summary").is_none());
            let table = window.find("lifecycle-table-scroll").bounds();
            let alerts = window.find("lifecycle-alerts").bounds();
            assert!(alerts.top() >= table.bottom(), "{alerts:?} {table:?}");
            window.click(node("talos-cp-fra1-01"), cx);
            window.render_frame(cx);
            window.find("lifecycle-detail");
            layout_check::assert_edge_frame(window, cx, &LIFECYCLE_SPLIT);
            layout_check::assert_table(window, cx, &LIFECYCLE_TABLE);
            layout_check::assert_bare(window, "lifecycle-table-scroll");
        })
        .unwrap();
    }
}

/// Until the first answer the roster shows the shared loading rows under
/// its header, not a state in its place, and they give way to the rows.
#[gpui_kit::test]
fn the_loading_rows_give_way_to_the_roster_when_it_answers(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        let target = screen.update(cx, |screen, cx| {
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.loader.reset();
            screen.loader.state.begin(target.clone());
            cx.notify();
            target
        });
        window.render_frame(cx);
        assert!(window.find("lifecycle-loading").visible());
        window.find(("lifecycle-sort", 0usize));
        assert!(window.try_find("lifecycle-state").is_none());
        assert!(window.try_find("lifecycle-empty").is_none());
        assert!(window.try_find("lifecycle-alerts").is_none());
        screen.update(cx, |screen, cx| {
            screen.resolve(target, Ok(example_view()));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("lifecycle-loading").is_none());
        window.find(node("talos-cp-fra1-01"));
        window.find("lifecycle-alerts");
    })
    .unwrap();
}

/// A roster with no nodes says so in the table, under its header, and the
/// rest of the page still shows what was read.
#[gpui_kit::test]
fn an_empty_roster_says_so_in_the_table(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let mut view = example_view();
            view.snapshot.nodes.clear();
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.resolve(target, Ok(view));
            cx.notify();
        });
        window.render_frame(cx);
        let empty = window.find("lifecycle-empty");
        assert!(empty.visible());
        let header = window.find(("lifecycle-sort", 0usize)).bounds();
        assert!(empty.bounds().top() >= header.bottom());
        assert!(window.try_find("lifecycle-loading").is_none());
        window.find("lifecycle-etcd");
        window.find("lifecycle-sources");
    })
    .unwrap();
}

/// A failed first read shows the failure in the table's place, with Retry;
/// a failed refresh keeps the roster under a banner in the inset above it.
#[gpui_kit::test]
fn a_failure_replaces_the_table_until_data_and_then_sits_above_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        let target = screen.read(cx).source.as_ref().unwrap().target.clone();
        screen.update(cx, |screen, cx| {
            screen.loader.reset();
            screen.resolve(target.clone(), Err("example failure".into()));
            cx.notify();
        });
        window.render_frame(cx);
        let state = window.find("lifecycle-state").bounds();
        let retry = window.find("screen-retry").bounds();
        assert!(state.contains(&retry.center()), "{retry:?} {state:?}");
        assert!(window.try_find("lifecycle-list").is_none());
        assert!(window.try_find("lifecycle-loading").is_none());

        screen.update(cx, |screen, cx| {
            screen.resolve(target.clone(), Ok(example_view()));
            screen.resolve(target.clone(), Err("example failure".into()));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("lifecycle-state").is_none());
        let retry = window.find("screen-retry").bounds();
        let toolbar = window.find("lifecycle-toolbar").bounds();
        let table = window.find("lifecycle-table-scroll").bounds();
        assert!(retry.top() >= toolbar.bottom(), "{retry:?} {toolbar:?}");
        assert!(retry.bottom() <= table.top(), "{retry:?} {table:?}");
        window.find(node("talos-cp-fra1-01"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_state_sits_under_the_header(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    let under_the_header = |window: &mut gpui_kit::Window| {
        let toolbar = window.find("lifecycle-toolbar").bounds();
        window.find("lifecycle-title");
        let state = window.find("lifecycle-state").bounds();
        assert!(state.top() >= toolbar.bottom(), "{state:?} {toolbar:?}");
    };
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("screen-retry");
        under_the_header(window);
        // A target that failed still has its context in the status bar.
        assert!(screen.update(cx, |screen, _| screen.status().is_some()));
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        under_the_header(window);
    })
    .unwrap();
    // With no target there is nothing to report, so the shell draws no
    // segment; `status()` derives it when the shell asks.
    cx.update(|cx| screen.update(cx, |screen, _| assert!(screen.status().is_none())));
}

/// An unsafe etcd verdict and alerts to review are toned as warnings, and
/// the tooltip's note gives the verdict's detail.
#[test]
fn the_status_warns_of_an_unsafe_etcd_and_alerts_to_review() {
    let mut view = example_view();
    view.snapshot.etcd_pre_operation = SourceSnapshot::Available(EtcdPreOperationAudit {
        total_members: 3,
        responding_members: 3,
        quorum_required: 2,
        can_lose: 0,
        quorum: QuorumState::Degraded {
            healthy: 2,
            total: 3,
        },
    });
    let view = view.prepare();
    let status = &view.display.status;
    assert_eq!(
        status[2],
        status::Part::new("etcd pre-check not safe").tone(Tone::Warn)
    );
    assert!(
        view.display
            .status_note
            .starts_with("etcd pre-check: Not safe to take a control plane down. 3/3"),
        "{}",
        view.display.status_note
    );
    let alerts = &view.display.alerts;
    let review = alerts
        .iter()
        .filter(|alert| {
            matches!(
                alert.health,
                HealthIndicator::Warning | HealthIndicator::Error
            )
        })
        .count();
    assert!(review > 0, "the example has alerts to review");
    let plural = if alerts.len() == 1 { "alert" } else { "alerts" };
    let text = if review == alerts.len() {
        format!("{} {plural} to review", alerts.len())
    } else {
        format!("{} {plural}, {review} to review", alerts.len())
    };
    assert_eq!(status[3], status::Part::new(text).tone(Tone::Warn));
}

/// The shared Inspector opens with a selection: beside the roster on a
/// page wide enough for both, under it otherwise, where the roster keeps
/// the height of its rows and the alerts follow the split.
#[gpui_kit::test]
fn the_inspector_opens_with_a_selection_beside_or_under_the_roster(cx: &mut TestAppContext) {
    for (width, beside) in [(1700., true), (1100., false)] {
        let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("lifecycle-detail").is_none());
            window.click(node("talos-cp-fra1-01"), cx);
            layout_check::assert_inspector(
                window,
                cx,
                "lifecycle-split",
                "lifecycle-table-scroll",
                "lifecycle-detail",
                "lifecycle-detail-title",
            );
            let scroll = window.find("lifecycle-table-scroll").bounds();
            let detail = window.find("lifecycle-detail").bounds();
            let rows = screen.read(cx).rows_and_alerts().0;
            let last = window.find(node(&rows.last().unwrap().name));
            assert!(
                last.bounds().bottom() <= scroll.bottom() + px(0.5),
                "{width}: the last row is cut off"
            );
            if beside {
                assert!(detail.left() >= scroll.right() - px(0.5), "{width}");
            } else {
                assert!(detail.top() >= scroll.bottom() - px(0.5), "{width}");
            }
            let split = window.find("lifecycle-split").bounds();
            let alerts = window.find("lifecycle-alerts").bounds();
            assert!(alerts.top() >= split.bottom(), "{alerts:?} {split:?}");
        })
        .unwrap();
    }
}

/// A selected node that leaves the roster keeps the Inspector open under
/// its name and says so.
#[gpui_kit::test]
fn a_selected_node_that_leaves_the_roster_keeps_its_name(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        screen.update(cx, |screen, cx| {
            screen.selected = Some(Item::Node("talos-old-node".into()));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("lifecycle-detail-gone").visible());
        let title = window
            .find("lifecycle-detail-title")
            .label()
            .unwrap()
            .to_owned();
        assert!(title.contains("talos-old-node"), "{title}");
    })
    .unwrap();
}

/// A width dragged to is saved under `lifecycle` in `navigation.json`,
/// and the next page opens its Inspector at it.
#[gpui_kit::test]
fn the_inspector_width_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-lifecycle-inspector-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| cx.set_global(NavigationFile::open(Some(&preferences))));
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1700.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(node("talos-cp-fra1-01"), cx);
        window.render_frame(cx);
        let state = screen.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(400., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(reopened.inspector_width("lifecycle"), Some(400.));
    cx.update(|cx| cx.set_global(reopened));
    let (_runtime, screen, _handle) = mount_sized(cx, "talos-cp-fra1-01", 1700.);
    cx.read(|cx| assert_eq!(screen.read(cx).split.width(), 400.));
    let _ = std::fs::remove_dir_all(&directory);
}

/// A width saved wider than the room beside the roster still opens beside
/// it, the Inspector taking only the rest, so the split never sticks
/// stacked with no handle to drag it back.
#[gpui_kit::test]
fn an_oversized_saved_width_stays_beside_the_roster(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-lifecycle-oversized-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| {
        let file = NavigationFile::open(Some(&preferences));
        file.set_inspector_width("lifecycle", 2000., cx);
        cx.set_global(file);
    });
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1700.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(node("talos-cp-fra1-01"), cx);
        window.render_frame(cx);
        let roster = screen.read(cx).loader.data().unwrap().display.width;
        let scroll = window.find("lifecycle-table-scroll").bounds();
        let detail = window.find("lifecycle-detail").bounds();
        let split = window.find("lifecycle-split").bounds();
        assert!(
            detail.left() >= scroll.right() - px(0.5),
            "stacked: {detail:?}"
        );
        assert!(
            scroll.size.width >= crate::ui::dp_px(roster, window) - px(0.5),
            "the roster scrolls sideways: {scroll:?}, {roster} dp"
        );
        assert!(
            detail.right() <= split.right() + px(0.5),
            "{detail:?} {split:?}"
        );
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
}

/// An alert stays selected by what it is about: a refresh that raises
/// another alert ahead of it keeps the Inspector on the same one.
#[gpui_kit::test]
fn a_selected_alert_survives_a_new_alert_ahead_of_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("lifecycle-alert", 0usize), cx);
        window.render_frame(cx);
        let title = |window: &gpui_kit::Window| {
            (window.find("lifecycle-detail-title").label())
                .unwrap()
                .to_owned()
        };
        assert!(title(window).contains("skew"), "{}", title(window));
        screen.update(cx, |screen, cx| {
            let mut view = example_view();
            view.snapshot
                .alerts
                .push(freshkube_core::security_lifecycle::LifecycleAlert {
                    health: HealthIndicator::Warning,
                    message: "Configuration drift detected across nodes".into(),
                });
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.resolve(target, Ok(view));
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find(("lifecycle-alert", 1usize)).selected(),
            Some(true)
        );
        assert!(title(window).contains("skew"), "{}", title(window));
    })
    .unwrap();
}
