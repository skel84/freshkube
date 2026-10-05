use super::*;
use crate::{
    palette::palette,
    screens::inset_width,
    ui::{MONO_FONT, dp},
};
use gpui_kit::component::Sizable;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    assets::IconName,
    component::{button::Button, h_flex, v_flex},
    prelude::*,
};

pub(super) fn card_columns(window: &Window) -> usize {
    ((inset_width(window) + 14.) / 330.).floor().clamp(1., 3.) as usize
}

impl Pilot {
    pub(super) fn joined_node_list(
        &self,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns = if compact { 1 } else { card_columns(window) };
        let count = if compact {
            self.node_workspace.rows.len()
        } else {
            self.node_workspace.lines.len()
        };
        if count == 0 {
            return ui::empty_state(
                IconName::Server,
                "No matching nodes",
                "No nodes match these filters.",
                None,
                Vec::new(),
                cx,
            )
            .id("nodes-empty")
            .test_support()
            .role(gpui_kit::Role::Status)
            .aria_label("No matching nodes")
            .into_any_element();
        }
        let list = uniform_list(
            "nodes-cards-list",
            count.div_ceil(columns),
            cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        if compact {
                            view.joined_node_row(&view.node_workspace.rows[index], true, cx)
                        } else {
                            let start = index * columns;
                            let end = (start + columns).min(view.node_workspace.lines.len());
                            div()
                                .grid()
                                .grid_cols(columns as u16)
                                .gap(dp(14.))
                                .pb(dp(14.))
                                .children(view.node_workspace.lines[start..end].iter().map(|ix| {
                                    view.joined_node_row(&view.node_workspace.rows[*ix], false, cx)
                                }))
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.node_workspace.scroll)
        .size_full();
        let rows = div()
            .id("nodes-cards")
            .test_support()
            .role(gpui_kit::Role::ListBox)
            .size_full()
            .min_h_0()
            .child(list)
            .into_any_element();
        if compact {
            rows
        } else {
            v_flex()
                .size_full()
                .min_h_0()
                .child(div().flex_1().min_h_0().child(rows))
                .child(self.nodes_meter_legend(window, cx))
                .into_any_element()
        }
    }

    fn joined_node_row(&self, row: &NodeRow, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let cards = !compact;
        let selected = self.node_workspace.selected.as_ref() == Some(&row.key);
        let key = row.key.clone();
        let content = if compact {
            v_flex()
                .gap(dp(5.))
                .child(
                    h_flex()
                        .gap(dp(8.))
                        .child(
                            h_flex()
                                .gap(dp(6.))
                                .children(ui::status_glyph(row.tone, cx))
                                .child(row.ready),
                        )
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
        } else {
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
                        .child(
                            h_flex()
                                .gap(dp(6.))
                                .children(ui::status_glyph(row.tone, cx))
                                .child(row.ready),
                        ),
                )
                .child(div().text_color(p.muted).child(row.address.clone()))
                .when_some(
                    self.node_workspace.resource_cells.get(&row.key),
                    |this, resources| {
                        use freshkube_ui::meters::Resource;
                        // One label width, so a card's two bullets line up.
                        let line = |label: &'static str, cell: AnyElement| {
                            h_flex()
                                .gap(dp(10.))
                                .child(div().w(dp(60.)).flex_none().child(label))
                                .child(cell)
                        };
                        this.child(line(
                            "CPU",
                            resources.cpu.render(
                                Resource::Cpu,
                                (row.id.clone(), 20usize).into(),
                                &p,
                            ),
                        ))
                        .child(line(
                            "Memory",
                            resources.memory.render(
                                Resource::Memory,
                                (row.id.clone(), 21usize).into(),
                                &p,
                            ),
                        ))
                    },
                )
                .child(div().text_color(p.muted).child(row.services.clone()))
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
        };
        let name = row.name.clone();
        let frame = if cards {
            freshkube_ui::page::card(cx)
        } else {
            div()
        };
        frame
            .id(row.id.clone())
            .test_support()
            .role(gpui_kit::Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(row.name.clone())
            .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
            .when(cards, |this| this.h(dp(232.)).overflow_hidden())
            .px(dp(12.))
            .py(dp(if cards { 14. } else { 10. }))
            .border_b_1()
            .border_color(if cards && selected { p.accent } else { p.line })
            .when(cards, |this| {
                this.when(selected, |this| this.bg(p.accent_soft))
                    .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            })
            .when(!cards, |this| {
                this.bg(if selected { p.surface } else { p.surface_2 })
                    .hover(|style| style.bg(p.surface))
            })
            .cursor_pointer()
            .child(content)
            .on_click(
                cx.listener(move |view, _, window, cx| view.open_node(key.clone(), window, cx)),
            )
            .into_any_element()
    }
}
