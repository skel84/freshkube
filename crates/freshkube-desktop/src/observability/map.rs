use super::*;
impl ObservabilityPage {
    pub(super) fn prepare_map(&mut self) {
        self.visible_links = self
            .connections
            .iter()
            .enumerate()
            .filter(|(_, edge)| !self.map_problems || edge.status != Status::Ok)
            .map(|(index, _)| index)
            .collect();
        if !self.visible_links.contains(&self.selected_link) {
            self.selected_link = self.visible_links.first().copied().unwrap_or(0);
        }
    }
    fn map_toolbar(&self, cx: &Context<Self>) -> Div {
        line()
            .flex_wrap()
            .child(section("Service map"))
            .child(muted("payments, platform, edge", cx))
            .child(div().flex_1())
            .child(
                action(
                    "obs-map-layout",
                    if self.map_topology {
                        "Topology"
                    } else {
                        "Tiers"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.map_topology = !this.map_topology;
                    cx.notify();
                })),
            )
            .child(
                action("obs-map-problems", "Problems and neighbours")
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
        let nodes = self.nodes.clone();
        let connections = self.connections.clone();
        let selected = self.selected_link;
        let visible_links = self.visible_links.clone();
        let topology = self.map_topology;
        let edges = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for &ix in &visible_links {
                    let edge = &connections[ix];
                    let from = &nodes[edge.from];
                    let to = &nodes[edge.to];
                    let y = |node: &MapNode| if topology { 0.1 + node.y * 0.8 } else { node.y };
                    let a = point(
                        bounds.left() + bounds.size.width * (from.x + 0.18),
                        bounds.top() + bounds.size.height * y(from) + ui::dp_px(27., window),
                    );
                    let b = point(
                        bounds.left() + bounds.size.width * to.x,
                        bounds.top() + bounds.size.height * y(to) + ui::dp_px(27., window),
                    );
                    let color = if ix == selected {
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
            .h(dp(510.))
            .child(edges)
            .children(self.visible_links.iter().map(|&ix| {
                let edge = &self.connections[ix];
                let from = &self.nodes[edge.from];
                let to = &self.nodes[edge.to];
                let y = (from.y + to.y) / 2.;
                Button::new(SharedString::from(format!("obs-map-edge-{ix}")))
                    .ghost()
                    .group("fog-control")
                    .absolute()
                    .left(relative((from.x + 0.20 + to.x) / 2. - 0.01))
                    .top(relative(if topology {
                        0.1 + y * 0.8 + 0.04
                    } else {
                        y + 0.04
                    }))
                    .w(dp(24.))
                    .h(dp(22.))
                    .px_0()
                    .child(status(edge.status, cx))
                    .tooltip(format!("{} · {}", edge.label, edge.detail))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_link = ix;
                        cx.notify();
                    }))
            }))
            .children(self.nodes.iter().enumerate().map(|(index, node)| {
                let key = node.app;
                let ypos = if topology { 0.1 + node.y * 0.8 } else { node.y };
                Button::new(SharedString::from(format!("obs-map-node-{}", node.app)))
                    .outline()
                    .group("fog-control")
                    .absolute()
                    .left(relative(node.x))
                    .top(relative(ypos))
                    .w(relative(0.20))
                    .h(dp(54.))
                    .px(dp(10.))
                    .bg(p.surface_2)
                    .border_color(if index == 5 {
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
                            .child(mono(node.label).truncate().font_weight(ui::HEADING_WEIGHT))
                            .child(muted(node.namespace, cx).truncate()),
                    )
                    .tooltip(format!("{} · Open application", node.app))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_named(key, Report::Net, cx)),
                    )
            }));
        map.into_any_element()
    }
    fn map_inspector(&self, stacked: bool, cx: &Context<Self>) -> Div {
        let edge = &self.connections[self.selected_link];
        card("Selected link", cx)
            .when_else(
                stacked,
                |this| this.w_full(),
                |this| this.w(dp(264.)).flex_none(),
            )
            .child(
                body()
                    .child(
                        line()
                            .child(status(edge.status, cx))
                            .child(mono(edge.label).font_weight(ui::HEADING_WEIGHT)),
                    )
                    .child(text(edge.detail))
                    .child(pair(
                        "Transport",
                        [
                            "HTTP :4180",
                            "HTTP :8080",
                            "HTTP :8080",
                            "HTTP :8080",
                            "HTTP :8080",
                            "TCP :5432 → Service :6432",
                            "TCP :6379",
                            "TCP :6432",
                        ][self.selected_link],
                        cx,
                    ))
                    .child(pair(
                        "Observed",
                        if edge.status == Status::Ok {
                            "Serving requests"
                        } else if self.selected_link == 3 || self.selected_link == 7 {
                            "Since node event · 14:48"
                        } else {
                            "Since release · 12:52"
                        },
                        cx,
                    ))
                    .child(
                        action("obs-map-open-app", "Open application").on_click(cx.listener(
                            |this, _, _, cx| {
                                let key = this.nodes[this.connections[this.selected_link].from].app;
                                this.open_named(key, Report::Net, cx);
                            },
                        )),
                    )
                    .when(
                        self.nodes[edge.from].app == example::WORKER
                            || self.nodes[edge.to].app == example::WORKER,
                        |this| {
                            this.child(action("obs-map-compare", "Compare releases").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.release = 0;
                                    this.comparison = 1;
                                    this.prepare_release();
                                    this.open(Destination::Deployments, cx);
                                }),
                            ))
                        },
                    )
                    .child(ui::caption("Connections", cx))
                    .children(self.visible_links.iter().map(|&index| {
                        let edge = &self.connections[index];
                        Button::new(SharedString::from(format!("obs-map-link-{index}")))
                            .ghost()
                            .group("fog-control")
                            .small()
                            .selected(index == self.selected_link)
                            .justify_start()
                            .child(status(edge.status, cx))
                            .label(edge.label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_link = index;
                                cx.notify();
                            }))
                    })),
            )
    }
    pub(super) fn render_map(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let stacked = crate::screens::content_width(window) < 900.;
        let toolbar = self.map_toolbar(cx);
        let map = self.map_graph(cx);
        let map_panel = v_flex()
            .flex_1()
            .min_w_0()
            .rounded(px(12.))
            .bg(p.surface)
            .border_1()
            .border_color(p.line)
            .child(
                line()
                    .justify_between()
                    .p(dp(14.))
                    .children(["EDGE", "TIER 1", "TIER 2", "DATA"].map(|s| ui::caption(s, cx))),
            )
            .child(
                div()
                    .id("obs-map-horizontal")
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
                            ("degraded", Status::Warning),
                            ("failing", Status::Critical),
                        ]
                        .map(|(label, state)| {
                            line().child(status(state, cx)).child(muted(label, cx))
                        }),
                    )
                    .child(muted(
                        "Line width = traffic · choose a connection to inspect it",
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
