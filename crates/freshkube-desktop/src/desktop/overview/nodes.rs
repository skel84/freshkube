use super::*;

impl Pilot {
    pub(super) fn nodes_section(
        &self,
        summary: &ClusterSummary,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let view = self.node_view;
        let header = h_flex()
            .gap_2p5()
            .flex_wrap()
            .mb_3()
            .child(
                div()
                    .font_family(DISPLAY_FONT)
                    .text_size(dp(17.))
                    .child("Nodes"),
            )
            .child(div().text_size(dp(12.)).text_color(p.muted).child(format!(
                "{} of {} responding · select a node to show it in Services and Logs",
                summary.responding, summary.total
            )))
            .child(div().flex_1())
            .child(
                ButtonGroup::new("node-view")
                    .outline()
                    .small()
                    .child(
                        Button::new("view-cards")
                            .icon(IconName::LayoutGrid)
                            .label("Cards")
                            .selected(view == NodeView::Cards),
                    )
                    .child(
                        Button::new("view-table")
                            .icon(IconName::Table2)
                            .label("Table")
                            .selected(view == NodeView::Table),
                    )
                    .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                        view.node_view = if selected.first() == Some(&1) {
                            NodeView::Table
                        } else {
                            NodeView::Cards
                        };
                        cx.notify();
                    })),
            );
        let region = div()
            .id("nodes-region")
            .test_support()
            .aria_label("Cluster nodes; use arrow keys to change the target node")
            .key_context("TalosNodes")
            .track_focus(&self.node_focus)
            .on_action(
                cx.listener(|view, _: &PreviousNode, window, cx| view.step_node(-1, window, cx)),
            )
            .on_action(cx.listener(|view, _: &NextNode, window, cx| view.step_node(1, window, cx)));
        let region = match view {
            NodeView::Cards => region
                .grid()
                .grid_cols(card_columns(width))
                .gap(dp(GAP))
                .children(
                    self.nodes
                        .iter()
                        .enumerate()
                        .map(|(ix, node)| self.node_card(ix, node, cx).into_any_element()),
                ),
            NodeView::Table => region.overflow_x_scroll().child(self.node_table(cx)),
        };
        v_flex().child(header).child(region).into_any_element()
    }

    fn node_card(&self, ix: usize, node: &NodeSummary, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let target = self.selected_node.as_ref() == Some(&node.name);
        let name = node.name.clone();
        let label = format!("{} · {} · {}", node.name, node.address, node.role.label());
        let card = v_flex()
            .id(SharedString::from(node.name.clone()))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(target)
            .aria_label(label)
            .gap(dp(13.))
            .p(dp(14.))
            .pb(dp(if node.responding { 8. } else { 14. }))
            .rounded(px(10.))
            .border_1()
            .min_w_0()
            .cursor_pointer()
            .map(|this| {
                if node.responding {
                    this.bg(p.surface)
                } else {
                    this.border_dashed()
                }
            })
            .border_color(if target {
                p.accent
            } else if node.responding {
                p.line
            } else {
                p.line_strong
            })
            .when(target, |this| {
                this.shadow(vec![BoxShadow {
                    color: p.accent,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(1.),
                    inset: false,
                }])
            })
            .when(!target, |this| {
                this.hover(|style| style.border_color(p.line_strong))
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_node_by_name(name.clone(), window, cx);
                window.focus(&view.node_focus, cx);
            }));
        let header = h_flex()
            .gap_1p5()
            .flex_wrap()
            .min_h(dp(20.))
            .child(
                h_flex()
                    .gap_1p5()
                    .mr_0p5()
                    .child(
                        Icon::new(role_icon(node.role))
                            .size(dp(15.))
                            .text_color(p.muted),
                    )
                    .child(ui::caption(node.role.label(), cx)),
            )
            .when(node.etcd_member, |this| {
                this.child(ui::tag(Tone::Outline, None, "etcd", cx))
            })
            .when(target, |this| {
                this.child(ui::tag(
                    Tone::Accent,
                    Some(IconName::Crosshair),
                    "Target",
                    cx,
                ))
            })
            .child(div().flex_1())
            .child(if node.responding {
                div()
                    .id(("responding", ix))
                    .tooltip(|window, cx| {
                        Tooltip::new("Responding to the Talos API").build(window, cx)
                    })
                    .child(div().size(dp(8.)).rounded_full().bg(p.good))
                    .into_any_element()
            } else {
                ui::tag(
                    Tone::Unknown,
                    Some(IconName::CircleDashed),
                    "No response",
                    cx,
                )
                .into_any_element()
            });
        let identity = v_flex()
            .gap(dp(3.))
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(dp(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(dp(18.))
                    .child(node.name.clone()),
            )
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(match &node.version {
                        Some(version) => format!("{} · {version}", node.address),
                        None => node.address.clone(),
                    }),
            );
        let card = card.child(header).child(identity);
        if !node.responding {
            return card
                .child(
                div()
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .line_height(dp(19.))
                    .child(format!(
                        "The Talos API at {}:50000 didn't answer the last refresh. CPU, memory and services are unknown, not failed.",
                        node.address
                    )),
                )
                .into_any_element();
        }
        let metric_row = |label: String| {
            h_flex().gap_2().flex_wrap().text_size(dp(12.)).child(
                div()
                    .text_color(p.ink_2)
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
        };
        let value = |text: String| {
            div()
                .font_family(MONO_FONT)
                .text_size(dp(11.5))
                .text_color(p.ink_2)
                .child(text)
        };
        let samples = self.load_history.get(&node.name);
        let sample_count = samples.len();
        let capped = node
            .cores
            .is_some_and(|cores| samples.iter().all(|value| *value <= cores as f64));
        let load =
            v_flex()
                .gap_1p5()
                .child(metric_row("Load".into()).child(div().flex_1()).child(value(
                    match node.load {
                        Some([one, five, fifteen]) => {
                            format!("{one:.2} · {five:.2} · {fifteen:.2}")
                        }
                        None => "Unavailable".into(),
                    },
                )))
                .child(ui::sparkline(samples, node.cores, cx))
                .child(
                    h_flex()
                        .justify_between()
                        .text_size(dp(11.))
                        .text_color(p.muted)
                        .child(if sample_count < 2 {
                            "collecting samples…"
                        } else {
                            "last 5 min"
                        })
                        .child(match node.cores {
                            Some(cores) if capped => format!("dashed line = {cores} cores"),
                            Some(cores) => format!("above {cores} cores"),
                            None => "cores unknown".into(),
                        }),
                );
        let memory = match node.memory {
            Some(memory) => {
                let percent = memory.percent();
                let level = memory.level();
                v_flex()
                    .gap_1p5()
                    .child(
                        metric_row("Memory".into())
                            .children(ui::memory_tone(level).map(|(tone, text)| {
                                ui::tag(tone, Some(IconName::TriangleAlert), text, cx)
                            }))
                            .child(div().flex_1())
                            .child(value(format!(
                                "{} / {} · {percent:.0} %",
                                format_bytes(memory.used),
                                format_bytes(memory.total)
                            ))),
                    )
                    .child(ui::meter(percent, level, cx))
            }
            None => v_flex().child(
                metric_row("Memory".into())
                    .child(div().flex_1())
                    .child(value("Unavailable".into())),
            ),
        };
        let counts = node.health_counts();
        let unhealthy: Vec<String> = node.unhealthy_services().map(|s| s.id.clone()).collect();
        let services = v_flex()
            .gap(dp(7.))
            .pt(dp(11.))
            .border_t_1()
            .border_color(p.line)
            .child(
                h_flex().gap(dp(5.)).flex_wrap().min_h(dp(10.)).children(
                    node.services
                        .iter()
                        .enumerate()
                        .map(|(service_ix, service)| {
                            let health = presentation::service_health(service);
                            let tip =
                                format!("{} · {}", service.id, presentation::health_text(&health));
                            div()
                                .id(("service-dot", ix * 64 + service_ix))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                                .child(ui::glyph(health, cx))
                        }),
                ),
            )
            .child(
                metric_row(plural(node.services.len(), "service", "services"))
                    .child(div().flex_1())
                    .child(if unhealthy.is_empty() {
                        div().text_color(p.muted).child(format!(
                            "{} healthy · {} not reported",
                            counts.healthy, counts.unknown
                        ))
                    } else {
                        div().text_color(p.crit_ink).child(format!(
                            "{} unhealthy: {}",
                            unhealthy.len(),
                            unhealthy.join(", ")
                        ))
                    }),
            );
        let services_target = node.name.clone();
        let logs_target = node.name.clone();
        let actions = h_flex()
            .gap_0p5()
            .ml(dp(-6.))
            .child(
                Button::new(("node-services", ix))
                    .ghost()
                    .small()
                    .icon(IconName::HeartPulse)
                    .label("Services")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.select_node_by_name(services_target.clone(), window, cx);
                        view.navigate(Page::Services, window, cx);
                    })),
            )
            .child(
                Button::new(("node-logs", ix))
                    .ghost()
                    .small()
                    .icon(IconName::ScrollText)
                    .label("Logs")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.select_node_by_name(logs_target.clone(), window, cx);
                        view.navigate(Page::Logs, window, cx);
                    })),
            );
        card.child(load)
            .child(memory)
            .child(services)
            .child(actions)
            .into_any_element()
    }

    fn node_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        // Node takes the remaining width; the rest fit a 1040 px pane.
        const COLUMNS: [(&str, f32); 8] = [
            ("Role", 126.),
            ("Node", 200.),
            ("Address", 112.),
            ("Status", 124.),
            ("CPU", 80.),
            ("Load 1 / 5 / 15", 150.),
            ("Memory", 122.),
            ("Services", 110.),
        ];
        let cell = |ix: usize| {
            let cell = div().px_3().min_w_0().whitespace_nowrap();
            if ix == 1 {
                cell.flex_1().min_w(dp(COLUMNS[ix].1))
            } else {
                cell.flex_none().w(dp(COLUMNS[ix].1))
            }
        };
        let head = h_flex()
            .py(dp(10.))
            .border_b_1()
            .border_color(p.line)
            .children(
                COLUMNS
                    .iter()
                    .enumerate()
                    .map(|(ix, (label, _))| cell(ix).child(ui::caption(label, cx))),
            );
        let rows =
            self.nodes.iter().enumerate().map(|(ix, node)| {
                let target = self.selected_node.as_ref() == Some(&node.name);
                let name = node.name.clone();
                let counts = node.health_counts();
                h_flex()
                    .id(SharedString::from(node.name.clone()))
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_selected(target)
                    .aria_label(format!(
                        "{} · {} · {}",
                        node.name,
                        node.address,
                        node.role.label()
                    ))
                    .py(dp(9.))
                    .border_b_1()
                    .border_color(p.line)
                    .cursor_pointer()
                    .when(target, |this| this.bg(p.accent_soft))
                    .when(!target, |this| this.hover(|style| style.bg(p.hover)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_node_by_name(name.clone(), window, cx);
                        window.focus(&view.node_focus, cx);
                    }))
                    .child(
                        cell(0).child(
                            h_flex()
                                .gap_1p5()
                                .child(
                                    Icon::new(role_icon(node.role))
                                        .size(dp(14.))
                                        .text_color(p.muted),
                                )
                                .child(div().text_size(dp(12.5)).child(node.role.label())),
                        ),
                    )
                    .child(
                        cell(1)
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .child(node.name.clone()),
                    )
                    .child(
                        cell(2)
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .child(node.address.clone()),
                    )
                    .child(cell(3).child(if node.responding {
                        ui::tag(Tone::Good, None, "Responding", cx)
                    } else {
                        ui::tag(
                            Tone::Unknown,
                            Some(IconName::CircleDashed),
                            "No response",
                            cx,
                        )
                    }))
                    .child(
                        cell(4).text_size(dp(12.5)).child(
                            node.cores
                                .map(|c| format!("{c} cores"))
                                .unwrap_or_else(|| "—".into()),
                        ),
                    )
                    .child(cell(5).font_family(MONO_FONT).text_size(dp(12.)).child(
                        match node.load {
                            Some([a, b, c]) => format!("{a:.2} / {b:.2} / {c:.2}"),
                            None => "—".into(),
                        },
                    ))
                    .child(
                        cell(6).child(match node.memory {
                            Some(memory) => h_flex()
                                .id(("memory-cell", ix))
                                .gap_2()
                                .tooltip({
                                    let detail = format!(
                                        "{} of {} used",
                                        format_bytes(memory.used),
                                        format_bytes(memory.total)
                                    );
                                    move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
                                })
                                .child(div().w(dp(48.)).child(ui::meter(
                                    memory.percent(),
                                    memory.level(),
                                    cx,
                                )))
                                .child(
                                    div()
                                        .font_family(MONO_FONT)
                                        .text_size(dp(12.))
                                        .child(format!("{:.0} %", memory.percent())),
                                )
                                .into_any_element(),
                            None => div().child("—").into_any_element(),
                        }),
                    )
                    .child(cell(7).text_size(dp(12.5)).child(if counts.unhealthy > 0 {
                        div()
                            .text_color(p.crit_ink)
                            .child(format!("{} unhealthy", counts.unhealthy))
                    } else if node.services.is_empty() {
                        div().text_color(p.muted).child("—")
                    } else {
                        div().text_color(p.muted).child(plural(
                            node.services.len(),
                            "service",
                            "services",
                        ))
                    }))
            });
        v_flex()
            .min_w(dp(COLUMNS.iter().map(|(_, width)| width).sum::<f32>()))
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .overflow_hidden()
            .child(head)
            .children(rows)
            .into_any_element()
    }
}
