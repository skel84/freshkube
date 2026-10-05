use super::*;
use crate::{
    palette::palette,
    screens::content_width,
    ui::{MONO_FONT, dp},
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    assets::IconName,
    component::{button::Button, h_flex, v_flex},
    prelude::*,
};
use gpui_kit::{base::Selectable, component::Sizable};

impl Pilot {
    pub(super) fn joined_node_list(
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
}
