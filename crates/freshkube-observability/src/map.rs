use super::*;
use freshkube_core::coroot::AppId;
use freshkube_ui::graph::{GraphEdge, GraphNode, GraphSource, GraphState, GraphText, GraphView};
use freshkube_ui::inspector::InspectorSplit;

/// The service map's graph state: its page, filter and selection, and what
/// it draws. Clearing the page's observations resets it to an empty map.
pub(super) fn display() -> GraphState<AppId> {
    GraphState::new(
        "obs-map",
        GraphText {
            nouns: ("applications", "connections"),
            title: "Connections".into(),
            problems: "Problem connections".into(),
            caption: "Callers on the left · line width is traffic · dashed lines have a problem · every connection is listed in Connections".into(),
        },
    )
}

impl ObservabilityPage {
    /// Hands the map's applications and connections to its graph, which
    /// lays out the page they land on.
    pub(super) fn prepare_map(&mut self) {
        let nodes = self
            .nodes
            .iter()
            .map(|node| GraphNode {
                key: node.app.clone(),
                id: node.element_id.clone(),
                label: node.label.clone().into(),
                detail: node.namespace.clone().into(),
                tone: tone(node.status),
                tooltip: node.tooltip.clone().into(),
            })
            .collect();
        let edges = self
            .connections
            .iter()
            .map(|edge| GraphEdge {
                from: edge.id.0.clone(),
                to: edge.id.1.clone(),
                marker_id: edge.element_id.clone(),
                row_id: edge.button_id.clone(),
                tone: tone(edge.status),
                width: 0.5 + edge.traffic * 0.6,
                label: edge.label.clone().into(),
                tooltip: edge.tooltip.clone().into(),
            })
            .collect();
        self.map_display.set(nodes, edges);
    }

    pub(super) fn render_map(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        GraphView::new(freshkube_ui::page::content_width(window))
            .page_width(freshkube_ui::page::page_width(window))
            .render(self, window, cx)
    }

    /// The page's connection for a call of the graph.
    fn map_connection(&self, edge: &GraphEdge<AppId>) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|connection| connection.id.0 == edge.from && connection.id.1 == edge.to)
    }
}

impl GraphSource for ObservabilityPage {
    type Key = AppId;

    fn graph(&self) -> &GraphState<AppId> {
        &self.map_display
    }

    fn graph_mut(&mut self) -> &mut GraphState<AppId> {
        &mut self.map_display
    }

    fn graph_split(&self) -> &InspectorSplit {
        &self.map_split
    }

    fn highlighted(&self) -> Option<&AppId> {
        self.selected_app.as_ref()
    }

    fn open(&mut self, node: &AppId, _: &mut Window, cx: &mut Context<Self>) {
        self.open_app(node.clone(), Report::Net, cx)
    }

    fn inspector_head(
        &self,
        selected: Option<&GraphEdge<AppId>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(edge) = selected.and_then(|selected| self.map_connection(selected)) else {
            return muted("Choose a connection to inspect its evidence", cx).into_any_element();
        };
        v_flex()
            .gap(dp(12.))
            .min_w_0()
            .child(
                line()
                    .child(status(edge.status, cx))
                    .child(text(edge.status.label())),
            )
            .child(mono(edge.label.clone()))
            .child(text(edge.detail.clone()))
            .into_any_element()
    }

    /// The selected connection's one action: its caller's report.
    fn inspector_footer(
        &self,
        selected: Option<&GraphEdge<AppId>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let app = self.map_connection(selected?)?.id.0.clone();
        Some(
            action("obs-map-open-app", "Open application")
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_app(app.clone(), Report::Net, cx)),
                )
                .into_any_element(),
        )
    }

    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        text("No service map nodes were returned").into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::Destination;
    use crate::{
        projection,
        tests::{large_map, mount},
    };
    use freshkube_ui::graph::{MARKERS, PER_PAGE};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, ScrollStrategy, TestAppContext};

    #[gpui_kit::test]
    fn every_retained_link_is_reachable_and_selected_endpoints_survive_paging(
        cx: &mut TestAppContext,
    ) {
        let (_runtime, handle, page) = mount(cx, true);
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                (page.nodes, page.connections) = projection::map(&large_map());
                page.prepare_map();
                page.open(Destination::ServiceMap, cx);
                assert_eq!(page.map_display.placed().count(), PER_PAGE);
                assert_eq!(page.map_display.pages(), 5);
                assert_eq!(page.map_display.visible_count(), 300);
                assert!(page.map_display.summary().contains("of 120 applications"));
                assert!(
                    page.map_display
                        .summary()
                        .contains("of 300 filtered connections")
                );
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("obs-map-next", cx);
            assert_eq!(page.read(cx).map_display.page(), 1);
            page.read(cx)
                .map_display
                .reveal(299, ScrollStrategy::Bottom);
            window.render_frame(cx);
            let id = page.read(cx).connections[299].button_id.clone();
            window.click(id, cx);
            let selected = page.read(cx).connections[299].id.clone();
            let ends = (selected.0.clone(), selected.1.clone());
            assert_eq!(page.read(cx).map_display.selected(), Some(&ends));
            window.render_frame(cx);
            window.click("obs-map-next", cx);
            let display = &page.read(cx).map_display;
            assert!(display.placed().any(|(n, ..)| n.key == selected.0));
            assert!(display.placed().any(|(n, ..)| n.key == selected.1));
            assert!(display.placed().count() <= PER_PAGE + 2);
            assert!(display.markers().count() <= MARKERS + 1);
            assert!(
                display
                    .markers()
                    .any(|edge| edge.from == selected.0 && edge.to == selected.1)
            );
            page.update(cx, |page, _| {
                (page.nodes, page.connections) = projection::map(&Default::default());
                page.prepare_map();
                assert!(page.map_display.selected().is_none());
                assert_eq!(page.map_display.page(), 0);
                assert_eq!(page.map_display.placed().count(), 0);
            });
        })
        .unwrap();
    }
    #[gpui_kit::test]
    fn no_connection_runs_through_a_box_on_any_page(cx: &mut TestAppContext) {
        use freshkube_graph::layout::{NODE_H, NODE_W, point_at};
        let (_runtime, _handle, page) = mount(cx, true);
        cx.update(|cx| {
            page.update(cx, |page, _| {
                (page.nodes, page.connections) = projection::map(&large_map());
                page.prepare_map();
                for number in 0..page.map_display.pages() {
                    page.map_display.set_page(number);
                    let display = &page.map_display;
                    let boxes: Vec<_> = display.placed().map(|(_, x, y)| (x, y)).collect();
                    for (edge, route) in display.shown() {
                        for segment in &route.segments {
                            for step in 0..=16 {
                                let (x, y) = point_at(segment, step as f32 / 16.);
                                let inside = boxes.iter().any(|&(bx, by)| {
                                    x > bx + 0.5
                                        && x < bx + NODE_W - 0.5
                                        && y > by + 0.5
                                        && y < by + NODE_H - 0.5
                                });
                                assert!(!inside, "page {number}: {} at {x}, {y}", edge.label);
                            }
                        }
                    }
                }
            })
        });
    }
    #[gpui_kit::test]
    fn narrow_map_scrolls_inside_its_card_at_large_text(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = crate::tests::mount_size(cx, true, 760., 560.);
        cx.update(|cx| {
            crate::text_size::set(20., cx);
            page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx));
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            use gpui_kit::{ScrollDelta, point, px};
            window.render_frame(cx);
            let panel = window.find("obs-map-panel").bounds();
            let viewport = window.find("obs-map-scroll").bounds();
            let before = window.find("obs-map-graph").bounds();
            assert!(panel.right() <= px(760.));
            assert!(viewport.left() - panel.left() >= px(14.));
            assert!(panel.right() - viewport.right() >= px(14.));
            assert!(before.size.width > viewport.size.width);
            // The top-left box is in view before the map scrolls.
            let node = page
                .read(cx)
                .map_display
                .placed()
                .min_by(|a, b| (a.1 + a.2).total_cmp(&(b.1 + b.2)))
                .unwrap()
                .0
                .id
                .clone();
            window.scroll(node, ScrollDelta::Pixels(point(px(-1000.), px(0.))), cx);
            window.render_frame(cx);
            assert!(window.find("obs-map-graph").bounds().left() < before.left());
        })
        .unwrap();
    }

    /// Open application moved into the inspector's footer: it keeps its id
    /// and opens the selected connection's caller.
    #[gpui_kit::test]
    fn the_footer_opens_the_selected_connection_s_caller(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount(cx, true);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("obs-map-open-app").is_none());
            let connection = page.read(cx).connections[0].clone();
            window.click(connection.button_id.clone(), cx);
            window.render_frame(cx);
            assert!(window.try_find("obs-map-inspector-footer").is_some());
            window.click("obs-map-open-app", cx);
            let page = page.read(cx);
            assert_eq!(page.destination, Destination::Application);
            assert_eq!(page.selected_app.as_ref(), Some(&connection.id.0));
        })
        .unwrap();
    }

    /// A width dragged to beside the map is saved under `inspector.map` in
    /// the app's store of sizes, and the next page opens its inspector at it.
    #[gpui_kit::test]
    fn the_map_inspector_width_survives_reopening(cx: &mut TestAppContext) {
        use freshkube_ui::inspector::width_key;
        use freshkube_ui::split_size::{MemorySizes, SizeStore, set_store};
        let widths = std::rc::Rc::new(MemorySizes::default());
        cx.update(|cx| set_store(widths.clone(), cx));
        let open = |cx: &mut TestAppContext| {
            let (runtime, handle, page) = crate::tests::mount_size(cx, true, 1800., 900.);
            cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx)));
            cx.run_until_parked();
            (runtime, handle, page)
        };
        let (_runtime, handle, page) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let (lead, inspector) = (
                window.find("obs-map-lead").bounds(),
                window.find("obs-map-inspector").bounds(),
            );
            assert!(inspector.left() >= lead.right(), "{lead:?} {inspector:?}");
            let state = page.read(cx).map_split.beside_state().clone();
            state.update(cx, |state, cx| {
                state.resize_panel(1, crate::ui::dp_px(400., window), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(widths.size(width_key("map")), Some(400.));
        assert_eq!(widths.size(width_key("incidents")), None);

        let (_runtime, handle, _page) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let width = window.find("obs-map-inspector").bounds().size.width;
            let expected = crate::ui::dp_px(400., window);
            assert!(
                (width - expected).abs() <= gpui_kit::px(1.),
                "{width:?}, expected {expected:?}"
            );
        })
        .unwrap();
    }
}
