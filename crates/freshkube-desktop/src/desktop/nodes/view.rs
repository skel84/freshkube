use super::*;
use crate::{
    logs::TalosPanel,
    palette::palette,
    screens::content_width,
    ui::{MONO_FONT, dp, dp_px},
};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::{
    assets::IconName,
    component::{
        button::{Button, ButtonVariants},
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
        } else if let Some(state) = self.nodes_state(cx) {
            state
        } else if self.node_workspace.view == NodeView::Table {
            freshkube_ui::table::data_table(self, window, cx)
                .size_full()
                .min_h_0()
                .into_any_element()
        } else {
            self.joined_node_list(false, window, cx)
        };
        freshkube_ui::page::page("nodes-page")
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
                use freshkube_ui::table::{self, TableSource};
                if let Some(key) = view
                    .node_workspace
                    .selected
                    .clone()
                    .filter(|key| view.line_of(key).is_some())
                    .or_else(|| table::step(view, 1, cx))
                {
                    view.open_node(key, window, cx);
                }
            }))
            .when(!pane, |this| this.child(self.nodes_header(window, cx)))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn nodes_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.node_workspace.empty.as_ref()?;
        let (id, label, element) = match state {
            Empty::Loading => (
                "nodes-loading",
                "Waiting for nodes".to_owned(),
                freshkube_ui::page::card(cx)
                    .p_3()
                    .gap_3()
                    .children((0..9).map(|_| ui::skeleton(relative(0.7), dp(12.)))),
            ),
            Empty::Failed(reason) => (
                "nodes-failed",
                format!("Nodes unavailable · {reason}"),
                ui::empty_state(
                    IconName::CircleDashed,
                    "Nodes unavailable",
                    "The cluster summaries could not report nodes. Retry reads them again.",
                    Some(reason.to_string()),
                    vec![
                        Button::new("nodes-retry")
                            .primary()
                            .label("Retry")
                            .on_click(
                                cx.listener(|view, _, window, cx| view.refresh_now(window, cx)),
                            )
                            .into_any_element(),
                    ],
                    cx,
                ),
            ),
            Empty::Loaded => (
                "nodes-empty",
                "No nodes reported".to_owned(),
                ui::empty_state(
                    IconName::Server,
                    "No nodes reported",
                    "No nodes were reported for this context.",
                    None,
                    Vec::new(),
                    cx,
                ),
            ),
        };
        Some(
            element
                .id(id)
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(label)
                .into_any_element(),
        )
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
                div().flex_1().min_w_0().child(
                    div()
                        .id("node-pane-title")
                        .test_support()
                        .aria_label(row.name.clone())
                        .font_family(MONO_FONT)
                        .text_size(dp(13.5))
                        .truncate()
                        .child(row.name.clone()),
                ),
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
