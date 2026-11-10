use super::{PageSource, ScreenPanel, TopologyScreen};
use crate::desktop::layout_check;
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    resources::traffic::routes::example::Shape,
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};

fn mount(
    cx: &mut TestAppContext,
    shape: Shape,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<TopologyScreen>,
    WindowHandle<Root>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                TopologyScreen::new(runtime.handle().clone(), window, cx).with_example(shape);
            view.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "invented-topology".into(),
                        context: "invented-topology-context".into(),
                        access: ClusterAccess::EXAMPLE,
                    }),
                    ..PageSource::default()
                },
                window,
                cx,
            );
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, page.unwrap(), window)
}

#[gpui_kit::test]
fn the_graph_holds_every_box_and_call_of_the_topology(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("topology-title");
        let page = page.read(cx);
        let topology = page.topology().expect("a topology");
        assert_eq!(page.graph.nodes().len(), topology.nodes.len());
        assert_eq!(page.graph.edges().len(), topology.edges.len());
        assert!(
            page.graph
                .nodes()
                .iter()
                .any(|n| n.key == "workload:Deployment/shop/cart")
        );
        // A box that should be there and is not stays a dashed ghost.
        let ghost = page
            .graph
            .nodes()
            .iter()
            .find(|n| n.key == "service:shop/ghost")
            .unwrap();
        assert!(ghost.ghost);
        window.find("topology-graph");
    })
    .unwrap();
}

#[gpui_kit::test]
fn selecting_a_box_or_a_call_fills_the_inspector_and_opens_the_object(cx: &mut TestAppContext) {
    use crate::resources::ResourceLink;
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, link: &ResourceLink, _| {
            seen.borrow_mut().push(link.clone())
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            use freshkube_ui::graph::GraphSource as _;
            page.open(&"gateway:infra/edge".to_owned(), window, cx);
        });
        window.render_frame(cx);
        window.find("topology-node-detail");
        window.find("topology-open");
        page.update(cx, |page, cx| page.open_selected(cx));
        // A call is selected by its ends.
        page.update(cx, |page, cx| {
            page.graph.select(
                &"route:shop/web".to_owned(),
                &"service:shop/cart".to_owned(),
            );
            cx.notify();
        });
        window.render_frame(cx);
        window.find("topology-edge-detail");
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    assert_eq!(links.len(), 1, "{links:?}");
    match &links[0] {
        ResourceLink::Object(kind, object, _) => {
            assert_eq!(kind.kind, "Gateway");
            assert_eq!(object.name, "edge");
        }
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_ghost_gateway_opens_nothing(cx: &mut TestAppContext) {
    use crate::resources::ResourceLink;
    let (_runtime, page, window) = mount(cx, Shape::Gateways, 1280., 880.);
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, _: &ResourceLink, _| {
            seen.set(seen.get() + 1)
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            use freshkube_ui::graph::GraphSource as _;
            page.open(&"gateway:infra/retired".to_owned(), window, cx);
        });
        window.render_frame(cx);
        window.find("topology-open");
        page.update(cx, |page, cx| page.open_selected(cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 0);
}

#[gpui_kit::test]
fn refused_workloads_draw_unknown_boxes_and_say_so_in_the_status_bar(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::WorkloadsRefused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let page = page.read(cx);
        let ghost = page
            .graph
            .nodes()
            .iter()
            .find(|n| n.key == "workloads:shop/cart")
            .unwrap();
        assert!(ghost.ghost);
        assert!(
            ghost.tooltip.contains("could not be read"),
            "{}",
            ghost.tooltip
        );
    })
    .unwrap();
    cx.update_window(window.into(), |_, _, cx| {
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        assert!(format!("{segment:?}").contains("unknown"), "{segment:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_gateway_list_is_an_error_with_retry_never_an_empty_graph(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::GatewaysRefused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(page.read(cx).graph.nodes().is_empty());
        let state = window.find("topology-state");
        assert!(state.visible());
        window.find("topology-retry");
        assert!(window.try_find("topology-graph").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn not_installed_and_empty_read_plainly(cx: &mut TestAppContext) {
    for shape in [Shape::NotInstalled, Shape::Incomplete, Shape::Empty] {
        let (_runtime, _page, window) = mount(cx, shape, 1280., 880.);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("topology-state").visible(), "{shape:?}");
            assert!(window.try_find("topology-graph").is_none(), "{shape:?}");
            assert!(window.try_find("topology-retry").is_none(), "{shape:?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_notes_of_what_was_left_out_are_banners(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // The mesh route is not drawn, and the page says so.
        let banner = window.find("topology-notice-0");
        assert!(banner.label().is_some_and(|l| l.contains("mesh route")));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_page_follows_the_shared_page_frame_wide_narrow_and_at_large_text(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, _page, window) = mount(cx, Shape::All, width, height);
        for text in [13., 20.] {
            cx.update_window(window.into(), |_, window, cx| {
                crate::text_size::set(text, cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                layout_check::settle_header(window, cx);
                window.render_frame(cx);
                layout_check::assert_page_frame(
                    window,
                    cx,
                    &layout_check::PageFrame {
                        page: "topology-page",
                        title: "topology-title",
                        title_text: "Topology",
                        content: "topology-panel",
                    },
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn a_held_read_keeps_the_loading_state_until_it_answers(cx: &mut TestAppContext) {
    let hold = freshkube_core::cluster_source::Hold {
        routes: true,
        ..Default::default()
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = TopologyScreen::new(runtime.handle().clone(), window, cx);
            view.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "invented-held".into(),
                        context: "invented".into(),
                        access: ClusterAccess::Example(hold),
                    }),
                    ..PageSource::default()
                },
                window,
                cx,
            );
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let page = page.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(page.read(cx).loader.is_loading());
        assert!(window.find("topology-state").visible());
        assert!(window.try_find("topology-retry").is_none());
        assert!(window.try_find("topology-graph").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_page_says_what_it_holds_from_what_was_prepared_with_the_read(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let page = page.read(cx);
        assert!(
            page.summary
                .as_deref()
                .is_some_and(|s| s.contains("Gateways"))
        );
        assert!(page.broken > 0);
        assert!(page.notices.iter().any(|n| n.contains("mesh route")));
    })
    .unwrap();
}

#[test]
fn a_link_carries_the_uid_of_a_live_cluster_and_none_of_the_examples() {
    use freshkube_core::resources::traffic::routes::{Topology, example};
    let read = example::read(Shape::All).unwrap();
    let topology = Topology::new(&read, &example::workloads(Shape::All)).unwrap();
    let node = topology.node("gateway:infra/edge").unwrap();
    assert_eq!(node.target.as_ref().unwrap().uid, "gw-infra-edge");
}

#[gpui_kit::test]
fn a_calls_share_or_listener_is_in_its_own_words_and_no_label_sits_on_a_line(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let page = page.read(cx);
        // Nothing is drawn on a line, where the box before it would cut it.
        assert!(page.graph.edges().iter().all(|e| e.pill.is_none()));
        let split = page
            .graph
            .edges()
            .iter()
            .find(|e| e.from == "route:shop/cart-split" && e.to == "service:shop/canary")
            .unwrap();
        assert!(split.label.ends_with("· 20%"), "{}", split.label);
        let gateway = page
            .graph
            .edges()
            .iter()
            .find(|e| e.from == "gateway:infra/edge" && e.to == "route:shop/web")
            .unwrap();
        assert!(gateway.label.ends_with("· https"), "{}", gateway.label);
    })
    .unwrap();
}

#[test]
fn a_topology_draws_at_most_the_box_cap_and_counts_the_gateways_it_leaves_out() {
    use freshkube_core::resources::traffic::routes::{MAX_BOXES, Topology, example};
    let mut read = example::read(Shape::Gateways).unwrap();
    let template = read.gateways.as_ref().unwrap().items[0].clone();
    let items = &mut read.gateways.as_mut().unwrap().items;
    for n in 0..MAX_BOXES {
        let mut gateway = template.clone();
        gateway.name = format!("bulk{n}");
        gateway.namespace = "bulk".into();
        items.push(gateway);
    }
    let topology = Topology::new(&read, &example::workloads(Shape::Gateways)).unwrap();
    assert!(topology.nodes.len() < MAX_BOXES + 20);
    assert!(
        topology.notes.iter().any(|n| n.contains("not drawn")),
        "{:?}",
        topology.notes
    );
    for call in &topology.edges {
        assert!(topology.node(&call.from).is_some() && topology.node(&call.to).is_some());
    }
}

#[test]
fn one_gateway_with_hundreds_of_routes_still_stays_under_the_box_cap() {
    use freshkube_core::resources::traffic::routes::{
        MAX_BOXES, RouteEntry, RouteFacts, Topology, example, trace_route,
    };
    let _ = trace_route;
    let mut read = example::read(Shape::All).unwrap();
    // Four hundred more routes on `infra/edge`, copied from `web`.
    let template = read
        .routes
        .iter()
        .find(|r| r.facts.name == "web")
        .unwrap()
        .clone();
    for n in 0..400 {
        let mut facts: RouteFacts = template.facts.clone();
        facts.name = format!("bulk{n}");
        read.routes
            .push(RouteEntry::new(facts, template.trace.clone()));
    }
    let topology = Topology::new(&read, &example::workloads(Shape::All)).unwrap();
    // The cap, plus the one route whose boxes were being drawn.
    assert!(
        topology.nodes.len() <= MAX_BOXES + 6,
        "{}",
        topology.nodes.len()
    );
    assert!(
        topology
            .notes
            .iter()
            .any(|n| n.contains("routes are not drawn")),
        "{:?}",
        topology.notes
    );
    for call in &topology.edges {
        assert!(topology.node(&call.from).is_some() && topology.node(&call.to).is_some());
    }
}

#[gpui_kit::test]
fn a_selected_box_lists_its_calls_and_shows_its_detail_without_scrolling(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let all = page.read(cx).graph.listed_count();
        page.update(cx, |page, cx| {
            use freshkube_ui::graph::GraphSource as _;
            page.open(&"gateway:infra/edge".to_owned(), window, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let own = page.read(cx).graph.listed_count();
        assert!(own > 0 && own < all, "{own} of {all} calls");
        let (viewport, detail) = (window.viewport_size(), window.find("topology-node-detail"));
        assert!(
            detail.bounds().bottom() <= viewport.height,
            "the detail sits below the window: {:?} in {viewport:?}",
            detail.bounds()
        );
        window.press("escape", cx);
    })
    .unwrap();
}
