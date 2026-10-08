//! Nodes' shared header, with cached source and freshness metadata.
use super::projection::Status;
use super::*;
use crate::ui::dp;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::ui::Tone;
use freshkube_ui::{page, table};
use gpui_kit::base::Selectable;
use gpui_kit::{
    assets::IconName,
    component::{
        Icon, Sizable,
        button::{Button, ButtonGroup, ButtonVariants},
        h_flex,
        input::Input,
    },
    prelude::*,
};

impl Pilot {
    pub(super) fn nodes_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = page::PageHeader::new("nodes", "Nodes");
        let segment = ButtonGroup::new("nodes-view")
            .outline()
            .small()
            .child(
                Button::new("nodes-view-cards")
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label("Cards")
                    .selected(self.node_workspace.view == NodeView::Cards),
            )
            .child(
                Button::new("nodes-view-table")
                    .h(dp(crate::ui::CONTROL_HEIGHT))
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
            }));
        let filter = div()
            .key_context("NodeWorkspaceFilter")
            .on_action(
                cx.listener(|view, _: &BackNode, window, cx| view.leave_node_filter(window, cx)),
            )
            .child(
                Input::new(&self.node_workspace.query)
                    .id(header.id("filter"))
                    .small()
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .cleanable(true)
                    .aria_label("Filter nodes by name, address or role")
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
        let chips = table::status_chips(
            header.id("chips"),
            Status::ALL.map(|status| {
                table::status_chip(
                    status.id(),
                    status.tone(),
                    self.node_workspace.counts[status.index()],
                    status.what(),
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
                    view.node_workspace.table.reveal(0, ScrollStrategy::Top);
                    view.node_workspace
                        .scroll
                        .scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }))
            }),
            cx,
        );
        let header = header.filter(filter).chips(Some(
            h_flex().gap(dp(8.)).flex_wrap().child(segment).child(chips),
        ));
        let header = if self.node_workspace.view == NodeView::Table {
            let items = self.nodes_columns_items(cx);
            let hidden = &self.node_workspace.hidden_columns;
            header.foldable(
                self.nodes_columns_menu(items.clone()),
                page::columns_fold(
                    items,
                    hidden.len(),
                    hidden.iter().eq(super::DEFAULT_HIDDEN.iter()),
                ),
            )
        } else {
            header
        };
        let refresh = page::handler(cx, |view: &mut Self, window, cx| view.refresh(window, cx));
        let button = Button::new("nodes-refresh")
            .ghost()
            .small()
            .size(dp(crate::ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label("Refresh nodes")
            .tooltip("Refresh nodes")
            .on_click({
                let refresh = refresh.clone();
                move |_, window, cx| refresh(window, cx)
            });
        header
            .foldable(button, page::item("Refresh", refresh))
            .render(window, cx)
    }
}

impl Pilot {
    pub(super) fn rebuild_nodes_meta(&mut self) {
        let (context, lead) = match (&self.applied.context, self.fixture) {
            (_, true) => (None, Some(Part::new("Example data"))),
            (Some(context), false) => (Some(context.clone()), None),
            (None, false) => (None, Some(Part::new("Not connected"))),
        };
        let mut parts: Vec<Part> = lead.into_iter().collect();
        if self.kubernetes_only.is_none() {
            parts.push(source_status("Talos", &self.overview));
        }
        let nodes = self
            .registry
            .active()
            .kubernetes_summary
            .data()
            .map(|summary| &summary.nodes);
        parts.push(match nodes {
            Some(nodes) if !nodes.is_current() => Part::new(format!(
                "Kubernetes · last known · {}",
                nodes.error().unwrap_or("awaiting current nodes")
            ))
            .tone(Tone::Warn),
            _ => source_status("Kubernetes", &self.registry.active().kubernetes_summary),
        });
        self.node_workspace.status = Segment::new(context, parts);
    }
}

/// A source's state as the status bar words it: `Talos · current 14:02:11`.
fn source_status<T, I: Clone + Eq>(name: &str, snapshot: &crate::state::Snapshot<T, I>) -> Part {
    let state = if snapshot.is_loading() {
        "loading"
    } else if snapshot.is_stale() {
        "last known"
    } else if snapshot.data().is_some() {
        "current"
    } else {
        "unavailable"
    };
    let mut text = format!("{name} · {state}");
    if let Some(time) = snapshot.last_successful() {
        let time: chrono::DateTime<chrono::Local> = time.into();
        text.push_str(&format!(" {}", time.format("%H:%M:%S")));
    }
    if let Some(error) = snapshot.error() {
        text.push_str(&format!(" · {error}"));
    }
    let part = Part::new(text);
    if snapshot.is_stale() {
        part.tone(Tone::Warn)
    } else {
        part
    }
}
