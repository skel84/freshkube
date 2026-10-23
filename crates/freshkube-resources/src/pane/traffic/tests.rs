use super::super::tests::mount;
use super::super::{ClusterAccess, Duration, Failure, FailureKind, example};
use super::Loaded;
use crate::example::traffic::Shape;
use crate::pane::{DetailPane, DetailTarget};
use crate::ui::Tone;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, Window};

fn target(resource: &str, namespace: &str, name: &str) -> DetailTarget {
    let namespace = (!namespace.is_empty()).then_some(namespace);
    let (_, rows) = example::read("prod-fra", resource, namespace, 1_767_312_000).unwrap();
    DetailTarget {
        identity: rows
            .into_iter()
            .find(|row| row.identity.name == name)
            .unwrap()
            .identity,
        kind: example::kind(resource).unwrap(),
    }
}

fn open(pane: &Entity<DetailPane>, target: &DetailTarget, shape: Shape, cx: &mut App) {
    pane.update(cx, |pane, cx| {
        // The same object asked for again keeps what it read.
        pane.close(cx);
        pane.set_services_shape(shape);
        pane.open(
            target.clone(),
            ClusterAccess::EXAMPLE,
            "1",
            Duration::ZERO,
            cx,
        )
    });
}

fn steps(pane: &Entity<DetailPane>, cx: &App) -> Vec<(&'static str, Tone, String)> {
    pane.read(cx)
        .traffic
        .steps
        .iter()
        .map(|step| (step.kind, step.tone, step.summary.to_string()))
        .collect()
}

fn check_groups(pane: &Entity<DetailPane>, window: &mut Window, cx: &mut App) {
    assert!(window.find("traffic-section").visible());
    for group in ["ready", "draining", "not-ready", "unrouted"] {
        let id = format!("traffic-group-{group}");
        assert!(window.find(id).visible(), "{group}");
    }
    let view = &pane.read(cx).traffic;
    assert_eq!(view.source.as_deref(), Some("EndpointSlices"));
    let reasons = view.pods.iter().map(|pod| pod.reason).collect::<Vec<_>>();
    for reason in [
        "Terminating",
        "Not ready",
        "No matching port",
        "Outside the slices read",
    ] {
        assert!(reasons.contains(&reason), "{reasons:?}");
    }
    // Config and status are separate lines.
    let evidence = view.steps.iter().map(|s| s.evidence).collect::<Vec<_>>();
    assert!(window.find("traffic-group-unrouted-hint").visible());
    let unrouted = view.groups.iter().find(|g| g.title == "Unrouted").unwrap();
    assert_eq!(
        unrouted.hint,
        Some("No port on this slice matches a Service port")
    );
    assert_eq!(evidence.first(), Some(&"Config"));
    assert_eq!(evidence.last(), Some(&"Status"));
}

#[gpui_kit::test]
fn a_service_shows_every_group_and_why_pods_are_left_out(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(
            &pane,
            &target("services", "payments", "api"),
            Shape::Mixed,
            cx,
        );
        window.render_frame(cx);
        check_groups(&pane, window, cx);
    })
    .unwrap();
}

/// Opens the example Service in `shape` and returns its steps.
fn shaped(
    pane: &Entity<DetailPane>,
    window: &mut Window,
    shape: Shape,
    cx: &mut App,
) -> Vec<(&'static str, Tone, String)> {
    open(pane, &target("services", "payments", "api"), shape, cx);
    window.render_frame(cx);
    steps(pane, cx)
}

#[gpui_kit::test]
fn a_service_that_is_up_reads_good_and_one_that_publishes_nothing_reads_broken(
    cx: &mut TestAppContext,
) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let last = shaped(&pane, window, Shape::Healthy, cx).pop().unwrap();
        assert_eq!(last.1, Tone::Good);
        assert!(pane.read(cx).traffic.pods.is_empty());
        assert!(window.try_find("traffic-retry").is_none());
        let last = shaped(&pane, window, Shape::Nothing, cx).pop().unwrap();
        assert_eq!(last.1, Tone::Crit);
        assert!(last.2.contains("no endpoints"));
        assert_eq!(pane.read(cx).traffic.pods.len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn draining_endpoints_warn_and_the_legacy_source_is_named(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let last = shaped(&pane, window, Shape::Draining, cx).pop().unwrap();
        assert_eq!(last.1, Tone::Warn);
        assert!(window.find("traffic-group-draining").visible());
        shaped(&pane, window, Shape::Legacy, cx);
        assert_eq!(
            pane.read(cx).traffic.source.as_deref(),
            Some("Endpoints (legacy)")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_selectorless_and_an_alias_service_are_neutral_not_broken(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let last = shaped(&pane, window, Shape::NoSelector, cx).pop().unwrap();
        assert_eq!(last.1, Tone::Outline);
        assert!(window.try_find("traffic-retry").is_none());
        let all = shaped(&pane, window, Shape::ExternalName, cx);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, Tone::Outline);
        assert!(all[0].2.contains("ExternalName"));
        // An alias publishes nothing: no source and no group is drawn.
        assert!(pane.read(cx).traffic.source.is_none());
        assert!(pane.read(cx).traffic.groups.is_empty());
        assert!(window.try_find("traffic-group-ready").is_none());
        assert!(window.try_find("traffic-pods-unknown").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_load_balancer_without_an_address_warns(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let balancer = shaped(&pane, window, Shape::Pending, cx)
            .into_iter()
            .find(|step| step.0 == "Load balancer")
            .unwrap();
        assert_eq!(balancer.1, Tone::Warn);
        assert!(balancer.2.contains("pending"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_read_is_unknown_with_a_retry_never_zero(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(
            &pane,
            &target("services", "payments", "api"),
            Shape::Refused,
            cx,
        );
        window.render_frame(cx);
        let view = &pane.read(cx).traffic;
        assert!(view.groups.is_empty());
        let last = view.steps.last().unwrap();
        assert_eq!(last.tone, Tone::Unknown);
        assert!(last.summary.contains("unknown"));
        assert!(view.pods_note.is_some());
        assert!(window.find("traffic-retry").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_workload_shows_the_chain_down_to_what_its_service_publishes(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let deployment = target("deployments.apps", "payments", "api");
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &deployment, Shape::Healthy, cx);
        window.render_frame(cx);
        let chain = steps(&pane, cx);
        let kinds = chain.iter().map(|step| step.0).collect::<Vec<_>>();
        assert_eq!(kinds, ["Workload", "Service", "Endpoints"]);
        assert_eq!(chain.last().unwrap().1, Tone::Good);
        assert!(window.find("traffic-step-2").visible());

        open(&pane, &deployment, Shape::ServicesRefused, cx);
        window.render_frame(cx);
        let chain = steps(&pane, cx);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].1, Tone::Unknown);
        assert!(window.find("traffic-retry").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn other_kinds_show_no_traffic(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let (_, rows) = example::read("prod-fra", "nodes", None, 1_767_312_000).unwrap();
        let node = DetailTarget {
            identity: rows.into_iter().next().unwrap().identity,
            kind: example::kind("nodes").unwrap(),
        };
        open(&pane, &node, Shape::Mixed, cx);
        window.render_frame(cx);
        assert!(window.try_find("traffic-section").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_old_answer_cannot_overwrite_a_newer_one_or_a_hidden_pane(cx: &mut TestAppContext) {
    let (_runtime, pane, _handle, _) = mount(cx);
    let service = target("services", "payments", "api");
    cx.update(|cx| {
        open(&pane, &service, Shape::Healthy, cx);
        let seq = pane.read(cx).traffic.seq;
        let before = steps(&pane, cx);
        let refused = || Failure::new(FailureKind::Forbidden, "late");
        pane.update(cx, |pane, cx| {
            pane.finish_traffic(
                &service.identity,
                seq.wrapping_sub(1),
                Loaded::Service(Box::new(Err(refused()))),
                ("Service".into(), Default::default()),
                cx,
            )
        });
        assert_eq!(steps(&pane, cx), before);
        // Hidden, a pane reads nothing and takes no answer.
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        let seq = pane.read(cx).traffic.seq;
        pane.update(cx, |pane, cx| {
            pane.finish_traffic(
                &service.identity,
                seq,
                Loaded::Service(Box::new(Err(refused()))),
                ("Service".into(), Default::default()),
                cx,
            )
        });
        assert_eq!(steps(&pane, cx), before);
        assert!(pane.read(cx).traffic.job.is_none());
    });
}

#[gpui_kit::test]
fn the_section_and_its_rows_stay_inside_the_pane(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(
            &pane,
            &target("services", "payments", "api"),
            Shape::Mixed,
            cx,
        );
        window.render_frame(cx);
        let section = window.find("traffic-section").bounds();
        let pane_bounds = window.find("detail-overview").bounds();
        assert!(section.left() >= pane_bounds.left() - gpui_kit::px(1.));
        assert!(section.right() <= pane_bounds.right() + gpui_kit::px(1.));
        for id in [
            "traffic-step-0",
            "traffic-group-ready",
            "traffic-group-unrouted",
        ] {
            let bounds = window.find(id).bounds();
            assert!(bounds.right() <= section.right() + gpui_kit::px(1.), "{id}");
        }
    })
    .unwrap();
}

// -- HTTPRoute (#949, N2a) -------------------------------------------------

mod route {
    use super::*;
    use crate::pane::{DetailEvent, Tab};
    use crate::{ResourceLink, route_trace::TraceLink};
    use freshkube_core::resources::traffic::routes::example::Shape as Routes;

    const KEY: &str = "httproutes.gateway.networking.k8s.io";

    fn route(shape: Routes, name: &str) -> (DetailTarget, Routes) {
        (target(KEY, "shop", name), shape)
    }

    fn open_route(pane: &Entity<DetailPane>, target: &DetailTarget, shape: Routes, cx: &mut App) {
        pane.update(cx, |pane, cx| {
            pane.close(cx);
            pane.set_routes_shape(shape);
            pane.open(
                target.clone(),
                ClusterAccess::EXAMPLE,
                "1",
                Duration::ZERO,
                cx,
            )
        });
    }

    #[gpui_kit::test]
    fn a_route_shows_its_trace_compact_in_overview_and_full_on_its_own_tab(
        cx: &mut TestAppContext,
    ) {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            let (target, shape) = route(Routes::All, "checkout");
            open_route(&pane, &target, shape, cx);
            window.render_frame(cx);
            assert!(window.find("traffic-section").visible());
            let compact = window.find("route-trace-compact");
            assert!(compact.visible());
            assert!(
                window
                    .find("route-trace-verdict")
                    .label()
                    .unwrap()
                    .starts_with("Not serving")
            );
            // The tab carries the route's host and path.
            window.find("detail-tab-trace");
            assert_eq!(pane.read(cx).traffic.route_label(), "checkout.example.com/");
            window.click("detail-tab-trace", cx);
            window.render_frame(cx);
            assert_eq!(pane.read(cx).tab(), Tab::Trace);
            window.find("detail-trace-page");
            window.find("route-trace-full");
            // Each step of the one parent, then the branch's, are drawn.
            for step in 0..4 {
                window.find(format!("route-trace-parent-0-step-{step}"));
            }
            for step in 0..3 {
                window.find(format!("route-trace-parent-0-branch-0-step-{step}"));
            }
            // The refused reference proposes a grant to copy, scoped to the pair.
            window.find("route-trace-grant-0-copy");
            window.find("route-trace-grant-0-yaml");
            let proposals = pane
                .read(cx)
                .traffic
                .route
                .as_ref()
                .map(|view| match view {
                    super::super::RouteView::Trace { model, .. } => {
                        model.proposals().map(str::to_owned).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                })
                .unwrap();
            assert_eq!(proposals.len(), 1);
            assert!(proposals[0].contains("namespace: payments"));
            assert!(proposals[0].contains("namespace: shop"));
            assert!(proposals[0].contains("name: ledger"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn only_a_route_has_a_trace_tab_and_asking_for_it_elsewhere_opens_details(
        cx: &mut TestAppContext,
    ) {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            open(
                &pane,
                &target("services", "payments", "api"),
                Shape::Healthy,
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("detail-tab-trace").is_none());
            pane.update(cx, |pane, cx| pane.set_tab(Tab::Trace, cx));
            window.render_frame(cx);
            assert_eq!(pane.read(cx).tab(), Tab::Overview);
            // From a route's Trace tab to another kind: back to Details.
            let (route, shape) = route(Routes::All, "web");
            open_route(&pane, &route, shape, cx);
            pane.update(cx, |pane, cx| pane.set_tab(Tab::Trace, cx));
            window.render_frame(cx);
            assert_eq!(pane.read(cx).tab(), Tab::Trace);
            open(
                &pane,
                &target("services", "payments", "api"),
                Shape::Healthy,
                cx,
            );
            window.render_frame(cx);
            assert_eq!(pane.read(cx).tab(), Tab::Overview);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_strip_steps_through_details_trace_and_yaml_for_a_route(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            let (route, shape) = route(Routes::All, "web");
            open_route(&pane, &route, shape, cx);
            window.render_frame(cx);
            let mut seen = vec![pane.read(cx).tab()];
            for _ in 0..3 {
                pane.update(cx, |pane, cx| pane.turn_tab(1, cx));
                seen.push(pane.read(cx).tab());
            }
            assert_eq!(seen, [Tab::Overview, Tab::Trace, Tab::Yaml, Tab::Overview]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_traces_links_go_through_the_normal_object_links(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, emitted) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            let (route, shape) = route(Routes::All, "checkout");
            open_route(&pane, &route, shape, cx);
            pane.update(cx, |pane, cx| pane.set_tab(Tab::Trace, cx));
            window.render_frame(cx);
            window.click("route-trace-parent-0-gateway", cx);
            // The Service's button is below the fold of this pane; it is the
            // same link, so ask for it as the button does.
            window.find("route-trace-parent-0-branch-0-service");
            pane.update(cx, |pane, cx| {
                pane.open_trace_link(
                    &TraceLink::Service {
                        namespace: "payments".into(),
                        name: "ledger".into(),
                    },
                    cx,
                )
            });
        })
        .unwrap();
        cx.run_until_parked();
        let links = emitted
            .borrow()
            .iter()
            .filter_map(|event| match event {
                DetailEvent::Link(link) => Some(link.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            matches!(&links[0], ResourceLink::Owner { kind, object, .. }
                if kind == "Gateway" && object.name == "edge" && object.namespace == "infra"),
            "{links:?}"
        );
        assert!(
            matches!(&links[1], ResourceLink::Object(kind, object, Tab::Overview)
                if kind.plural == "services" && object.name == "ledger" && object.namespace == "payments"),
            "{links:?}"
        );
        let _ = TraceLink::Gateway {
            namespace: String::new(),
            name: String::new(),
        };
    }

    #[gpui_kit::test]
    fn a_gateway_api_that_is_not_there_or_a_refused_read_is_plain_never_not_serving(
        cx: &mut TestAppContext,
    ) {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            let (route, _) = route(Routes::All, "web");
            open_route(&pane, &route, Routes::NotInstalled, cx);
            window.render_frame(cx);
            let message = window.find("route-trace-message");
            assert!(message.label().unwrap().contains("not installed"));
            assert!(window.try_find("traffic-retry").is_none());
            open_route(&pane, &route, Routes::Refused, cx);
            window.render_frame(cx);
            let message = window.find("route-trace-message");
            let words = message.label().unwrap();
            assert!(words.starts_with("Trace unknown"), "{words}");
            assert!(!words.contains("Not serving"));
            window.find("traffic-retry");
            // The Trace tab says the same, with the same Retry.
            pane.update(cx, |pane, cx| pane.set_tab(Tab::Trace, cx));
            window.render_frame(cx);
            window.find("detail-trace-page");
            window.find("route-trace-message");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_mixed_bundle_is_noted_beside_the_trace(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            let (route, _) = route(Routes::All, "web");
            open_route(&pane, &route, Routes::MixedBundle, cx);
            window.render_frame(cx);
            let notes = window.find("route-trace-notes");
            assert!(notes.visible());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_hidden_pane_reads_no_trace(cx: &mut TestAppContext) {
        let (_runtime, pane, handle, _) = mount(cx);
        cx.update_window(handle, |_, window, cx| {
            let (route, shape) = route(Routes::All, "web");
            pane.update(cx, |pane, cx| {
                pane.set_active(false, cx);
                pane.set_routes_shape(shape);
                pane.open(
                    route.clone(),
                    ClusterAccess::EXAMPLE,
                    "1",
                    Duration::ZERO,
                    cx,
                )
            });
            window.render_frame(cx);
            assert!(pane.read(cx).traffic.route.is_none());
            pane.update(cx, |pane, cx| pane.set_active(true, cx));
            window.render_frame(cx);
            assert!(pane.read(cx).traffic.route.is_some());
        })
        .unwrap();
    }
}
