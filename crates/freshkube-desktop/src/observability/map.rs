use super::*;
const NODES_PER_PAGE: usize = 24;
const EDGE_MARKERS: usize = 24;

#[derive(Default)]
pub(super) struct MapDisplay {
    nodes: std::rc::Rc<Vec<MapNode>>,
    connections: std::rc::Rc<Vec<Connection>>,
    markers: Vec<usize>,
    summary: String,
    page_label: String,
    connection_count: String,
    pages: usize,
}

impl ObservabilityPage {
    pub(super) fn prepare_map(&mut self) {
        self.visible_links = self
            .connections
            .iter()
            .enumerate()
            .filter(|(_, edge)| {
                !self.map_problems || matches!(edge.status, Status::Warning | Status::Critical)
            })
            .map(|(index, _)| index)
            .collect();
        if self
            .selected_link
            .as_ref()
            .is_some_and(|id| !self.connections.iter().any(|edge| &edge.id == id))
        {
            self.selected_link = None;
        }
        let pages = self.nodes.len().div_ceil(NODES_PER_PAGE).max(1);
        self.map_page = self.map_page.min(pages - 1);
        let start = self.map_page * NODES_PER_PAGE;
        let mut nodes: Vec<_> = self
            .nodes
            .iter()
            .skip(start)
            .take(NODES_PER_PAGE)
            .cloned()
            .collect();
        // A selected connection is always inspectable even across page boundaries.
        if let Some(edge) = self
            .selected_link
            .as_ref()
            .and_then(|id| self.connections.iter().find(|e| &e.id == id))
        {
            for ix in [edge.from, edge.to] {
                let node = &self.nodes[ix];
                if !nodes.iter().any(|n| n.app == node.app) {
                    nodes.push(node.clone());
                }
            }
        }
        let rows = nodes.len().div_ceil(4).max(1);
        for (ix, node) in nodes.iter_mut().enumerate() {
            node.x = (ix % 4) as f32 * 0.25;
            node.y = (ix / 4) as f32 / rows as f32;
        }
        let positions: BTreeMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(ix, node)| (&node.app, ix))
            .collect();
        let connections: Vec<_> = self
            .visible_links
            .iter()
            .filter_map(|&ix| {
                let mut edge = self.connections[ix].clone();
                edge.from = *positions.get(&edge.id.0)?;
                edge.to = *positions.get(&edge.id.1)?;
                Some(edge)
            })
            .collect();
        let mut markers: Vec<_> = (0..connections.len().min(EDGE_MARKERS)).collect();
        if let Some(ix) = connections
            .iter()
            .position(|e| Some(&e.id) == self.selected_link.as_ref())
            && !markers.contains(&ix)
        {
            markers.push(ix);
        }
        self.map_display = MapDisplay {
            summary: format!(
                "{} of {} applications · {} of {} filtered connections drawn",
                nodes.len(),
                self.nodes.len(),
                connections.len(),
                self.visible_links.len()
            ),
            page_label: format!("Page {} of {}", self.map_page + 1, pages),
            connection_count: format!(
                "{} of {} connections",
                self.visible_links.len(),
                self.connections.len()
            ),
            nodes: nodes.into(),
            connections: connections.into(),
            markers,
            pages,
        };
    }
    fn select_link(&mut self, id: LinkId) {
        if let Some(edge) = self.connections.iter().find(|edge| edge.id == id) {
            self.map_page = edge.from / NODES_PER_PAGE;
            self.selected_link = Some(id);
            self.prepare_map();
        }
    }
    fn map_toolbar(&self, cx: &Context<Self>) -> Div {
        line()
            .flex_wrap()
            .child(section("Service map"))
            .child(div().flex_1())
            .when(self.map_display.pages > 1, |this| {
                this.child(
                    action("obs-map-previous", "Previous")
                        .disabled(self.map_page == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.map_page = this.map_page.saturating_sub(1);
                            this.prepare_map();
                            cx.notify();
                        })),
                )
                .child(muted(self.map_display.page_label.clone(), cx))
                .child(
                    action("obs-map-next", "Next")
                        .disabled(self.map_page + 1 >= self.map_display.pages)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.map_page += 1;
                            this.prepare_map();
                            cx.notify();
                        })),
                )
            })
            .child(
                action("obs-map-problems", "Problem connections")
                    .selected(self.map_problems)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.map_problems = !this.map_problems;
                        this.prepare_map();
                        cx.notify();
                    })),
            )
    }
    fn map_graph(&self, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let nodes = self.map_display.nodes.clone();
        let connections = self.map_display.connections.clone();
        let selected = self.selected_link.clone();

        let edges = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for edge in connections.iter() {
                    let from = &nodes[edge.from];
                    let to = &nodes[edge.to];
                    let y = |node: &MapNode| node.y;
                    let a = point(
                        bounds.left() + bounds.size.width * (from.x + 0.18),
                        bounds.top() + bounds.size.height * y(from) + ui::dp_px(27., window),
                    );
                    let b = point(
                        bounds.left() + bounds.size.width * to.x,
                        bounds.top() + bounds.size.height * y(to) + ui::dp_px(27., window),
                    );
                    let color = if selected.as_ref() == Some(&edge.id) {
                        p.accent
                    } else {
                        match edge.status {
                            Status::Critical => p.crit,
                            Status::Warning => p.warn,
                            _ => p.faint,
                        }
                    };
                    let mut path = PathBuilder::stroke(px(edge.traffic));
                    if edge.status == Status::Ok {
                        path.move_to(a);
                        path.line_to(b);
                    } else {
                        for part in (0..20).step_by(2) {
                            let t = part as f32 / 20.;
                            let next = (part + 1) as f32 / 20.;
                            path.move_to(a + (b - a) * t);
                            path.line_to(a + (b - a) * next);
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size_full();
        let map = div()
            .id("obs-service-map")
            .test_support()
            .relative()
            .min_w(dp(600.))
            .h(dp(
                (self.map_display.nodes.len().div_ceil(4).max(6) * 85) as f32
            ))
            .child(edges)
            .children(self.map_display.markers.iter().map(|&ix| {
                let edge = &self.map_display.connections[ix];
                let from = &self.map_display.nodes[edge.from];
                let to = &self.map_display.nodes[edge.to];
                let y = (from.y + to.y) / 2.;
                let id = edge.id.clone();
                Button::new(edge.element_id.clone())
                    .ghost()
                    .group("fog-control")
                    .absolute()
                    .left(relative((from.x + 0.20 + to.x) / 2. - 0.01))
                    .top(relative(y + 0.04))
                    .w(dp(24.))
                    .h(dp(22.))
                    .px_0()
                    .child(status(edge.status, cx))
                    .tooltip(edge.tooltip.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_link(id.clone());
                        cx.notify();
                    }))
            }))
            .children(self.map_display.nodes.iter().map(|node| {
                let key = node.app.clone();
                let ypos = node.y;
                Button::new(node.element_id.clone())
                    .outline()
                    .group("fog-control")
                    .absolute()
                    .left(relative(node.x))
                    .top(relative(ypos))
                    .w(relative(0.20))
                    .h(dp(54.))
                    .px(dp(10.))
                    .bg(p.surface_2)
                    .border_color(if self.selected_app.as_ref() == Some(&node.app) {
                        p.accent_line
                    } else {
                        p.line_strong
                    })
                    .justify_start()
                    .child(status(node.status, cx))
                    .child(
                        v_flex()
                            .min_w_0()
                            .items_start()
                            .child(
                                mono(node.label.clone())
                                    .truncate()
                                    .font_weight(ui::HEADING_WEIGHT),
                            )
                            .child(muted(node.namespace.clone(), cx).truncate()),
                    )
                    .tooltip(node.tooltip.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_app(key.clone(), Report::Net, cx)
                    }))
            }));
        map.into_any_element()
    }
    fn map_inspector(&self, stacked: bool, cx: &mut Context<Self>) -> Div {
        let mut inspector = card("Connections", cx).when_else(
            stacked,
            |this| this.w_full(),
            |this| this.w(dp(264.)).flex_none(),
        );
        if let Some(edge) = self
            .selected_link
            .as_ref()
            .and_then(|id| self.connections.iter().find(|edge| &edge.id == id))
        {
            let app = edge.id.0.clone();
            inspector = inspector.child(
                body()
                    .child(
                        line()
                            .child(status(edge.status, cx))
                            .child(text(edge.status.label())),
                    )
                    .child(mono(edge.label.clone()))
                    .child(text(edge.detail.clone()))
                    .child(
                        action("obs-map-open-app", "Open application").on_click(cx.listener(
                            move |this, _, _, cx| this.open_app(app.clone(), Report::Net, cx),
                        )),
                    ),
            );
        } else {
            inspector = inspector
                .child(body().child(muted("Choose a connection to inspect its evidence", cx)));
        }
        inspector.child(
            body()
                .child(muted(self.map_display.connection_count.clone(), cx))
                .child(
                    uniform_list(
                        "obs-map-connections",
                        self.visible_links.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|ix| this.connection_row(this.visible_links[ix], cx))
                                .collect()
                        }),
                    )
                    .track_scroll(&self.map_scroll)
                    .h(dp(300.))
                    .w_full(),
                ),
        )
    }
    fn connection_row(&self, ix: usize, cx: &Context<Self>) -> Div {
        let edge = &self.connections[ix];
        let id = edge.id.clone();
        div().h(dp(30.)).w_full().child(
            Button::new(edge.button_id.clone())
                .ghost()
                .small()
                .w_full()
                .justify_start()
                .selected(self.selected_link.as_ref() == Some(&edge.id))
                .child(status(edge.status, cx))
                .child(text(edge.label.clone()).truncate())
                .tooltip(edge.tooltip.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_link(id.clone());
                    cx.notify();
                })),
        )
    }
    pub(super) fn render_map(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let stacked = crate::screens::content_width(window) < 900.;
        if self.nodes.is_empty() {
            return v_flex()
                .gap(dp(12.))
                .child(self.map_toolbar(cx))
                .child(text("No service map nodes were returned"))
                .into_any_element();
        }
        let toolbar = self.map_toolbar(cx);
        let map = self.map_graph(cx);
        let map_panel = v_flex()
            .id("obs-map-panel")
            .test_support()
            .when(stacked, |this| this.w_full())
            .flex_1()
            .min_w_0()
            .rounded(px(12.))
            .bg(p.surface)
            .border_1()
            .border_color(p.line)
            .child(line().justify_between().p(dp(14.)).child(muted(
                self.map_display.summary.clone(),
                cx,
            )))
            .child(
                div()
                    .id("obs-map-horizontal")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .child(map),
            )
            .child(
                line()
                    .p(dp(14.))
                    .flex_wrap()
                    .children(
                        [
                            ("healthy", Status::Ok),
                            ("unknown", Status::Unknown),
                            ("degraded", Status::Warning),
                            ("failing", Status::Critical),
                        ]
                        .map(|(label, state)| {
                            line().child(status(state, cx)).child(muted(label, cx))
                        }),
                    )
                    .child(muted(
                        "Line width = traffic · first 24 markers shown · every connection is in the inspector",
                        cx,
                    )),
            );
        let inspector = self.map_inspector(stacked, cx);
        v_flex()
            .gap(dp(14.))
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .when_else(stacked, |this| this.flex_col(), |this| this.flex_row())
                    .items_start()
                    .gap(dp(12.))
                    .child(map_panel)
                    .child(inspector),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Destination, EDGE_MARKERS, NODES_PER_PAGE};
    use crate::observability::{
        projection,
        tests::{large_map, mount},
    };
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
                assert_eq!(page.map_display.nodes.len(), NODES_PER_PAGE);
                assert_eq!(page.map_display.pages, 5);
                assert_eq!(page.visible_links.len(), 300);
                assert!(page.map_display.summary.contains("of 120 applications"));
                assert!(
                    page.map_display
                        .summary
                        .contains("of 300 filtered connections")
                );
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("obs-map-next", cx);
            assert_eq!(page.read(cx).map_page, 1);
            page.read(cx)
                .map_scroll
                .scroll_to_item(299, ScrollStrategy::Bottom);
            window.render_frame(cx);
            let id = page.read(cx).connections[299].button_id.clone();
            window.click(id, cx);
            let selected = page.read(cx).connections[299].id.clone();
            assert_eq!(page.read(cx).selected_link.as_ref(), Some(&selected));
            window.render_frame(cx);
            window.click("obs-map-next", cx);
            let display = &page.read(cx).map_display;
            assert!(display.nodes.iter().any(|n| n.app == selected.0));
            assert!(display.nodes.iter().any(|n| n.app == selected.1));
            assert!(display.nodes.len() <= NODES_PER_PAGE + 2);
            assert!(display.markers.len() <= EDGE_MARKERS + 1);
            assert!(
                display
                    .markers
                    .iter()
                    .any(|&ix| display.connections[ix].id == selected)
            );
            page.update(cx, |page, _| {
                (page.nodes, page.connections) = projection::map(&Default::default());
                page.prepare_map();
                assert!(page.selected_link.is_none());
                assert_eq!(page.map_page, 0);
                assert!(page.map_display.nodes.is_empty());
            });
        })
        .unwrap();
    }
    #[gpui_kit::test]
    fn narrow_map_scrolls_inside_its_card_at_large_text(cx: &mut TestAppContext) {
        let (_runtime, handle, page) =
            crate::observability::tests::mount_size(cx, true, 760., 560.);
        cx.update(|cx| {
            crate::text_size::set(20., cx);
            page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx));
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            use gpui_kit::{ScrollDelta, point, px};
            window.render_frame(cx);
            let panel = window.find("obs-map-panel").bounds();
            let viewport = window.find("obs-map-horizontal").bounds();
            let before = window.find("obs-service-map").bounds();
            assert!(panel.right() <= px(760.));
            assert!(viewport.right() <= panel.right());
            assert!(before.size.width > viewport.size.width);
            let node = page.read(cx).nodes[0].element_id.clone();
            window.scroll(node, ScrollDelta::Pixels(point(px(-1000.), px(0.))), cx);
            window.render_frame(cx);
            assert!(window.find("obs-service-map").bounds().left() < before.left());
        })
        .unwrap();
    }
}
