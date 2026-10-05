//! Needs attention, on Overview and in a node's pane, and where its
//! subjects open.
use super::Pilot;
use crate::logs::TalosPanel;
use crate::palette::palette;
use crate::presentation::attention::{AttentionRow, Destination};
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::card::{CardHeader, ChartCard};
use freshkube_ui::table::{self, GroupRow};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
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

    /// Needs attention as a card of compact rows grouped by severity:
    /// eight on Overview until Show all, at most fifty, and every row of a
    /// node in its pane.
    pub(in crate::desktop) fn render_attention(
        &self,
        node: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let attention = &self.attention;
        let (rows, details) = match node {
            Some(node) => (
                attention.by_node.get(node).map_or(&[][..], Vec::as_slice),
                attention.node_details.get(node),
            ),
            None => (attention.rows.as_slice(), Some(&attention.details)),
        };
        let count = if self.attention_expanded || node.is_some() {
            rows.len()
        } else {
            rows.len().min(8)
        };
        let mut lines = Vec::new();
        let mut group = None;
        for row in &rows[..count] {
            if group != Some(row.group) {
                group = Some(row.group);
                let detail = details.map(|details| details[row.group.index()].to_string());
                lines.push(
                    GroupRow::new(
                        row.group.id(),
                        row.group.tone(),
                        row.group.label(),
                        table::COMPACT_ROW_HEIGHT,
                    )
                    .detail(detail.into_iter().collect())
                    .render(cx)
                    .into_any_element(),
                );
            }
            lines.push(self.attention_row(row, cx));
        }
        // Folded rows on Overview, and past the cap the rows that can't show.
        let showing = node.is_none() && attention.total > count;
        let bar = showing.then(|| {
            let show_all = (!self.attention_expanded).then(|| {
                Button::new("attention-show-all")
                    .xsmall()
                    .ghost()
                    .label(attention.more.clone())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.attention_expanded = true;
                        cx.notify();
                    }))
            });
            table::showing_bar("overview-collapsed", count, attention.total, show_all, cx)
        });
        let empty = rows.is_empty().then(|| {
            div()
                .px(dp(14.))
                .py(dp(12.))
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child(if attention.complete {
                    "No problems reported"
                } else {
                    "Some sources are unavailable; attention may be incomplete"
                })
        });
        let body = v_flex().children(bar).children(empty).child(
            v_flex()
                .id("needs-attention-rows")
                .test_support()
                .children(lines),
        );
        let stale = if self.overview.is_stale() {
            self.overview.error()
        } else if self.kubernetes_summary.is_stale() {
            self.kubernetes_summary.error()
        } else {
            None
        };
        let header = CardHeader::new("needs-attention", "Needs attention")
            .stale(stale.map(|error| SharedString::from(error.to_owned())));
        // The card's frame fills a grid cell; here it takes its rows' height.
        ChartCard::new(header)
            .render(body, cx)
            .h_auto()
            .into_any_element()
    }

    /// One problem on a compact row: its glyph, kind and name, the reason
    /// truncating with the whole of it in the tooltip, and its actions.
    fn attention_row(&self, row: &AttentionRow, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let open = row.open.clone();
        let reason = row.reason.clone();
        h_flex()
            .id(row.id.clone())
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(row.name.clone())
            .w_full()
            .h(dp(table::COMPACT_ROW_HEIGHT))
            .px_3()
            .gap(dp(10.))
            .border_b_1()
            .border_color(p.line)
            .text_size(dp(12.5))
            .tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx))
            .children(ui::status_glyph(row.tone, cx))
            .child(
                div()
                    .flex_none()
                    .w(dp(104.))
                    .truncate()
                    .text_color(p.muted)
                    .child(row.kind),
            )
            .child(
                div()
                    .flex_shrink()
                    .min_w(dp(80.))
                    .max_w(dp(280.))
                    .truncate()
                    .font_family(MONO_FONT)
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(p.muted)
                    .child(row.reason.clone()),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("attention-open")
                            .xsmall()
                            .ghost()
                            .label("Open")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_destination(open.clone(), window, cx)
                            })),
                    )
                    .children(row.logs.clone().map(|logs| {
                        Button::new("attention-logs")
                            .xsmall()
                            .ghost()
                            .label("Logs")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_destination(logs.clone(), window, cx)
                            }))
                    }))
                    .children(row.open_node.clone().map(|node| {
                        Button::new("attention-open-node")
                            .xsmall()
                            .ghost()
                            .label("Open node")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_destination(node.clone(), window, cx)
                            }))
                    })),
            )
            .into_any_element()
    }
}
