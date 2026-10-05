//! Needs attention, on Overview and in a node's pane, and where its
//! subjects open.
use super::Pilot;
use crate::logs::TalosPanel;
use crate::palette::palette;
use crate::presentation::attention::Destination;
use crate::ui::{self, MONO_FONT, dp};
use gpui_kit::component::{Sizable, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Pilot {
    pub(in crate::desktop) fn open_destination(
        &mut self,
        target: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Destination::Page(page) => self.navigate_from_keyboard(page, window, cx),
            Destination::Node(key, tab) => {
                self.open_node(key, window, cx);
                self.show_node_tab(tab, window, cx);
            }
            Destination::Object(key, object, tab) => {
                if let Some(kind) = freshkube_core::resources::builtin(key) {
                    self.open_object(kind, object, tab, window, cx);
                }
            }
            Destination::Service {
                node,
                service,
                logs,
            } => {
                self.open_node_by_name(
                    &node,
                    if logs {
                        crate::desktop::nodes::NodeTab::Logs
                    } else {
                        crate::desktop::nodes::NodeTab::Services
                    },
                    window,
                    cx,
                );
                self.selected_service = Some(service.clone());
                if logs {
                    self.logs
                        .update(cx, |view, cx| view.open_service(service, window, cx));
                }
            }
        }
    }

    pub(in crate::desktop) fn render_attention(
        &self,
        node: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let rows = node
            .and_then(|node| self.attention.by_node.get(node))
            .map(Vec::as_slice)
            .unwrap_or(if node.is_some() {
                &[]
            } else {
                &self.attention.rows
            });
        let count = if self.attention_expanded || node.is_some() {
            rows.len()
        } else {
            rows.len().min(8)
        };
        v_flex()
            .id("needs-attention")
            .test_support()
            .gap(dp(10.))
            .child(
                div()
                    .font_weight(ui::HEADING_WEIGHT)
                    .text_size(dp(20.))
                    .child("Needs attention"),
            )
            .when(rows.is_empty(), |this| {
                this.child(div().text_color(p.muted).child(if self.attention.complete {
                    "No problems reported"
                } else {
                    "Some sources are unavailable; attention may be incomplete"
                }))
            })
            .children(rows[..count].iter().map(|row| {
                let open = row.open.clone();
                let logs = row.logs.clone();
                let node = row.open_node.clone();
                v_flex()
                    .id(row.id.clone())
                    .test_support()
                    .min_w_0()
                    .p(dp(10.))
                    .gap(dp(6.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(
                        h_flex()
                            .gap(dp(8.))
                            .flex_wrap()
                            .child(ui::tag(row.tone, None, row.kind, cx))
                            .child(div().font_family(MONO_FONT).child(row.name.clone())),
                    )
                    .child(
                        div()
                            .text_color(p.muted)
                            .text_size(dp(12.))
                            .child(row.reason.clone()),
                    )
                    .child(
                        h_flex()
                            .gap(dp(6.))
                            .child(
                                Button::new("attention-open")
                                    .small()
                                    .label("Open")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_destination(open.clone(), window, cx)
                                    })),
                            )
                            .when_some(logs, |this, logs| {
                                this.child(
                                    Button::new("attention-logs")
                                        .small()
                                        .label("Logs")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_destination(logs.clone(), window, cx)
                                        })),
                                )
                            })
                            .when_some(node, |this, node| {
                                this.child(
                                    Button::new("attention-open-node")
                                        .small()
                                        .label("Open node")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_destination(node.clone(), window, cx)
                                        })),
                                )
                            }),
                    )
            }))
            .when(
                node.is_none() && !self.attention_expanded && self.attention.total > 8,
                |this| {
                    this.child(
                        Button::new("attention-show-all")
                            .small()
                            .label(self.attention.more.clone())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.attention_expanded = true;
                                cx.notify();
                            })),
                    )
                },
            )
            .into_any_element()
    }
}
