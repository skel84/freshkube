use std::sync::Arc;

use freshkube_core::security_lifecycle::SourceSnapshot;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    Drift, Item, LifecycleScreen, LifecycleView, ScreenPanel, ScreenSource, alert_rows,
    etcd_verdict, example, node_rows, parse_version,
};
use crate::backend::Target;
use crate::ui::Tone;
use crate::{fixture, presentation};
use freshkube_core::indicators::QuorumState;
use freshkube_core::security_lifecycle::EtcdPreOperationAudit;

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

fn example_view() -> LifecycleView {
    example(&source("talos-cp-fra1-01")).unwrap()
}

#[gpui_kit::test]
fn keyboard_selection_moves_through_nodes_and_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        window.find("lifecycle-details");
        window.click(("lifecycle-node", 0usize), cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("lifecycle-node", 1usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find(("lifecycle-node", 0usize)).selected(),
            Some(false)
        );
        let details = window.find("lifecycle-details").label().unwrap().to_owned();
        assert!(details.contains("talos-cp-fra1-02"), "{details}");
        // The alert follows the last node; End reaches it.
        window.press("end", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("lifecycle-alert", 0usize)).selected(),
            Some(true)
        );
        let details = window.find("lifecycle-details").label().unwrap().to_owned();
        assert!(details.contains("skew"), "{details}");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).selected.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn details_sit_beside_the_roster_only_when_it_fits_whole(cx: &mut TestAppContext) {
    for (width, beside) in [(1700., true), (1300., false)] {
        let (_runtime, _screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let scroll = window.find("lifecycle-node-scroll").bounds();
            let row = window.find(("lifecycle-node", 0usize)).bounds();
            let details = window.find("lifecycle-details").bounds();
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
            .find(("lifecycle-node", 4usize))
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
            .find(("lifecycle-node", 5usize))
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
            .find(("lifecycle-node", 0usize))
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

#[test]
fn versions_parse_with_suffixes() {
    assert_eq!(parse_version("v1.34.3"), Some((1, 34, 3)));
    assert_eq!(parse_version("1.13.2-rc1"), Some((1, 13, 2)));
    assert_eq!(parse_version("v1.30.0+k3s1"), Some((1, 30, 0)));
    assert_eq!(parse_version("garbage"), None);
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
                screen.set_summary_nodes(Some(subscription.clone()), cx)
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
            window.find("lifecycle-details");
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
            screen.set_summary_nodes(Some(subscription), cx)
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
        screen.update(cx, |screen, cx| screen.set_summary_nodes(None, cx));
        assert!(screen.read(cx).summary_nodes.is_none());
    })
    .unwrap();
}
