//! Nodes' shared page header and readiness projection, derived on updates.
use super::*;
use crate::{screens::content_width, ui::dp};
use freshkube_ui::{page, table};
use gpui_kit::{
    assets::IconName,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        h_flex,
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    NotReady,
    Unknown,
    Ready,
}

impl Status {
    pub(super) const ALL: [Self; 3] = [Self::NotReady, Self::Unknown, Self::Ready];

    fn index(self) -> usize {
        self as usize
    }

    pub(super) fn of(row: &NodeRow) -> Self {
        match row.ready {
            "Ready" => Self::Ready,
            "NotReady" => Self::NotReady,
            _ => Self::Unknown,
        }
    }

    pub(super) fn tone(self) -> ui::Tone {
        match self {
            Self::NotReady => ui::Tone::Crit,
            Self::Unknown => ui::Tone::Unknown,
            Self::Ready => ui::Tone::Good,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::NotReady => "not ready nodes",
            Self::Unknown => "nodes with unknown readiness",
            Self::Ready => "ready nodes",
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::NotReady => "nodes-chip-not-ready",
            Self::Unknown => "nodes-chip-unknown",
            Self::Ready => "nodes-chip-ready",
        }
    }
}

impl Nodes {
    pub(super) fn rebuild_lines(&mut self) {
        self.counts = [0; 3];
        self.lines = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| {
                let status = Status::of(row);
                self.counts[status.index()] += 1;
                (self.filter.is_none() || self.filter == Some(status)).then_some(ix)
            })
            .collect();
    }
}

impl Pilot {
    pub(super) fn nodes_header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let header = page::PageHeader::new(
            "nodes",
            "Nodes",
            content_width(window) < page::HEADER_NARROW,
        );
        let segment = h_flex().gap(dp(2.)).children(
            [
                ("nodes-view-cards", "Cards", NodeView::Cards),
                ("nodes-view-table", "Table", NodeView::Table),
            ]
            .map(|(id, label, choice)| {
                ui::segment(
                    Button::new(id).small().label(label),
                    self.node_workspace.view == choice,
                    cx,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.node_workspace.view = choice;
                    cx.notify();
                }))
            }),
        );
        let chips = table::status_chips(
            header.id("chips"),
            Status::ALL.map(|status| {
                table::status_chip(
                    status.id(),
                    status.tone(),
                    self.node_workspace.counts[status.index()],
                    status.label(),
                    self.node_workspace.filter == Some(status),
                    cx,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.node_workspace.filter = if view.node_workspace.filter == Some(status) {
                        None
                    } else {
                        Some(status)
                    };
                    view.node_workspace.rebuild_lines();
                    cx.notify();
                }))
            }),
            cx,
        );
        let header = header.chips(Some(
            h_flex().gap(dp(8.)).flex_wrap().child(segment).child(chips),
        ));
        let density_id = header.id("density");
        let header = if self.node_workspace.view == NodeView::Table {
            header
                .control(
                    Button::new(density_id)
                        .outline()
                        .small()
                        .icon(if self.node_workspace.table.compact {
                            IconName::Rows4
                        } else {
                            IconName::Rows2
                        })
                        .accessibility_label("Toggle node row density")
                        .tooltip(if self.node_workspace.table.compact {
                            "Compact · 26px rows. Switch to comfortable"
                        } else {
                            "Comfortable · 34px rows. Switch to compact"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.node_workspace.table.compact = !view.node_workspace.table.compact;
                            cx.notify();
                        })),
                )
                .control(self.nodes_columns_menu(cx))
        } else {
            header
        };
        header
            .control(
                Button::new("nodes-refresh")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .accessibility_label("Refresh nodes")
                    .tooltip("Refresh nodes")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            )
            .render(cx)
    }
}
