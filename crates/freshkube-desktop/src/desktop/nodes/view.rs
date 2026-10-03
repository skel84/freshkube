use super::*;
use crate::{
    palette::palette,
    screens::content_width,
    ui::{MONO_FONT, dp, dp_px},
};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    assets::IconName,
    component::{
        button::{Button, ButtonGroup},
        h_flex,
        resizable::{h_resizable, resizable_panel},
        v_flex,
    },
    prelude::*,
};
use gpui_kit::{base::Selectable, component::Sizable};

impl Pilot {
    pub(in crate::desktop) fn render_nodes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pane = self.node_workspace.open;
        let body = if pane {
            let detail = self.render_node_pane(window, cx);
            if self.node_workspace.expanded || content_width(window) < 900. {
                detail
            } else {
                h_resizable("nodes-split")
                    .with_state(&self.node_workspace.split)
                    .child(
                        resizable_panel()
                            .size(dp_px(280., window))
                            .size_range(dp_px(240., window)..Pixels::MAX)
                            .child(self.joined_node_list(true, window, cx)),
                    )
                    .child(
                        resizable_panel()
                            .size_range(dp_px(480., window)..Pixels::MAX)
                            .child(detail),
                    )
                    .into_any_element()
            }
        } else {
            self.joined_node_list(false, window, cx)
        };
        v_flex()
            .id("nodes-page")
            .test_support()
            .size_full()
            .min_h_0()
            .key_context("NodeWorkspace")
            .track_focus(&self.node_focus)
            .on_action(cx.listener(|view, _: &super::super::NextNode, window, cx| {
                view.step_joined_node(1, window, cx)
            }))
            .on_action(
                cx.listener(|view, _: &super::super::PreviousNode, window, cx| {
                    view.step_joined_node(-1, window, cx)
                }),
            )
            .on_action(cx.listener(|view, _: &ExpandNode, _, cx| {
                if view.node_workspace.open {
                    view.node_workspace.expanded = !view.node_workspace.expanded;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|view, _: &CloseNode, window, cx| view.close_node(window, cx)))
            .on_action(
                cx.listener(|view, _: &crate::logs::ClearSelection, window, cx| {
                    view.node_back(window, cx)
                }),
            )
            .on_action(cx.listener(|view, _: &BackNode, window, cx| view.node_back(window, cx)))
            .on_action(
                cx.listener(|view, _: &NextNodeTab, window, cx| view.step_node_tab(1, window, cx)),
            )
            .on_action(cx.listener(|view, _: &PreviousNodeTab, window, cx| {
                view.step_node_tab(-1, window, cx)
            }))
            .on_action(cx.listener(|view, _: &OpenNode, window, cx| {
                if let Some(key) = view
                    .node_workspace
                    .selected
                    .clone()
                    .or_else(|| view.node_workspace.rows.first().map(|row| row.key.clone()))
                {
                    view.open_node(key, window, cx);
                }
            }))
            .p(dp(18.))
            .gap(dp(14.))
            .when(!pane, |this| {
                this.child(
                    h_flex()
                        .gap(dp(10.))
                        .child(div().flex_1().text_size(dp(26.)).child("Nodes"))
                        .child(
                            ButtonGroup::new("nodes-view")
                                .outline()
                                .small()
                                .child(
                                    Button::new("nodes-view-cards")
                                        .label("Cards")
                                        .selected(self.node_workspace.view == NodeView::Cards),
                                )
                                .child(
                                    Button::new("nodes-view-table")
                                        .label("Table")
                                        .selected(self.node_workspace.view == NodeView::Table),
                                )
                                .on_click(cx.listener(|view, choice: &Vec<usize>, _, cx| {
                                    view.node_workspace.view = if choice.first() == Some(&0) {
                                        NodeView::Cards
                                    } else {
                                        NodeView::Table
                                    };
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn joined_node_list(
        &self,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some((title, message)) = &self.node_workspace.empty {
            return ui::empty_state(
                IconName::Server,
                title.clone(),
                message.clone(),
                None,
                Vec::new(),
                cx,
            )
            .into_any_element();
        }
        let cards = self.node_workspace.view == NodeView::Cards && !compact;
        let columns = if cards {
            ((content_width(window) + 14.) / 330.).floor().clamp(1., 3.) as usize
        } else {
            1
        };
        let rows = self.node_workspace.rows.clone();
        let talos = self.kubernetes_only.is_none();
        let list =
            uniform_list(
                "joined-nodes-list",
                rows.len().div_ceil(columns),
                cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| {
                            if cards {
                                let start = index * columns;
                                let end = (start + columns).min(rows.len());
                                div()
                                    .grid()
                                    .grid_cols(columns as u16)
                                    .gap(dp(14.))
                                    .pb(dp(14.))
                                    .children(rows[start..end].iter().map(|row| {
                                        view.joined_node_row(row, compact, true, talos, cx)
                                    }))
                                    .into_any_element()
                            } else {
                                view.joined_node_row(&rows[index], compact, false, talos, cx)
                            }
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.node_workspace.scroll)
            .size_full();
        let table = v_flex()
            .size_full()
            .min_h_0()
            .when(!compact && !cards, |this| {
                this.min_w(dp(if talos { 960. } else { 470. }))
            })
            .when(!compact && !cards, |this| {
                this.child(
                    h_flex()
                        .px(dp(12.))
                        .py(dp(8.))
                        .gap(dp(12.))
                        .child(div().flex_1().min_w(dp(170.)).child("Name"))
                        .children(
                            ["Role", "Kubernetes"]
                                .map(|name| div().w(dp(90.)).child(ui::caption(name, cx))),
                        )
                        .when(talos, |this| {
                            this.children(
                                ["Talos", "Load", "Memory"]
                                    .map(|name| div().w(dp(90.)).child(ui::caption(name, cx))),
                            )
                        })
                        .child(div().w(dp(50.)).child(ui::caption("Pods", cx)))
                        .when(talos, |this| {
                            this.child(
                                div()
                                    .id("node-table-services")
                                    .test_support()
                                    .w(dp(140.))
                                    .child(ui::caption("System services", cx)),
                            )
                        }),
                )
            })
            .child(list);
        if !compact && !cards {
            div()
                .id("nodes-table-scroll")
                .test_support()
                .size_full()
                .overflow_x_scroll()
                .child(table)
                .into_any_element()
        } else {
            table.into_any_element()
        }
    }

    fn joined_node_row(
        &self,
        row: &NodeRow,
        compact: bool,
        cards: bool,
        talos: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let selected = self.node_workspace.selected.as_ref() == Some(&row.key);
        let key = row.key.clone();
        let content = if compact {
            v_flex()
                .gap(dp(5.))
                .child(
                    h_flex()
                        .gap(dp(8.))
                        .child(ui::tag(row.tone, None, row.ready, cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .truncate()
                                .id("node-row-name")
                                .test_support()
                                .font_family(MONO_FONT)
                                .text_size(dp(12.))
                                .child(row.name.clone()),
                        ),
                )
                .child(div().text_color(p.muted).child(row.note.clone()))
                .into_any_element()
        } else if cards {
            v_flex()
                .gap(dp(10.))
                .child(
                    h_flex()
                        .gap(dp(10.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .truncate()
                                .id("node-row-name")
                                .test_support()
                                .font_family(MONO_FONT)
                                .child(row.name.clone()),
                        )
                        .child(ui::tag(row.tone, None, row.ready, cx)),
                )
                .child(div().text_color(p.muted).child(row.address.clone()))
                .when_some(row.talos.as_ref(), |this, node| {
                    this.child(ui::sparkline(
                        self.load_history.get(&node.name),
                        node.cores,
                        cx,
                    ))
                })
                .when_some(
                    row.talos.as_ref().and_then(|node| node.memory),
                    |this, memory| this.child(ui::meter(memory.percent(), memory.level(), cx)),
                )
                .child(
                    h_flex()
                        .gap(dp(14.))
                        .child(row.memory.clone())
                        .child(row.services.clone()),
                )
                .child(
                    Button::new(row.open_id.clone())
                        .small()
                        .outline()
                        .label("Open")
                        .on_click(cx.listener({
                            let key = key.clone();
                            move |view, _, window, cx| view.open_node(key.clone(), window, cx)
                        })),
                )
                .into_any_element()
        } else {
            h_flex()
                .gap(dp(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w(dp(170.))
                        .whitespace_nowrap()
                        .truncate()
                        .id("node-row-name")
                        .test_support()
                        .font_family(MONO_FONT)
                        .child(row.name.clone()),
                )
                .child(div().w(dp(90.)).child(row.role.label()))
                .child(
                    div()
                        .w(dp(90.))
                        .child(ui::tag(row.tone, None, row.ready, cx)),
                )
                .when(talos, |this| {
                    this.children(
                        [
                            row.talos_state.clone(),
                            row.load.clone(),
                            row.memory.clone(),
                        ]
                        .map(|value| div().w(dp(90.)).truncate().child(value)),
                    )
                })
                .child(div().w(dp(50.)).child(row.pods.clone()))
                .when(talos, |this| {
                    this.child(div().w(dp(140.)).truncate().child(row.services.clone()))
                })
                .into_any_element()
        };
        let name = row.name.clone();
        div()
            .id(row.id.clone())
            .test_support()
            .role(gpui_kit::Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(row.name.clone())
            .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
            .when(cards, |this| {
                this.h(dp(232.))
                    .rounded(px(10.))
                    .border_1()
                    .overflow_hidden()
            })
            .px(dp(12.))
            .py(dp(if cards { 14. } else { 10. }))
            .border_b_1()
            .border_color(p.line)
            .bg(if selected { p.surface } else { p.surface_2 })
            .cursor_pointer()
            .hover(|style| style.bg(p.surface))
            .child(content)
            .on_click(
                cx.listener(move |view, _, window, cx| view.open_node(key.clone(), window, cx)),
            )
            .into_any_element()
    }

    fn render_node_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.node_workspace.row().cloned() else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let tab = self.node_workspace.tab;
        let body = match tab {
            NodeTab::Overview => div()
                .id("node-overview")
                .overflow_y_scroll()
                .p(dp(16.))
                .child(
                    v_flex()
                        .gap(dp(12.))
                        .child(self.render_attention(Some(row.name.as_ref()), cx))
                        .children(
                            row.problems
                                .iter()
                                .map(|problem| ui::tag(row.tone, None, problem.clone(), cx)),
                        )
                        .when_some(row.talos.as_ref(), |this, node| {
                            this.child(ui::sparkline(
                                self.load_history.get(&node.name),
                                node.cores,
                                cx,
                            ))
                        })
                        .when_some(
                            row.talos.as_ref().and_then(|node| node.memory),
                            |this, memory| {
                                this.child(ui::meter(memory.percent(), memory.level(), cx))
                            },
                        )
                        .when(self.node_history.read(cx).shows(), |this| {
                            this.child(self.node_history.clone())
                        })
                        .children(row.facts.iter().map(|(label, value)| {
                            h_flex()
                                .gap(dp(12.))
                                .child(div().w(dp(132.)).text_color(p.muted).child(label.clone()))
                                .child(div().flex_1().child(value.clone()))
                        })),
                )
                .into_any_element(),
            NodeTab::Pods => self.node_pods.clone().into_any_element(),
            NodeTab::Services => self.render_services(window, cx),
            NodeTab::Logs => self.render_logs_page(),
            NodeTab::Events | NodeTab::Yaml => {
                self.node_workspace.document.clone().into_any_element()
            }
            _ => self
                .active_screen()
                .map(|screen| screen.view().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
        let header = h_flex()
            .gap(dp(8.))
            .flex_wrap()
            .when(content_width(window) < 900., |this| {
                this.child(
                    Button::new("node-back")
                        .small()
                        .label("Back")
                        .on_click(cx.listener(|view, _, window, cx| view.close_node(window, cx))),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(MONO_FONT)
                    .text_size(dp(20.))
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(ui::tag(row.tone, None, row.ready, cx))
            .child(
                Button::new("node-expand")
                    .small()
                    .outline()
                    .label(if self.node_workspace.expanded {
                        "Collapse"
                    } else {
                        "Expand"
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.node_workspace.expanded = !view.node_workspace.expanded;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("node-close")
                    .small()
                    .icon(IconName::X)
                    .on_click(cx.listener(|view, _, window, cx| view.close_node(window, cx))),
            );
        let tabs = h_flex()
            .id("node-tabs")
            .key_context("NodeWorkspaceTabs")
            .track_focus(&self.node_workspace.tab_focus)
            .tab_index(0)
            .gap(dp(4.))
            .flex_none()
            .overflow_x_scroll()
            .track_scroll(&self.node_workspace.tab_scroll)
            .children(self.node_workspace.inline_tabs.iter().map(|tab| {
                Button::new(tab.id())
                    .small()
                    .label(if *tab == NodeTab::Pods {
                        row.pod_label.clone()
                    } else {
                        tab.label().into()
                    })
                    .selected(*tab == self.node_workspace.tab)
                    .toggled(*tab == self.node_workspace.tab)
                    .when(
                        *tab == NodeTab::Logs && self.logs.read(cx).collecting_count() > 0,
                        |this| {
                            this.icon(
                                gpui_kit::component::Icon::new(IconName::Circle)
                                    .text_color(p.good)
                                    .size(dp(7.)),
                            )
                        },
                    )
                    .when(*tab == NodeTab::Services && row.service_problem, |this| {
                        this.child(ui::status_mark(
                            "node-services-problem",
                            ui::Tone::Warn,
                            "A system service on this node is unhealthy",
                            cx,
                        ))
                    })
                    .on_click(cx.listener({
                        let tab = *tab;
                        move |view, _, window, cx| {
                            view.show_node_tab(tab, window, cx);
                        }
                    }))
            }))
            .when(self.node_workspace.more, |this| {
                this.child(
                    Button::new("node-more")
                        .small()
                        .label("More")
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let pilot = cx.weak_entity();
                            move |menu, _, _| {
                                let events = pilot.clone();
                                let yaml = pilot.clone();
                                menu.item(PopupMenuItem::new("Events").on_click(
                                    move |_, window, cx| {
                                        let _ = events.update(cx, |view, cx| {
                                            view.show_node_tab(NodeTab::Events, window, cx)
                                        });
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new("YAML").on_click(move |_, window, cx| {
                                        let _ = yaml.update(cx, |view, cx| {
                                            view.show_node_tab(NodeTab::Yaml, window, cx)
                                        });
                                    }),
                                )
                            }
                        }),
                )
            });
        v_flex()
            .id("node-pane")
            .test_support()
            .size_full()
            .min_h_0()
            .gap(dp(12.))
            .pl(dp(12.))
            .child(header)
            .child(
                h_flex()
                    .id("node-chips")
                    .flex_none()
                    .h(dp(26.))
                    .overflow_x_scroll()
                    .gap(dp(6.))
                    .children(
                        row.chips
                            .iter()
                            .map(|chip| ui::tag(ui::Tone::Outline, None, chip.clone(), cx)),
                    ),
            )
            .child(tabs)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
