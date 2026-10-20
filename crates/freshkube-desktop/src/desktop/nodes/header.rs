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

impl Nodes {
    pub(super) fn nodes_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = page::PageHeader::new("nodes", "Nodes");
        let segment = ButtonGroup::new("nodes-view")
            .outline()
            .small()
            .child(
                Button::new("nodes-view-cards")
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label("Cards")
                    .selected(self.view == NodeView::Cards),
            )
            .child(
                Button::new("nodes-view-table")
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label("Table")
                    .selected(self.view == NodeView::Table),
            )
            .on_click(cx.listener(|view, choice: &Vec<usize>, _, cx| {
                view.view = if choice.first() == Some(&0) {
                    NodeView::Cards
                } else {
                    NodeView::Table
                };
                cx.notify();
            }));
        let filter = div()
            .key_context("NodeWorkspaceFilter")
            .on_action(cx.listener(|view, _: &BackNode, window, cx| view.leave_filter(window, cx)))
            .child(
                Input::new(&self.query)
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
                    self.counts[status.index()],
                    status.what(),
                    self.filter == Some(status),
                    cx,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.filter = if view.filter == Some(status) {
                        None
                    } else {
                        Some(status)
                    };
                    view.rebuild_lines();
                    view.table.reveal(0, ScrollStrategy::Top);
                    view.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }))
            }),
            cx,
        );
        let header = header.filter(filter).chips(Some(
            h_flex().gap(dp(8.)).flex_wrap().child(segment).child(chips),
        ));
        let header = if self.view == NodeView::Table {
            let items = self.nodes_columns_items(cx);
            let hidden = &self.hidden_columns;
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
        let button = Button::new("nodes-refresh")
            .ghost()
            .small()
            .size(dp(crate::ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label("Refresh nodes")
            .tooltip_with_action("Refresh nodes", &page::Refresh, Some(page::SHELL_CONTEXT))
            .on_click(page::dispatch(page::Refresh, &self.focus));
        header
            .foldable(
                button,
                page::action_item("Refresh", page::Refresh, &self.focus),
            )
            .render(window, cx)
    }
}

impl Pilot {
    pub(super) fn rebuild_nodes_meta(&mut self, cx: &mut App) {
        let (context, lead) = match (&self.applied.context, self.mode().is_example()) {
            (_, true) => (None, Some(Part::new("Example data"))),
            (Some(context), false) => (Some(context.clone()), None),
            (None, false) => (None, Some(Part::new("Not connected"))),
        };
        let mut parts: Vec<Part> = lead.into_iter().collect();
        if self.mode().has_talos() {
            parts.push(source_status("Talos", &self.overview));
        }
        let nodes = self
            .switch
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
            _ => source_status(
                "Kubernetes",
                &self.switch.registry.active().kubernetes_summary,
            ),
        });
        let status = Segment::new(context, parts);
        self.pages
            .node_workspace
            .update(cx, |nodes, _| nodes.status = status);
    }
}

/// A source's state as the status bar words it: `Talos · current 14:02:11`.
fn source_status<T, I: Clone + Eq, E: std::fmt::Display>(
    name: &str,
    snapshot: &crate::state::Snapshot<T, I, E>,
) -> Part {
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
