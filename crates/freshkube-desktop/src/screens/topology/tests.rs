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
