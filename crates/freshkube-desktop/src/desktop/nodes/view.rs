use super::*;
use crate::{
    logs::TalosPanel,
    palette::palette,
    screens::page_width,
    ui::{MONO_FONT, dp},
};
use freshkube_ui::{
    inspector::{self, Inspector},
    page::PANE_PADDING,
};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ElementExt, Sizable};
use gpui_kit::{
    assets::IconName,
    component::{
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::*,
};

impl Pilot {
    pub(in crate::desktop) fn render_nodes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pane = self.node_workspace.open;
        let expanded = pane && self.node_workspace.expanded;
        let beside = page_width(window) >= inspector::SPLIT_WIDTH;
        // The node's screens lay out by the inspector's width beside the
        // table; stacked or expanded, it has the page's.
        crate::screens::set_node_pane_width(if beside && !expanded {
            self.node_workspace.split.live_width(cx)
        } else {
            f32::MAX
        });
        // A short window scrolls the frame, and the list and the inspector
        // keep their least heights in it.
        let short = freshkube_ui::page::is_short(window);
        // The table and the inspector run edge to edge; the cards and a
        // state keep the inset.
        let mut edge = true;
        let body = if expanded {
            self.render_node_pane(window, cx)
        } else if let Some(state) = self.nodes_state(cx).filter(|_| !pane) {
            edge = false;
            state
        } else if self.node_workspace.view == NodeView::Table {
            let table = freshkube_ui::table::data_table(self, window, cx)
                .size_full()
                .min_h_0();
            let table = v_flex()
                .id("nodes-table")
                .test_support()
                .size_full()
                .min_h_0()
                .child(table)
                .into_any_element();
            let detail = pane.then(|| self.render_node_pane(window, cx));
            inspector::split(
                "nodes-split",
                &self.node_workspace.split,
                beside,
                table,
                detail,
                window,
            )
        } else if pane {
            // Beside the inspector the cards give way to the roster, one
            // to a row, in their inset.
            let roster = div()
                .size_full()
                .px(dp(freshkube_ui::page::PANE_PADDING))
                .py(dp(freshkube_ui::page::PANE_PADDING_Y))
                .child(self.joined_node_list(true, window, cx))
                .into_any_element();
            let detail = self.render_node_pane(window, cx);
            inspector::split(
                "nodes-split",
                &self.node_workspace.split,
                beside,
                roster,
                Some(detail),
                window,
            )
        } else {
            edge = false;
            self.joined_node_list(false, window, cx)
        };
        let least = self.node_workspace.split.short_height(beside, pane);
        freshkube_ui::page::page("nodes-page")
            .track_scroll(&self.node_workspace.page_scroll)
            .when(short, |this| {
                this.overflow_y_scroll().restrict_scroll_to_axis()
            })
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
            .on_action(cx.listener(|view, _: &ExpandNode, _, cx| view.toggle_node_expanded(cx)))
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
                // Enter opens a node from the list alone; anywhere else in
                // the workspace, such as on a focused button, it keeps its
                // own meaning.
                if !view.node_focus.is_focused(window) {
                    return cx.propagate();
                }
                if let Some(key) = view
                    .node_workspace
                    .selected
                    .clone()
                    .filter(|key| view.line_of(key).is_some())
                    .or_else(|| table::step(view, 1, cx))
                {
                    view.open_node(key, window, cx);
                    view.focus_node_pane(window, cx);
                }
            }))
            .when(!expanded, |this| {
                this.child(freshkube_ui::page::toolbar(cx).child(self.nodes_header(window, cx)))
            })
            .child(
                div()
                    .when(edge, |this| this.flex().flex_col())
                    .flex_1()
                    .min_h_0()
                    .when(short, |this| this.min_h(dp(least)))
                    .when(!edge, |this| {
                        this.px(dp(freshkube_ui::page::PANE_PADDING))
                            .py(dp(freshkube_ui::page::PANE_PADDING_Y))
                    })
                    .child(body),
            )
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
        let body = self.render_node_body(&row, window, cx);
        // The chips wrap onto more lines in a narrow inspector rather than
        // cut one mid-text.
        let chips = (!row.chips.is_empty()).then(|| {
            h_flex()
                .id("node-chips")
                .test_support()
                .flex_wrap()
                .gap(dp(6.))
                .children(
                    row.chips
                        .iter()
                        .map(|chip| ui::tag(ui::Tone::Outline, None, chip.clone(), cx)),
                )
        });
        let inspector = Inspector::new("node-inspector")
            .heading(self.render_node_heading(&row, cx))
            .banner(chips)
            .tabs(
                &self.node_workspace.tab_strip,
                self.render_node_tabs(&row, cx),
            )
            .content(body)
            .render(cx);
        div()
            .id("node-pane")
            .test_support()
            .size_full()
            .min_h_0()
            // A click in the pane that no field, list or log inside it
            // takes moves the keyboard to its tab strip, not the table
            // around it (#340).
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    if !window.default_prevented() {
                        window.focus(&view.node_workspace.tab_focus, cx);
                        window.prevent_default();
                    }
                }),
            )
            .child(inspector)
            .into_any_element()
    }

    /// The selected tab's body, which lays itself out and scrolls itself.
    fn render_node_body(
        &mut self,
        row: &NodeRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        // The bodies that don't pad themselves sit 12 in, as Resources'
        // log does.
        let inset = |body: AnyElement| {
            v_flex()
                .size_full()
                .min_h_0()
                .px(dp(PANE_PADDING))
                .py(dp(PANE_PADDING))
                .child(body)
                .into_any_element()
        };
        match self.node_workspace.tab {
            NodeTab::Overview => div()
                .id("node-overview")
                .size_full()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .p(dp(PANE_PADDING))
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
            NodeTab::Pods => inset(self.node_pods.clone().into_any_element()),
            NodeTab::Services => inset(self.render_services(window, cx)),
            NodeTab::Logs => {
                // In a short window the body scrolls inside the inspector:
                // it's as tall as its room and the log's toolbar and
                // notices, so the lines fill the room once it has scrolled
                // past them.
                let short = freshkube_ui::page::is_short(window);
                let height = self.node_workspace.logs_height.filter(|_| short);
                div()
                    .id("node-logs-scroll")
                    .test_support()
                    .size_full()
                    .min_h_0()
                    .track_scroll(&self.node_workspace.logs_scroll)
                    .when(short, |this| {
                        let logs = self.logs.downgrade();
                        let pilot = cx.entity().downgrade();
                        this.overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .on_prepaint(move |bounds, window, cx| {
                                measure_log_body(&pilot, &logs, bounds.size.height, window, cx)
                            })
                    })
                    .child(
                        v_flex()
                            .size_full()
                            .when_some(height, |this, height| this.min_h(height))
                            .px(dp(PANE_PADDING))
                            .pt(dp(freshkube_ui::page::PANE_PADDING_Y))
                            .pb(dp(PANE_PADDING))
                            .child(self.logs.clone().cached(super::super::cached_page_style())),
                    )
                    .into_any_element()
            }
            NodeTab::Events | NodeTab::Yaml => {
                self.node_workspace.document.clone().into_any_element()
            }
            _ => inset(
                self.active_screen()
                    .map(|screen| screen.view().into_any_element())
                    .unwrap_or_else(|| div().into_any_element()),
            ),
        }
    }

    fn render_node_heading(&self, row: &NodeRow, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_1()
            .min_w_0()
            .gap(dp(8.))
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
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_node_expanded(cx))),
            )
            .child(
                Button::new("node-close")
                    .small()
                    .icon(IconName::X)
                    .tooltip("Close node pane")
                    .accessibility_label("Close node pane")
                    .on_click(cx.listener(|view, _, window, cx| view.close_node(window, cx))),
            )
    }

    fn render_node_tabs(&self, row: &NodeRow, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let collecting = self.logs.read(cx).collecting_count() > 0;
        let workspace = &self.node_workspace;
        // Events and YAML sit in More, so while one shows, More is the
        // strip's active place.
        let active = workspace
            .inline_tabs
            .iter()
            .position(|tab| *tab == workspace.tab)
            .unwrap_or(workspace.inline_tabs.len());
        workspace
            .tab_strip
            .row("node-tabs", active)
            .key_context("NodeWorkspaceTabs")
            .track_focus(&workspace.tab_focus)
            .tab_index(0)
            .gap_1()
            .children(self.node_workspace.inline_tabs.iter().map(|tab| {
                let label: SharedString = if *tab == NodeTab::Pods {
                    row.pod_label.clone()
                } else {
                    tab.label().into()
                };
                inspector::tab(tab.id(), label, *tab == self.node_workspace.tab, cx)
                    .when(*tab == NodeTab::Logs && collecting, |this| {
                        this.child(
                            gpui_kit::component::Icon::new(IconName::Circle)
                                .text_color(p.good)
                                .size(dp(7.)),
                        )
                    })
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
                            view.focus_node_pane(window, cx);
                        }
                    }))
            }))
            .when(self.node_workspace.more, |this| {
                this.child(
                    div().flex_none().ml_1().child(
                        Button::new("node-more")
                            .small()
                            .ghost()
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
                                                view.show_node_tab(NodeTab::Events, window, cx);
                                                view.focus_node_pane(window, cx);
                                            });
                                        },
                                    ))
                                    .item(
                                        PopupMenuItem::new("YAML").on_click(
                                            move |_, window, cx| {
                                                let _ = yaml.update(cx, |view, cx| {
                                                    view.show_node_tab(NodeTab::Yaml, window, cx);
                                                    view.focus_node_pane(window, cx);
                                                });
                                            },
                                        ),
                                    )
                                }
                            }),
                    ),
                )
            })
    }
}

/// Keeps the Logs tab's body as tall as its room and the log's toolbar and
/// notices, and draws again when that changes: the room is known only once
/// the body is laid out.
fn measure_log_body(
    pilot: &WeakEntity<Pilot>,
    logs: &WeakEntity<crate::logs::LogPanel>,
    room: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    // The body's padding around the log.
    let padding = ui::dp_px(freshkube_ui::page::PANE_PADDING_Y + PANE_PADDING, window);
    let Some(height) = logs
        .read_with(cx, |logs, _| padding + logs.height_for_list(room - padding))
        .ok()
    else {
        return;
    };
    let changed = pilot
        .update(cx, |pilot, _| {
            let last = pilot.node_workspace.logs_height;
            let changed = last.is_none_or(|last| (last - height).abs() > px(0.5));
            if changed {
                pilot.node_workspace.logs_height = Some(height);
            }
            changed
        })
        .unwrap_or(false);
    if changed {
        let pilot = pilot.clone();
        window.on_next_frame(move |_, cx| {
            _ = pilot.update(cx, |_, cx| cx.notify());
        });
    }
}
